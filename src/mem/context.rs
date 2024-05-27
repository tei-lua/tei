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
// TODO: finalizers? probably needs to modify can_upgrade and add phase tracking.
pub(super) struct State {
    head: Cell<Option<Allocation>>,
    grey: RefCell<Vec<Allocation>>,
    is_sweeping: Cell<bool>,
}

impl State {
    pub(super) unsafe fn new() -> Self {
        Self {
            head: Cell::new(None),
            grey: RefCell::new(Vec::new()),
            is_sweeping: Cell::new(false),
        }
    }

    pub(crate) unsafe fn mutation_context<'gc>(&self) -> &Mutation<'gc> {
        mem::transmute::<&Self, &Mutation>(&self)
    }

    fn visitor_context(&self) -> &Visitor {
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

    fn can_upgrade(&self, alloc: Allocation) -> bool {
        return alloc.header().is_live();
    }

    fn trace(&self, alloc: Allocation) {
        let header = alloc.header();

        if matches!(header.color(), GcColor::White | GcColor::WhiteWeak) {
            if header.needs_trace() {
                // A white traceable object is not the grey queue already.
                // Thus it becomes grey and enters it.
                header.set_color(GcColor::Grey);
                debug_assert!(header.is_live());
                self.grey.borrow_mut().push(alloc);
            } else {
                // A white object that doesn't need tracing becomes black.
                header.set_color(GcColor::Black);
            }
        }
    }

    fn trace_weak(&self, alloc: Allocation) {
        let header = alloc.header();

        if header.color() == GcColor::White {
            header.set_color(GcColor::WhiteWeak);
        }
    }

    fn rescurrect(&self, alloc: Allocation) {
        let header = alloc.header();
        debug_assert!(header.is_live());

        if matches!(header.color(), GcColor::White | GcColor::WhiteWeak) {
            header.set_color(GcColor::Grey);
            self.grey.borrow_mut().push(alloc);
        }
    }

    fn do_mark<R: Managed>(&self, root: &R) {
        let visitor = self.visitor_context();
        root.trace(visitor);

        // While the grey queue isn't empty, pop one, trace it and turn it black.
        // Once the queue is empty, we've traced all reachable objects.
        while let Some(grey) = self.grey.borrow_mut().pop() {
            // To prevent incomplete tracing if `Managed::trace` panics, use a drop guard to
            // push it back onto the grey queue. This only delays the problem
            // until the next collection but it should be sufficient for the
            // application to resolve the problem.
            struct DropGuard<'a> {
                state: &'a State,
                alloc: Allocation,
            }

            impl<'a> Drop for DropGuard<'a> {
                fn drop(&mut self) {
                    self.state.grey.borrow_mut().push(self.alloc);
                }
            }

            let guard = DropGuard {
                state: self,
                alloc: grey,
            };
            let header = grey.header();
            debug_assert!(header.is_live());
            unsafe {
                grey.trace_value(visitor);
            }
            header.set_color(GcColor::Black);
            mem::forget(guard);
        }
    }

    fn do_sweep(&self) {
        // We copy the allocation list in `self.head` here. Any allocations made during
        // the sweep phase will be added to `self.head` but not to to `sweep`.
        // This ensures we keep allocations alive until we've had a chance to trace them.
        let mut sweep = self.head.get();
        let mut sweep_prev: Option<Allocation> = None;

        while let Some(mut curr) = sweep {
            let curr_header = curr.header();
            let next = curr_header.next();
            sweep = next;

            match curr_header.color() {
                // If the next object in the sweep subsection of the allocation list is white,
                // we need to remove it from the main object list and remove it.
                GcColor::White => {
                    if let Some(prev) = sweep_prev {
                        prev.header().set_next(next);
                    } else {
                        // If `sweep_prev` is None, then the sweep pointer is also the
                        // beginning of the main object list, so we need to adjust it.
                        debug_assert_eq!(self.head.get(), sweep);
                        self.head.set(next);
                    }

                    // SAFETY: At this point, the object is white and wasn't traced by a weak pointer
                    // during this cycle, meaning it is not reachable, so we can free the allocation.
                    unsafe {
                        free_alloc(curr);
                    }
                }
                GcColor::WhiteWeak => {
                    // Keep the allocation and let it remain in the allocation list if we traced
                    // weak pointer to it. This is because the weak pointer needs to access the
                    // allocation header to check if the object is still alive. We can only deallocate
                    // the memory once there are no weak pointers left.

                    sweep_prev = Some(curr);
                    curr_header.set_color(GcColor::White);

                    // Only drop the object if it wasn't dropped previously.
                    if curr_header.is_live() {
                        curr_header.set_live(false);

                        // SAFETY: Since the object is white, there are no strong pointers to this object
                        // and only weak ones. Since those perform a check on access, we can drop the
                        // contents of the allocation.
                        unsafe {
                            curr.drop_in_place();
                        }
                    }
                }
                GcColor::Black => {
                    // There are strong pointers to this object, so we need to keep it alive.
                    sweep_prev = Some(curr);
                    curr_header.set_color(GcColor::White);
                }
                GcColor::Grey => debug_assert!(false, "unexpected gray object in sweep list"),
            }
        }
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
                        // SAFETY: The state owns its managed objects.
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
