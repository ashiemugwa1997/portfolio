//! fuzzrank for the browser, with no wasm-bindgen: a few plain exports.
//!
//! 1. `alloc(len)` reserves bytes; the page writes UTF-8 there.
//! 2. `load(ptr, len)` takes the corpus, one entry per line.
//! 3. `search(ptr, len, out, max)` writes up to `max` line indexes (u32, best first) to `out`.
//! 4. `dealloc(ptr, len)` gives memory back.

use std::alloc::Layout;
use std::cell::RefCell;

use fuzzrank::Index;

thread_local! {
    static INDEX: RefCell<Index> = RefCell::new(Index::default());
}

// ── Wasm exports ────────────────────────────────────────────────────────────

// 8-aligned, so the page can read results through a Uint32Array (needs 4)
fn layout(len: usize) -> Layout {
    Layout::from_size_align(len.max(1), 8).expect("allocation size fits")
}

#[no_mangle]
pub extern "C" fn alloc(len: usize) -> *mut u8 {
    let ptr = unsafe { std::alloc::alloc(layout(len)) };
    if ptr.is_null() {
        std::alloc::handle_alloc_error(layout(len));
    }
    ptr
}

/// # Safety
/// `ptr` and `len` must come from one earlier `alloc(len)` call.
#[no_mangle]
pub unsafe extern "C" fn dealloc(ptr: *mut u8, len: usize) {
    std::alloc::dealloc(ptr, layout(len));
}

/// # Safety
/// `ptr..ptr+len` must be memory the page wrote through `alloc`.
#[no_mangle]
pub unsafe extern "C" fn load(ptr: *const u8, len: usize) -> u32 {
    let text = String::from_utf8_lossy(std::slice::from_raw_parts(ptr, len));
    INDEX.with(|i| {
        *i.borrow_mut() = Index::from_text(&text);
        i.borrow().len() as u32
    })
}

/// # Safety
/// `q_ptr..q_ptr+q_len` holds the query; `out` has room for `max` u32 values.
#[no_mangle]
pub unsafe extern "C" fn search(q_ptr: *const u8, q_len: usize, out: *mut u32, max: usize) -> u32 {
    let query = String::from_utf8_lossy(std::slice::from_raw_parts(q_ptr, q_len));
    let hits = INDEX.with(|i| i.borrow().search(&query, max));
    for (n, m) in hits.iter().enumerate() {
        *out.add(n) = m.index as u32;
    }
    hits.len() as u32
}

/// Matched character positions of line `line` for the query, for highlighting.
///
/// # Safety
/// `q_ptr..q_ptr+q_len` holds the query; `out` has room for `max` u32 values.
#[no_mangle]
pub unsafe extern "C" fn positions(line: u32, q_ptr: *const u8, q_len: usize, out: *mut u32, max: usize) -> u32 {
    let query = String::from_utf8_lossy(std::slice::from_raw_parts(q_ptr, q_len));
    let at = INDEX.with(|i| i.borrow().positions(line as usize, &query)).unwrap_or_default();
    let n = at.len().min(max);
    for (k, p) in at.iter().take(n).enumerate() {
        *out.add(k) = *p as u32;
    }
    n as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(clippy::cast_ptr_alignment)] // alloc hands out 8-aligned memory
    fn exports_round_trip() {
        unsafe {
            let corpus = b"Deployment pipeline\nBudgeting module\nSAML single sign-on";
            let p = alloc(corpus.len());
            std::ptr::copy_nonoverlapping(corpus.as_ptr(), p, corpus.len());
            assert_eq!(load(p, corpus.len()), 3);
            dealloc(p, corpus.len());

            let q = b"budget";
            let qp = alloc(q.len());
            std::ptr::copy_nonoverlapping(q.as_ptr(), qp, q.len());
            let out = alloc(4 * 8).cast::<u32>();
            assert_eq!(search(qp, q.len(), out, 8), 1);
            assert_eq!(*out, 1);
            dealloc(qp, q.len());
            dealloc(out.cast::<u8>(), 32);
        }
    }
}
