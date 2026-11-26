//! Small fuzzy search for command palettes and quick filters.
//!
//! * Every word of the query must match a line, in any order.
//! * A word matches when its letters appear in the line in order (`dply` finds "Deployment").
//! * Exact substrings beat scattered letters; letters at the start of a word or right after
//!   the previous match score higher; shorter lines win ties.
//! * Case-insensitive and Unicode-aware (`CAFÉ` finds "café").
//! * No dependencies, no unsafe code, no global state: it compiles to WebAssembly as is.
//!
//! ```
//! use fuzzrank::Index;
//!
//! let index = Index::new(["Deployment pipeline with rollback", "SAML single sign-on", "Budget approvals"]);
//! assert_eq!(index.rank("dep roll"), vec![0]);
//! assert_eq!(index.rank("sso"), vec![1]);
//!
//! let top = index.search("budget", 5);
//! assert_eq!(top[0].index, 2);
//! ```
//!
//! Ranking is pluggable: implement [`Scorer`] and build with [`Index::with_scorer`].
#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::cmp::{Ordering, Reverse};
use std::collections::BinaryHeap;

// ── Scorer ──────────────────────────────────────────────────────────────────

/// Scores one lowercased query word against one lowercased line.
pub trait Scorer {
    /// `None` when the word does not match the line at all. Higher is better.
    fn score(&self, term: &[char], line: &[char]) -> Option<i32>;
}

/// The default: substring first, then letters in order with word-start and run bonuses.
#[derive(Debug, Clone, Copy, Default)]
pub struct Subsequence;

impl Scorer for Subsequence {
    fn score(&self, term: &[char], hay: &[char]) -> Option<i32> {
        if term.is_empty() {
            return Some(0);
        }
        if let Some(pos) = find(hay, term) {
            let start_bonus = if word_start(hay, pos) { 30 } else { 10 };
            return Some(100 + start_bonus + term.len() as i32 * 4 - (pos.min(40) / 4) as i32);
        }
        let mut total = 0;
        let mut ti = 0;
        let mut last: Option<usize> = None;
        for (hi, &h) in hay.iter().enumerate() {
            if ti == term.len() {
                break;
            }
            if h != term[ti] {
                continue;
            }
            total += 1;
            if word_start(hay, hi) {
                total += 8;
            }
            match last {
                Some(l) if l + 1 == hi => total += 5,
                Some(l) => total -= ((hi - l).min(10) / 3) as i32,
                None => {}
            }
            last = Some(hi);
            ti += 1;
        }
        (ti == term.len()).then_some(total)
    }
}

/// Plain case-insensitive substring matching, earlier positions first.
#[derive(Debug, Clone, Copy, Default)]
pub struct Substring;

impl Scorer for Substring {
    fn score(&self, term: &[char], hay: &[char]) -> Option<i32> {
        find(hay, term).map(|pos| 1000 - pos.min(1000) as i32)
    }
}

// ── Index ───────────────────────────────────────────────────────────────────

/// One result: which line matched and how well.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Match {
    /// Position of the line in the order it was given.
    pub index: usize,
    /// Higher is better. Only meaningful against other scores for the same query.
    pub score: i64,
}

// best first: higher score, then lower index
impl Ord for Match {
    fn cmp(&self, other: &Self) -> Ordering {
        self.score.cmp(&other.score).then_with(|| other.index.cmp(&self.index))
    }
}
impl PartialOrd for Match {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Debug, Clone)]
struct Line {
    chars: Vec<char>,
    mask: CharMask,
}

/// Lines to search, lowercased once, each with a character fingerprint for quick rejects.
#[derive(Debug, Clone)]
pub struct Index<S = Subsequence> {
    lines: Vec<Line>,
    scorer: S,
}

impl Default for Index<Subsequence> {
    fn default() -> Self {
        Index { lines: Vec::new(), scorer: Subsequence }
    }
}

impl Index<Subsequence> {
    /// Build an index over `lines` with the default scorer.
    pub fn new<I, T>(lines: I) -> Self
    where
        I: IntoIterator<Item = T>,
        T: AsRef<str>,
    {
        Index::with_scorer(lines, Subsequence)
    }

    /// One entry per line of `text`.
    pub fn from_text(text: &str) -> Self {
        Index::new(text.lines())
    }
}

impl<S: Scorer> Index<S> {
    /// Build an index that ranks with `scorer`.
    pub fn with_scorer<I, T>(lines: I, scorer: S) -> Self
    where
        I: IntoIterator<Item = T>,
        T: AsRef<str>,
    {
        let mut index = Index { lines: Vec::new(), scorer };
        index.extend(lines);
        index
    }

    /// Add more lines; their indexes continue from the current length.
    pub fn extend<I, T>(&mut self, lines: I)
    where
        I: IntoIterator<Item = T>,
        T: AsRef<str>,
    {
        self.lines.extend(lines.into_iter().map(|l| {
            let chars = lower(l.as_ref());
            let mask = CharMask::of(&chars);
            Line { chars, mask }
        }));
    }

    /// Number of lines.
    pub fn len(&self) -> usize {
        self.lines.len()
    }

    /// True when there is nothing to search.
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    fn candidates<'a>(&'a self, query: &str) -> Option<impl Iterator<Item = Match> + 'a> {
        let terms: Vec<Vec<char>> = query.split_whitespace().map(lower).collect();
        if terms.is_empty() {
            return None;
        }
        let need = terms.iter().fold(CharMask::default(), |m, t| m.union(CharMask::of(t)));
        Some(self.lines.iter().enumerate().filter_map(move |(index, line)| {
            if !line.mask.contains(need) {
                return None;
            }
            let mut total: i64 = 0;
            for t in &terms {
                total += i64::from(self.scorer.score(t, &line.chars)?);
            }
            let len_penalty = (line.chars.len().min(400) / 4) as i64;
            Some(Match { index, score: total * 100 - len_penalty })
        }))
    }

    /// Every matching line, best first. Ties keep the original order.
    pub fn matches(&self, query: &str) -> Vec<Match> {
        let Some(hits) = self.candidates(query) else { return Vec::new() };
        let mut all: Vec<Match> = hits.collect();
        all.sort_unstable_by(|a, b| b.cmp(a));
        all
    }

    /// The best `limit` matches, best first, in O(n log limit).
    pub fn search(&self, query: &str, limit: usize) -> Vec<Match> {
        if limit == 0 {
            return Vec::new();
        }
        let Some(hits) = self.candidates(query) else { return Vec::new() };
        // min-heap of the best seen so far: the weakest sits on top, ready to be replaced
        let mut heap: BinaryHeap<Reverse<Match>> = BinaryHeap::with_capacity(limit + 1);
        for m in hits {
            if heap.len() < limit {
                heap.push(Reverse(m));
            } else if heap.peek().is_some_and(|Reverse(weakest)| m > *weakest) {
                heap.pop();
                heap.push(Reverse(m));
            }
        }
        let mut best: Vec<Match> = heap.into_iter().map(|Reverse(m)| m).collect();
        best.sort_unstable_by(|a, b| b.cmp(a));
        best
    }

    /// Indexes of every matching line, best first.
    pub fn rank(&self, query: &str) -> Vec<usize> {
        self.matches(query).into_iter().map(|m| m.index).collect()
    }
}

// ── Character fingerprint ───────────────────────────────────────────────────

/// 64-bit set of the characters a line holds (a-z, 0-9 get their own bit, the rest share
/// hashed bits). A query word can only match a line whose set contains all of its bits.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct CharMask(u64);

impl CharMask {
    fn bit(c: char) -> u64 {
        let i = match c {
            'a'..='z' => c as u32 - 'a' as u32,
            '0'..='9' => 26 + (c as u32 - '0' as u32),
            c if c.is_whitespace() => return 0,
            c => 36 + (c as u32 % 28),
        };
        1 << i
    }

    fn of(chars: &[char]) -> Self {
        CharMask(chars.iter().fold(0, |m, &c| m | Self::bit(c)))
    }

    fn union(self, other: Self) -> Self {
        CharMask(self.0 | other.0)
    }

    fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn lower(s: &str) -> Vec<char> {
    s.chars().flat_map(char::to_lowercase).collect()
}

fn word_start(hay: &[char], i: usize) -> bool {
    i == 0 || !hay[i - 1].is_alphanumeric()
}

fn find(hay: &[char], needle: &[char]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    if needle.len() > hay.len() {
        return None;
    }
    hay.windows(needle.len()).position(|w| w == needle)
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const LINES: [&str; 6] = [
        "Deployment pipeline with automatic rollback",
        "Budgeting module: multi-level approval",
        "SAML single sign-on",
        "Penetration test remediation: 195 pages closed",
        "Drone Training Platform in React Native",
        "Developer experience improvements",
    ];

    fn idx() -> Index {
        Index::new(LINES)
    }

    #[test]
    fn blank_query_finds_nothing() {
        assert_eq!(idx().rank("   "), [] as [usize; 0]);
        assert_eq!(idx().search("", 5), [] as [Match; 0]);
    }

    #[test]
    fn substring_wins() {
        assert_eq!(idx().rank("saml")[0], 2);
    }

    #[test]
    fn case_does_not_matter() {
        assert_eq!(idx().rank("REACT NATIVE"), vec![4]);
    }

    #[test]
    fn scattered_letters_in_order() {
        assert_eq!(idx().rank("dply")[0], 0);
    }

    #[test]
    fn every_word_must_match() {
        assert_eq!(idx().rank("dep roll"), vec![0]);
        assert_eq!(idx().rank("saml budgeting"), [] as [usize; 0]);
    }

    #[test]
    fn word_starts_rank_higher() {
        assert_eq!(idx().rank("dev")[0], 5);
    }

    #[test]
    fn numbers_match() {
        assert_eq!(idx().rank("195"), vec![3]);
    }

    #[test]
    fn letters_out_of_order_do_not_match() {
        assert_eq!(idx().rank("lmas"), [] as [usize; 0]);
    }

    #[test]
    fn unicode() {
        let i = Index::new(["Café système → déploiement"]);
        assert_eq!(i.rank("déploi"), vec![0]);
        assert_eq!(i.rank("CAFÉ"), vec![0]);
    }

    #[test]
    fn search_equals_the_head_of_matches() {
        let i = idx();
        for q in ["e", "de", "pl", "o n", "a"] {
            for k in 0..8 {
                let mut expect = i.matches(q);
                expect.truncate(k);
                assert_eq!(i.search(q, k), expect, "query {q:?}, limit {k}");
            }
        }
    }

    #[test]
    fn ties_keep_input_order() {
        let i = Index::new(["abc", "abc", "abc", "abc"]);
        assert_eq!(i.rank("abc"), vec![0, 1, 2, 3]);
        assert_eq!(i.search("abc", 2).iter().map(|m| m.index).collect::<Vec<_>>(), vec![0, 1]);
    }

    #[test]
    fn extend_continues_indexes() {
        let mut i = Index::from_text("one\ntwo");
        i.extend(["three"]);
        assert_eq!(i.len(), 3);
        assert_eq!(i.rank("three"), vec![2]);
    }

    #[test]
    fn other_scorers_plug_in() {
        let i = Index::with_scorer(["deployment", "the deploy"], Substring);
        assert_eq!(i.rank("deploy"), vec![0, 1]);
        assert!(i.rank("dply").is_empty(), "substring scorer does not do scattered letters");
    }

    #[test]
    fn mask_rejects_without_false_negatives() {
        let lines: Vec<Vec<char>> = LINES.iter().map(|l| lower(l)).collect();
        for q in ["dply", "zzz", "195", "sso", "é", "→"] {
            let need = CharMask::of(&lower(q));
            for line in &lines {
                if Subsequence.score(&lower(q), line).is_some() {
                    assert!(CharMask::of(line).contains(need), "mask dropped a real match for {q:?}");
                }
            }
        }
    }

    #[test]
    fn empty_index() {
        let i = Index::default();
        assert!(i.is_empty());
        assert_eq!(i.rank("x"), [] as [usize; 0]);
    }
}
