use super::managed::Managed;
use super::ptr::{AllocationInner, Invariant};
use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;
use std::mem;
use std::ops::Deref;
use std::ptr;
use std::ptr::NonNull;

/// A garbage collected pointer to a type T. Implements Copy, and is implemented as a plain machine
/// pointer. You can only allocate `Gc` pointers through a `&HeapInterface<'gc>` inside a heap type,
/// and through "generativity" such `Gc` pointers may not escape the arena they were born in or
/// be stored inside TLS. This, combined with correct `Managed` implementations, means that `Gc`
/// pointers will never be dangling and are always safe to access.
pub struct Gc<'gc, T: ?Sized + 'gc> {
    pub(super) ptr: NonNull<AllocationInner<T>>,
    pub(super) _invariant: Invariant<'gc>,
}

impl<'gc, T: Managed + 'gc> Gc<'gc, T> {
    // TODO: impl new here
}

// TODO: impl managed

impl<'gc, T: 'gc> Gc<'gc, T> {
    /// Cast the internal pointer to a different type.
    ///
    /// **SAFETY:**:
    /// It must be valid to dereference a `*mut U` that has come from casting a `*mut T`.
    pub unsafe fn cast<U: 'gc>(this: Gc<'gc, T>) -> Gc<'gc, U> {
        Gc {
            ptr: NonNull::cast(this.ptr),
            _invariant: PhantomData,
        }
    }

    /// Retrieve a `Gc` from a raw pointer obtained from `Gc::as_ptr`
    ///
    /// **SAFETY:**:
    /// The provided pointer must have been obtained from `Gc::as_ptr`, and the pointer must not
    /// have been collected yet.
    pub unsafe fn from_ptr(ptr: *const T) -> Self {
        let header_offset = mem::offset_of!(AllocationInner<T>, value) as isize;
        let ptr = (ptr as *mut T)
            .byte_offset(-header_offset)
            .cast::<AllocationInner<T>>();

        Self {
            ptr: NonNull::new_unchecked(ptr),
            _invariant: PhantomData,
        }
    }
}

// TODO: impl unlock

impl<'gc, T: ?Sized + 'gc> Gc<'gc, T> {
    /// Obtains a long-lived reference to the contents of this `Gc`.
    ///
    /// Unlike `AsRef` or `Deref`, the returned reference isn't bound to the `Gc` itself, and
    /// will stay valid for the entirety of the current heap callback.
    pub fn as_ref(self: Gc<'gc, T>) -> &'gc T {
        // SAFETY: The returned reference cannot escape the current heap callback, as `&'gc T`
        // never implements `Managed` (unless `'gc` is `'static`, which is impossible here), and
        // so cannot be stored inside the GC root.
        unsafe { &self.ptr.as_ref().value }
    }

    pub fn as_ptr(gc: Gc<'gc, T>) -> *const T {
        unsafe {
            let inner = gc.ptr.as_ptr();
            core::ptr::addr_of!((*inner).value) as *const T
        }
    }

    // TODO: impl downgrade
    // TODO: impl write

    /// Returns true if two `Gc`s point to the same allocation.
    ///
    /// Similarly to `Rc::ptr_eq` and `Arc::ptr_eq`, this function ignores the metadata of `dyn`
    /// pointers.
    pub fn ptr_eq(this: Gc<'gc, T>, other: Gc<'gc, T>) -> bool {
        ptr::addr_eq(Gc::as_ptr(this), Gc::as_ptr(other))
    }

    // TODO: impl is_dead
    // TODO: impl rescurrect
}

impl<'gc, T: ?Sized + 'gc> Clone for Gc<'gc, T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<'gc, T: ?Sized + 'gc> Copy for Gc<'gc, T> {}

impl<'gc, T: ?Sized + 'gc> Deref for Gc<'gc, T> {
    type Target = T;

    fn deref(&self) -> &T {
        unsafe { &self.ptr.as_ref().value }
    }
}

impl<'gc, T: ?Sized + 'gc> AsRef<T> for Gc<'gc, T> {
    fn as_ref(&self) -> &T {
        unsafe { &self.ptr.as_ref().value }
    }
}

impl<'gc, T: ?Sized + 'gc> fmt::Pointer for Gc<'gc, T> {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        fmt::Pointer::fmt(&Gc::as_ptr(*self), fmt)
    }
}

impl<'gc, T: fmt::Debug + ?Sized + 'gc> fmt::Debug for Gc<'gc, T> {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        fmt::Debug::fmt(&**self, fmt)
    }
}

impl<'gc, T: fmt::Display + ?Sized + 'gc> fmt::Display for Gc<'gc, T> {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        fmt::Display::fmt(&**self, fmt)
    }
}

impl<'gc, T: PartialEq + ?Sized + 'gc> PartialEq for Gc<'gc, T> {
    fn eq(&self, other: &Self) -> bool {
        (**self).eq(other)
    }

    #[allow(clippy::partialeq_ne_impl)]
    fn ne(&self, other: &Self) -> bool {
        (**self).ne(other)
    }
}

impl<'gc, T: Eq + ?Sized + 'gc> Eq for Gc<'gc, T> {}

impl<'gc, T: PartialOrd + ?Sized + 'gc> PartialOrd for Gc<'gc, T> {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        (**self).partial_cmp(other)
    }

    fn le(&self, other: &Self) -> bool {
        (**self).le(other)
    }

    fn lt(&self, other: &Self) -> bool {
        (**self).lt(other)
    }

    fn ge(&self, other: &Self) -> bool {
        (**self).ge(other)
    }

    fn gt(&self, other: &Self) -> bool {
        (**self).gt(other)
    }
}

impl<'gc, T: Ord + ?Sized + 'gc> Ord for Gc<'gc, T> {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        (**self).cmp(other)
    }
}

impl<'gc, T: Hash + ?Sized + 'gc> Hash for Gc<'gc, T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (**self).hash(state)
    }
}
