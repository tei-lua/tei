use super::context::Visitor;
use super::managed::Managed;
use super::tag;
use core::ptr::NonNull;
use std::alloc;
use std::alloc::Layout;
use std::cell::Cell;
use std::marker::PhantomData;
use std::mem::ManuallyDrop;
use std::ptr;

/// A thin-pointer-sized box containing a type-erased GC object.
/// Stores the metadata required by the GC algorithm inline (see `AllocationInner`
/// for its typed counterpart).
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub(super) struct Allocation(NonNull<AllocationInner<()>>);

impl Allocation {
    /// Erases a pointer to a typed GC object.
    ///
    /// **SAFETY:** The pointer must point to a valid `AllocationInner`.`
    pub(super) unsafe fn erase<T: ?Sized>(ptr: NonNull<AllocationInner<T>>) -> Self {
        // This cast is sound because `AllocationInner` is `repr(C)`.
        let erased = ptr.as_ptr() as *mut AllocationInner<()>;
        Self(NonNull::new_unchecked(erased))
    }

    /// Gets a pointer to the value stored inside this box.
    /// `T` must be the same type the erased pointer was created from.
    fn unerased_value<T>(&self) -> *mut T {
        unsafe {
            let ptr = self.0.as_ptr() as *mut AllocationInner<T>;

            // Don't create a reference, to keep the full provenance.
            // Also, this gives us interior mutability "for free".
            ptr::addr_of_mut!((*ptr).value) as *mut T
        }
    }

    pub(super) fn header(&self) -> &AllocationHeader {
        unsafe { &self.0.as_ref().header }
    }

    /// Traces the stored value.
    ///
    /// **SAFETY**: `Self::drop_in_place` must not have been called.
    pub(super) unsafe fn trace_value(&self, visitor: &Visitor) {
        (self.header().vtable().trace_value)(*self, visitor)
    }

    /// Drops the stored value.
    ///
    /// **SAFETY**: once called, no GC pointers should access the stored value
    /// (but accessing the `Allocation` itself is still safe).
    pub(super) unsafe fn drop_in_place(&mut self) {
        (self.header().vtable().drop_value)(*self)
    }

    /// Deallocates the box. Failing to call `Self::drop_in_place` beforehand
    /// will cause the stored value to be leaked.
    ///
    /// **SAFETY**: once called, this `Allocation` should never be accessed by any GC pointers.
    pub(super) unsafe fn dealloc(self) {
        let layout = self.header().vtable().alloc_layout;
        let ptr = self.0.as_ptr() as *mut u8;
        // SAFETY: the pointer was allocated with this layout.
        alloc::dealloc(ptr, layout);
    }
}

pub(super) struct AllocationHeader {
    /// The next element in the global linked list of allocated objects.
    next: Cell<Option<Allocation>>,

    /// A custom virtual function table for handling type-specific operations.
    ///
    /// The lower bits of the pointer are used to store GC flags:
    /// - bits 0 & 1 for the current `GcColor`;
    /// - bit 2 for the `needs_trace` flag;
    /// - bit 3 for the `is_live` flag.
    tagged_vtable: Cell<*const ManagedVTable>,
}

impl AllocationHeader {
    pub(super) fn new<T>() -> Self
    where
        T: Managed,
    {
        let vtable: &'static _ = &const { ManagedVTable::for_type::<T>() };

        Self {
            next: Cell::new(None),
            tagged_vtable: Cell::new(vtable as *const _),
        }
    }

    /// Gets a reference to the `ManagedVTable` used by this allocation.
    fn vtable(&self) -> &'static ManagedVTable {
        let ptr = tag::untag(self.tagged_vtable.get());

        // SAFETY:
        // - the pointer was properly untagged.
        // - the vtable is stored in static memory.
        unsafe { &*ptr }
    }

    /// Gets the next element in the global linked list of allocated objects.
    pub(super) fn next(&self) -> Option<Allocation> {
        self.next.get()
    }

    /// Sets the next element in the global linked list of allocated objects.
    pub(super) fn set_next(&self, next: Option<Allocation>) {
        self.next.set(next)
    }

    pub(super) fn color(&self) -> GcColor {
        match tag::get::<0x3, _>(self.tagged_vtable.get()) {
            0x0 => GcColor::White,
            0x1 => GcColor::WhiteWeak,
            0x2 => GcColor::Black,
            _ => unreachable!(),
        }
    }

    pub(super) fn set_color(&self, color: GcColor) {
        tag::set::<0x3, _>(
            &self.tagged_vtable,
            match color {
                GcColor::White => 0x0,
                GcColor::WhiteWeak => 0x1,
                GcColor::Black => 0x2,
            },
        );
    }

    pub(super) fn needs_trace(&self) -> bool {
        tag::get::<0x4, _>(self.tagged_vtable.get()) != 0x0
    }

    pub(super) fn set_needs_trace(&self, needs_trace: bool) {
        tag::set_bool::<0x4, _>(&self.tagged_vtable, needs_trace);
    }

    /// Determines whether or not we've dropped the `dyn Managed` value
    /// stored in `Allocation.value`
    /// When we garbage-collect a `Allocation` that still has outstanding weak pointers,
    /// we set `alive` to false. When there are no more weak pointers remaining,
    /// we will deallocate the `Allocation`, but skip dropping the `dyn Managed` value
    /// (since we've already done it).
    pub(super) fn is_live(&self) -> bool {
        tag::get::<0x8, _>(self.tagged_vtable.get()) != 0x0
    }

    pub(super) fn set_live(&self, alive: bool) {
        tag::set_bool::<0x8, _>(&self.tagged_vtable, alive);
    }
}

/// Type-specific operations for GC managed allocations.
///
/// We use a custom vtable instead of `dyn Managed` for extra flexibility.
/// The type is over-aligned so that `AllcationHeader` can store flags into the LSBs of the vtable pointer.
#[repr(align(16))]
struct ManagedVTable {
    /// The layout of the `AllocationInner` the value is stored in.
    alloc_layout: Layout,

    /// Drops the value stored in the given `Allocation` (without deallocating).
    drop_value: unsafe fn(Allocation),

    /// Traces the value stored in the given `Allocation`.
    trace_value: unsafe fn(Allocation, &Visitor),
}

impl ManagedVTable {
    /// Makes a vtable for a known type.
    const fn for_type<T>() -> Self
    where
        T: Managed,
    {
        Self {
            alloc_layout: Layout::new::<AllocationInner<T>>(),
            drop_value: |erased_ptr| unsafe {
                let ptr = erased_ptr.unerased_value();
                ptr::drop_in_place::<T>(ptr);
            },
            trace_value: |erased_ptr, visitor| unsafe {
                let ptr = erased_ptr.unerased_value();
                T::trace(&*ptr, visitor);
            },
        }
    }
}

/// A typed allocated and managed value, together with its metadata.
/// This type is never manipulated directly by the GC algorithm, allowing
/// user-facing `Gc`s to freely cast their pointer to it.
#[repr(C)]
pub(super) struct AllocationInner<T>
where
    T: ?Sized,
{
    pub(super) header: AllocationHeader,

    /// The typed value stored in this `Allocation`.
    pub(super) value: ManuallyDrop<T>,
}

// Phantom type that holds a lifetime and ensures that it is invariant.
pub(super) type Invariant<'a> = PhantomData<Cell<&'a ()>>;

#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub(super) enum GcColor {
    White,
    WhiteWeak,
    Black,
}
