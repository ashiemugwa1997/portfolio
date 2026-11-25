//! Fuzzy search for the portfolio's command palette.
//!
//! Built for `wasm32-unknown-unknown` with no dependencies and no wasm-bindgen.
//! The page talks to it through a few plain exports:
//!
//! 1. `alloc(len)` reserves bytes in wasm memory; the page writes UTF-8 there.
//! 2. `load(ptr, len)` takes the corpus: one entry per line.
//! 3. `search(ptr, len, out_ptr, max)` scores every entry against the query and
//!    writes up to `max` entry indexes (u32, best first) to `out_ptr`, returning how many.
//! 4. `dealloc(ptr, len)` gives memory back.

use std::cell::RefCell;

/// one searchable line, lowercased once on load
struct Entry {
    chars: Vec<char>,
}

thread_local! {
    static CORPUS: RefCell<Vec<Entry>> = const { RefCell::new(Vec::new()) };
}

// ── Corpus ──────────────────────────────────────────────────────────────────

pub fn load_text(text: &str) {
    let entries = text
        .lines()
        .map(|l| Entry { chars: l.chars().flat_map(char::to_lowercase).collect() })
        .collect();
    CORPUS.with(|c| *c.borrow_mut() = entries);
}

// ── Scoring ─────────────────────────────────────────────────────────────────

/// Score one query term against one entry, or `None` when its letters do not all appear in order.
///
/// Every matched letter earns a point; letters that start a word, follow the previous
/// match directly, or sit at the start of the line earn more, so "dep pip" ranks
/// "Deployment pipeline" above a line that merely contains those letters scattered.
fn score_term(term: &[char], hay: &[char]) -> Option<i32> {
    if term.is_empty() {
        return Some(0);
    }
    // exact substring beats scattered letters
    if let Some(pos) = find(hay, term) {
        let start_bonus = if pos == 0 || !hay[pos - 1].is_alphanumeric() { 30 } else { 10 };
        return Some(100 + start_bonus + term.len() as i32 * 4 - (pos as i32).min(40) / 4);
    }
    let mut score = 0;
    let mut ti = 0;
    let mut last: Option<usize> = None;
    for (hi, &h) in hay.iter().enumerate() {
        if ti == term.len() {
            break;
        }
        if h == term[ti] {
            score += 1;
            if hi == 0 || !hay[hi - 1].is_alphanumeric() {
                score += 8;
            }
            if let Some(l) = last {
                if l + 1 == hi {
                    score += 5;
                } else {
                    score -= ((hi - l) as i32).min(10) / 3;
                }
            }
            last = Some(hi);
            ti += 1;
        }
    }
    (ti == term.len()).then_some(score)
}

fn find(hay: &[char], needle: &[char]) -> Option<usize> {
    if needle.len() > hay.len() {
        return None;
    }
    (0..=hay.len() - needle.len()).find(|&i| hay[i..i + needle.len()] == *needle)
}

/// Rank the corpus against `query`. Every word of the query must match; the result holds
/// entry indexes, best first, ties in corpus order.
pub fn rank(query: &str) -> Vec<u32> {
    let terms: Vec<Vec<char>> = query
        .split_whitespace()
        .map(|t| t.chars().flat_map(char::to_lowercase).collect())
        .collect();
    if terms.is_empty() {
        return Vec::new();
    }
    CORPUS.with(|c| {
        let corpus = c.borrow();
        let mut hits: Vec<(i32, u32)> = corpus
            .iter()
            .enumerate()
            .filter_map(|(i, e)| {
                let mut total = 0;
                for t in &terms {
                    total += score_term(t, &e.chars)?;
                }
                // shorter lines win ties
                Some((total * 100 - (e.chars.len() as i32).min(400) / 4, i as u32))
            })
            .collect();
        hits.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        hits.into_iter().map(|(_, i)| i).collect()
    })
}

// ── Wasm exports ────────────────────────────────────────────────────────────

#[no_mangle]
pub extern "C" fn alloc(len: usize) -> *mut u8 {
    let mut buf = Vec::<u8>::with_capacity(len.max(1));
    let ptr = buf.as_mut_ptr();
    std::mem::forget(buf);
    ptr
}

/// # Safety
/// `ptr` and `len` must come from one earlier `alloc(len)` call.
#[no_mangle]
pub unsafe extern "C" fn dealloc(ptr: *mut u8, len: usize) {
    drop(Vec::from_raw_parts(ptr, 0, len.max(1)));
}

/// # Safety
/// `ptr..ptr+len` must be memory the page wrote through `alloc`.
#[no_mangle]
pub unsafe extern "C" fn load(ptr: *const u8, len: usize) -> u32 {
    let bytes = std::slice::from_raw_parts(ptr, len);
    load_text(&String::from_utf8_lossy(bytes));
    CORPUS.with(|c| c.borrow().len() as u32)
}

/// # Safety
/// `q_ptr..q_ptr+q_len` holds the query; `out` has room for `max` u32 values.
#[no_mangle]
pub unsafe extern "C" fn search(q_ptr: *const u8, q_len: usize, out: *mut u32, max: usize) -> u32 {
    let query = String::from_utf8_lossy(std::slice::from_raw_parts(q_ptr, q_len));
    let ranked = rank(&query);
    let n = ranked.len().min(max);
    std::ptr::copy_nonoverlapping(ranked.as_ptr(), out, n);
    n as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    const CORPUS: &str = "Deployment pipeline with automatic rollback\n\
        Budgeting module: multi-level approval\n\
        SAML single sign-on\n\
        Penetration test remediation: 195 pages closed\n\
        Drone Training Platform in React Native\n\
        Developer experience improvements";

    fn ranked(q: &str) -> Vec<u32> {
        load_text(CORPUS);
        rank(q)
    }

    #[test]
    fn empty_query_finds_nothing() {
        assert!(ranked("   ").is_empty());
    }

    #[test]
    fn substring_match_is_found() {
        assert_eq!(ranked("saml")[0], 2);
    }

    #[test]
    fn case_does_not_matter() {
        assert_eq!(ranked("REACT NATIVE"), vec![4]);
    }

    #[test]
    fn scattered_letters_match_in_order() {
        // d-p-l-y appear in order only in "Deployment".
        assert_eq!(ranked("dply")[0], 0);
    }

    #[test]
    fn every_word_must_match() {
        assert_eq!(ranked("dep roll"), vec![0]);
        assert!(ranked("saml budgeting").is_empty());
    }

    #[test]
    fn word_starts_rank_above_mid_word() {
        // "dev" starts "Developer" but only appears scattered in "Deployment ... ".
        let r = ranked("dev");
        assert_eq!(r[0], 5);
    }

    #[test]
    fn numbers_are_searchable() {
        assert_eq!(ranked("195"), vec![3]);
    }

    #[test]
    fn letters_out_of_order_do_not_match() {
        assert!(ranked("lmas").is_empty());
    }

    #[test]
    fn non_ascii_text_is_safe() {
        load_text("Café système → déploiement");
        assert_eq!(rank("déploi"), vec![0]);
        assert_eq!(rank("CAFÉ"), vec![0]);
    }

    #[test]
    fn wasm_exports_round_trip() {
        unsafe {
            let corpus = CORPUS.as_bytes();
            let p = alloc(corpus.len());
            std::ptr::copy_nonoverlapping(corpus.as_ptr(), p, corpus.len());
            assert_eq!(load(p, corpus.len()), 6);
            dealloc(p, corpus.len());

            let q = b"budget";
            let qp = alloc(q.len());
            std::ptr::copy_nonoverlapping(q.as_ptr(), qp, q.len());
            let out = alloc(4 * 8) as *mut u32;
            let n = search(qp, q.len(), out, 8);
            assert_eq!(n, 1);
            assert_eq!(*out, 1);
            dealloc(qp, q.len());
            dealloc(out as *mut u8, 32);
        }
    }
}
