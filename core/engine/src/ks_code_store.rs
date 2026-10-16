//! Kitesurf: a place outside the heap where an embedder keeps bytes that compiled code
//! rarely reads (a script's source text for `toString`), fetched back on demand.

use std::{cell::RefCell, rc::Rc};

/// Kitesurf: an embedder's store for cold code bytes. Every method may fail; a refused
/// `put` keeps the bytes resident and a failed `read` falls back the way a reader chooses.
pub trait CodeStore {
    /// Kitesurf: values smaller than this stay resident, since a store write and its
    /// later delete cost more than the memory they would free.
    fn min_bytes(&self) -> usize;

    /// Kitesurf: stores `bytes` and returns the key to read them back with, or None to
    /// keep them resident (the store is full, failing, or does not want them).
    fn put(&self, bytes: &[u8]) -> Option<u64>;

    /// Kitesurf: replaces `out` with `len` bytes of the value `key` from `offset` on.
    /// Returns false when they cannot be read; `out` is then unspecified.
    fn read(&self, key: u64, offset: usize, len: usize, out: &mut Vec<u8>) -> bool;

    /// Kitesurf: the value's owner is gone, so the store may drop it. Called at most
    /// once per key, and possibly while the thread tears its state down.
    fn release(&self, key: u64);
}

thread_local! {
    static STORE: RefCell<Option<Rc<dyn CodeStore>>> = const { RefCell::new(None) };
}

/// Kitesurf: installs (or, with None, removes) this thread's code store. Values already
/// stored keep the store they went to; source texts made from now on may be swapped.
pub fn ks_set_code_store(store: Option<Rc<dyn CodeStore>>) {
    let _ = STORE.try_with(|s| *s.borrow_mut() = store);
}

pub(crate) fn current() -> Option<Rc<dyn CodeStore>> {
    STORE.try_with(|s| s.borrow().clone()).ok().flatten()
}
