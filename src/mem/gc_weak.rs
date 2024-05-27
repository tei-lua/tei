use super::gc::Gc;
use super::managed::Managed;
use super::types::{AllocationInner, Invariant};
use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;
use std::mem;
use std::ops::Deref;
use std::ptr;
use std::ptr::NonNull;

pub struct GcWeak<'gc, T: ?Sized + 'gc>(pub(super) Gc<'gc, T>);

// TODO: impl managed

impl<'gc, T: 'gc> GcWeak<'gc, T> {
    /// Cast the internal pointer to a different type.
    ///
    /// **SAFETY:**:
    /// It must be valid to dereference a `*mut U` that has come from casting a `*mut T`.
    pub unsafe fn cast<U: 'gc>(this: Gc<'gc, T>) -> GcWeak<'gc, U> {
        GcWeak(Gc::cast(this))
    }

    /// Retrieve a `Gc` from a raw pointer obtained from `GcWeak::as_ptr`
    ///
    /// **SAFETY:**:
    /// The provided pointer must have been obtained from `GcWeak::as_ptr` or `Gc::as_ptr`, and the pointer must not
    /// have been *fully* collected yet (it may be dropped but valid weak pointer).
    pub unsafe fn from_ptr(ptr: *const T) -> Self {
        Self(Gc::from_ptr(ptr))
    }
}

impl<'gc, T: ?Sized + 'gc> GcWeak<'gc, T> {
    pub fn as_ptr(gc: GcWeak<'gc, T>) -> *const T {
        Gc::as_ptr(gc.0)
    }

    // TODO: impl upgrade
    // TODO: impl is_dropped

    /// Returns true if two `Gc`s point to the same allocation.
    ///
    /// Similarly to `Rc::ptr_eq` and `Arc::ptr_eq`, this function ignores the metadata of `dyn`
    /// pointers.
    pub fn ptr_eq(this: GcWeak<'gc, T>, other: GcWeak<'gc, T>) -> bool {
        ptr::addr_eq(GcWeak::as_ptr(this), GcWeak::as_ptr(other))
    }

    // TODO: impl is_dead
    // TODO: impl rescurrect
}

impl<'gc, T: ?Sized + 'gc> Clone for GcWeak<'gc, T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<'gc, T: ?Sized + 'gc> Copy for GcWeak<'gc, T> {}

impl<'gc, T: ?Sized + 'gc> fmt::Pointer for GcWeak<'gc, T> {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        fmt::Pointer::fmt(&GcWeak::as_ptr(*self), fmt)
    }
}

impl<'gc, T: fmt::Debug + ?Sized + 'gc> fmt::Debug for GcWeak<'gc, T> {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        write!(fmt, "(GcWeak)")
    }
}
