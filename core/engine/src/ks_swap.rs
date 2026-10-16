//! Kitesurf: swap rounds, which hand cold code to the embedder's code store (ks_code_store)
//! so it leaves memory that, in wasm, never shrinks.

/// Kitesurf: swaps what outlived its eval since the last call and flushes the store; call it
/// at quiescent points. Returns how many values went to the store.
pub fn ks_swap_at_idle() -> usize {
    let Some(store) = crate::ks_code_store::current() else {
        return 0;
    };
    let swapped = crate::spanned_source_text::swap_source_texts();
    if swapped > 0 {
        store.flush();
    }
    swapped
}
