use core::{
    cell::{Cell, RefCell},
    mem,
    ops::Deref,
    ptr::NonNull,
};

use super::{
    managed::Managed,
    types::{Allocation, AllocationHeader, AllocationInner, GcColor, Invariant},
};

#[repr(transparent)]
pub struct Mutation<'gc> {
    state: State,
    _invariant: Invariant<'gc>,
}

#[repr(transparent)]
pub struct Visitor {
    state: State,
}

#[repr(transparent)]
pub struct Finalization<'gc> {
    state: State,
    _invariant: Invariant<'gc>,
}

// TODO: add metrics for invoking GC
// TODO: add tracing (see phaseguard)
// TODO: use a seperate mark list for marked objects that need to be traversed
// TODO: eliminate mark/sweep recursion
pub(super) struct State {
    head: Cell<Option<Allocation>>,
}

impl State {
    pub(super) unsafe fn new() -> Self {
        Self {
            head: Cell::new(None),
        }
    }

    pub(crate) unsafe fn mutation_context<'gc>(&self) -> &Mutation<'gc> {
        mem::transmute::<&Self, &Mutation>(&self)
    }

    fn visitor_context(&self) -> &Visitor {
        // SAFETY: `Visitor` is `repr(transparent)`
        unsafe { mem::transmute::<&Self, &Visitor>(self) }
    }

    pub(crate) unsafe fn finalization_context<'gc>(&self) -> &Finalization<'gc> {
        mem::transmute::<&Self, &Finalization>(&self)
    }

    fn allocate<T: Managed>(&self, t: T) -> NonNull<AllocationInner<T>> {
        let header = AllocationHeader::new::<T>();
        header.set_next(self.head.get());
        header.set_live(true);
        header.set_needs_trace(T::needs_trace());

        // TODO: better in-place construction optimization
        let (alloc, ptr) = unsafe {
            let mut uninitialized = Box::new(mem::MaybeUninit::<AllocationInner<T>>::uninit());
            core::ptr::write(uninitialized.as_mut_ptr(), AllocationInner::new(header, t));
            let ptr =
                NonNull::new_unchecked(Box::into_raw(uninitialized) as *mut AllocationInner<T>);
            (Allocation::erase(ptr), ptr)
        };

        self.head.set(Some(alloc));
        ptr
    }
}

impl Drop for State {
    fn drop(&mut self) {
        struct DropAll(Option<Allocation>);

        impl Drop for DropAll {
            fn drop(&mut self) {
                if let Some(gc_box) = self.0.take() {
                    let mut drop_resume = DropAll(Some(gc_box));
                    while let Some(gc_box) = drop_resume.0.take() {
                        drop_resume.0 = gc_box.header().next();
                        // SAFETY: the state owns its GC'd objects
                        unsafe { free_alloc(gc_box) }
                    }
                }
            }
        }

        DropAll(self.head.get());
    }
}

// SAFETY: the allocation must never be accessed after calling this function.
unsafe fn free_alloc(mut alloc: Allocation) {
    if alloc.header().is_live() {
        // If the alive flag is set, that means we haven't dropped the inner value of this object.
        alloc.drop_in_place();
    }

    alloc.dealloc();
}
