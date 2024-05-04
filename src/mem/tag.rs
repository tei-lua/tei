//! Utility functions for tagging and untagging pointers.

use core::cell::Cell;

trait ValidMask<const MASK: usize> {
    const CHECK: ();
}

impl<T, const MASK: usize> ValidMask<MASK> for T {
    const CHECK: () = assert!(MASK < core::mem::align_of::<T>());
}

/// Checks that `$mask` can be used to tag a pointer to `$type`.
/// If this isn't true, this macro will cause a post-monomorphization error.
macro_rules! check_mask {
    ($type:ty, $mask:expr) => {
        let _ = <$type as ValidMask<$mask>>::CHECK;
    };
}

pub(super) fn untag<T>(tagged_ptr: *const T) -> *const T {
    let mask = core::mem::align_of::<T>() - 1;
    tagged_ptr.map_addr(|addr| addr & !mask)
}

pub(super) fn get<const MASK: usize, T>(tagged_ptr: *const T) -> usize {
    check_mask!(T, MASK);
    tagged_ptr.addr() & MASK
}

pub(super) fn set<const MASK: usize, T>(pcell: &Cell<*const T>, tag: usize) {
    check_mask!(T, MASK);
    let ptr = pcell.get();
    let ptr = ptr.map_addr(|addr| (addr & !MASK) | (tag & MASK));
    pcell.set(ptr)
}

pub(super) fn set_bool<const MASK: usize, T>(pcell: &Cell<*const T>, value: bool) {
    check_mask!(T, MASK);
    let ptr = pcell.get();
    let ptr = ptr.map_addr(|addr| (addr & !MASK) | if value { MASK } else { 0 });
    pcell.set(ptr)
}
