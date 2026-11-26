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
//!
//! For highlighting, [`Index::positions`] gives the matched characters of a result, and
//! [`Matcher`] scores one string at a time with the same method names as `fuzzy-matcher`:
//!
//! ```
//! use fuzzrank::Matcher;
//!
//! let m = Matcher::default();
//! let (score, at) = m.fuzzy_indices("Deployment pipeline", "dply").unwrap();
//! assert!(score > 0);
//! assert_eq!(at, vec![0, 2, 3, 5]); // D-e-P-L-o-Y
//! ```
#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::cmp::{Ordering, Reverse};
use std::collections::BinaryHeap;

// ── Scorer ──────────────────────────────────────────────────────────────────

/// Scores one lowercased query word against one lowercased line.
pub trait Scorer {
    /// `None` when the word does not match the line at all. Higher is better.
    fn score(&self, term: &[char], line: &[char]) -> Option<i32>;

    /// Which characters of `line` the word matched, ascending. The default finds the
    /// first place the letters occur in order; scorers with a different notion of a match
    /// should override it.
    fn positions(&self, term: &[char], line: &[char]) -> Option<Vec<usize>> {
        tight(term, line)
    }
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
        tight(term, hay).map(|at| letters_score(&at, hay))
    }

    fn positions(&self, term: &[char], hay: &[char]) -> Option<Vec<usize>> {
        match find(hay, term) {
            Some(pos) => Some((pos..pos + term.len()).collect()),
            None => tight(term, hay),
        }
    }
}

/// every matched letter earns a point; word starts and runs earn more, gaps cost a little
fn letters_score(at: &[usize], hay: &[char]) -> i32 {
    let mut total = 0;
    let mut last: Option<usize> = None;
    for &i in at {
        total += 1;
        if word_start(hay, i) {
            total += 8;
        }
        match last {
            Some(l) if l + 1 == i => total += 5,
            Some(l) => total -= ((i - l).min(10) / 3) as i32,
            None => {}
        }
        last = Some(i);
    }
    total
}

/// Plain case-insensitive substring matching, earlier positions first.
#[derive(Debug, Clone, Copy, Default)]
pub struct Substring;

impl Scorer for Substring {
    fn score(&self, term: &[char], hay: &[char]) -> Option<i32> {
        find(hay, term).map(|pos| 1000 - pos.min(1000) as i32)
    }

    fn positions(&self, term: &[char], hay: &[char]) -> Option<Vec<usize>> {
        find(hay, term).map(|pos| (pos..pos + term.len()).collect())
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
    /// lowercased index -> original character index, only when lowercasing changed the
    /// length (e.g. 'İ' becomes two characters)
    origin: Option<Box<[u32]>>,
}

impl Line {
    fn new(text: &str) -> Self {
        let mut chars = Vec::with_capacity(text.len());
        let mut origin = Vec::new();
        let mut grew = false;
        for (i, c) in text.chars().enumerate() {
            let before = chars.len();
            chars.extend(c.to_lowercase());
            grew |= chars.len() - before != 1;
            origin.extend(std::iter::repeat(i as u32).take(chars.len() - before));
        }
        let mask = CharMask::of(&chars);
        Line { chars, mask, origin: grew.then(|| origin.into_boxed_slice()) }
    }

    fn original(&self, lowered: usize) -> usize {
        self.origin.as_ref().map_or(lowered, |o| o[lowered] as usize)
    }
}

fn terms_of(query: &str) -> Vec<Vec<char>> {
    query.split_whitespace().map(lower).collect()
}

/// every term must match; shorter lines win ties
fn line_score<S: Scorer>(scorer: &S, terms: &[Vec<char>], line: &[char]) -> Option<i64> {
    let mut total: i64 = 0;
    for t in terms {
        total += i64::from(scorer.score(t, line)?);
    }
    Some(total * 100 - (line.len().min(400) / 4) as i64)
}

fn line_positions<S: Scorer>(scorer: &S, terms: &[Vec<char>], line: &Line) -> Option<Vec<usize>> {
    let mut at = Vec::new();
    for t in terms {
        at.extend(scorer.positions(t, &line.chars)?.into_iter().map(|i| line.original(i)));
    }
    at.sort_unstable();
    at.dedup();
    Some(at)
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
        self.lines.extend(lines.into_iter().map(|l| Line::new(l.as_ref())));
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
        let terms = terms_of(query);
        if terms.is_empty() {
            return None;
        }
        let need = terms.iter().fold(CharMask::default(), |m, t| m.union(CharMask::of(t)));
        Some(self.lines.iter().enumerate().filter_map(move |(index, line)| {
            if !line.mask.contains(need) {
                return None;
            }
            line_score(&self.scorer, &terms, &line.chars).map(|score| Match { index, score })
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

    /// The characters of line `index` that `query` matched, as character positions in the
    /// original text, ascending. `None` when the line does not match. Call it only for the
    /// results you show: it is computed on demand.
    pub fn positions(&self, index: usize, query: &str) -> Option<Vec<usize>> {
        let terms = terms_of(query);
        let line = self.lines.get(index)?;
        if terms.is_empty() {
            return None;
        }
        line_positions(&self.scorer, &terms, line)
    }

    /// [`Index::search`] plus the matched positions of each result, for highlighting.
    pub fn search_with_positions(&self, query: &str, limit: usize) -> Vec<(Match, Vec<usize>)> {
        let terms = terms_of(query);
        self.search(query, limit)
            .into_iter()
            .filter_map(|m| Some((m, line_positions(&self.scorer, &terms, &self.lines[m.index])?)))
            .collect()
    }
}

// ── One string at a time ────────────────────────────────────────────────────

/// Scores one string at a time, for callers that keep their own list. The methods have the
/// same names and signatures as `fuzzy-matcher`'s `FuzzyMatcher`, so moving over is mostly a
/// change of type. One difference: a pattern with spaces is several words, each of which
/// must match, in any order.
#[derive(Debug, Clone, Copy)]
pub struct Matcher<S = Subsequence> {
    scorer: S,
}

impl Default for Matcher<Subsequence> {
    fn default() -> Self {
        Matcher { scorer: Subsequence }
    }
}

impl<S: Scorer> Matcher<S> {
    /// A matcher that ranks with `scorer`.
    pub fn with_scorer(scorer: S) -> Self {
        Matcher { scorer }
    }

    /// The score of `pattern` against `choice`, or `None` when it does not match. An empty or
    /// blank pattern matches everything with score 0, as in `fuzzy-matcher`.
    ///
    /// Built for being called once per item with the same pattern: the parsed pattern is
    /// kept per thread, most non-matches are rejected before any work on `choice`, and the
    /// lowercased text goes into a reused buffer.
    pub fn fuzzy_match(&self, choice: &str, pattern: &str) -> Option<i64> {
        PATTERN.with(|cell| {
            let mut c = cell.borrow_mut();
            c.load(pattern);
            if c.terms.is_empty() {
                return Some(0);
            }
            if !CharMask::of_str(choice).contains(c.need) {
                return None;
            }
            let Prepared { terms, buf, .. } = &mut *c;
            buf.clear();
            buf.extend(choice.chars().flat_map(char::to_lowercase));
            line_score(&self.scorer, terms, buf)
        })
    }

    /// The score and the matched character positions of `choice`, ascending. An empty or
    /// blank pattern gives `Some((0, vec![]))`.
    pub fn fuzzy_indices(&self, choice: &str, pattern: &str) -> Option<(i64, Vec<usize>)> {
        let terms = terms_of(pattern);
        if terms.is_empty() {
            return Some((0, Vec::new()));
        }
        let line = Line::new(choice);
        let score = line_score(&self.scorer, &terms, &line.chars)?;
        Some((score, line_positions(&self.scorer, &terms, &line)?))
    }
}

/// the last pattern seen on this thread, parsed, plus a scratch buffer
#[derive(Default)]
struct Prepared {
    pattern: String,
    terms: Vec<Vec<char>>,
    need: CharMask,
    buf: Vec<char>,
}

impl Prepared {
    fn load(&mut self, pattern: &str) {
        if self.pattern != pattern || (self.terms.is_empty() && !pattern.trim().is_empty()) {
            self.pattern.clear();
            self.pattern.push_str(pattern);
            self.terms = terms_of(pattern);
            self.need = self.terms.iter().fold(CharMask::default(), |m, t| m.union(CharMask::of(t)));
        }
    }
}

thread_local! {
    static PATTERN: std::cell::RefCell<Prepared> = std::cell::RefCell::new(Prepared::default());
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

    /// the mask of `s` lowercased, without building the lowercased text
    fn of_str(s: &str) -> Self {
        let mut m = 0;
        for b in s.bytes() {
            if b.is_ascii() {
                m |= Self::bit(char::from(b.to_ascii_lowercase()));
            } else {
                // rare: fall back to full Unicode lowercasing for the rest
                return CharMask(m | s.chars().flat_map(char::to_lowercase).fold(0, |m, c| m | Self::bit(c)));
            }
        }
        CharMask(m)
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

/// The letters of `term` in order, in the tightest stretch of `hay` that ends where a full
/// match first can: a forward pass finds that end, a backward pass from it takes the nearest
/// letters. "rlbk" in "Rebuilt ... rollback" lands on "rollback", not on the first r and l.
fn tight(term: &[char], hay: &[char]) -> Option<Vec<usize>> {
    if term.is_empty() {
        return Some(Vec::new());
    }
    let mut ti = 0;
    let end = hay.iter().position(|&h| {
        if h == term[ti] {
            ti += 1;
        }
        ti == term.len()
    })?;
    let mut at = vec![0; term.len()];
    let mut ti = term.len();
    for i in (0..=end).rev() {
        if hay[i] == term[ti - 1] {
            ti -= 1;
            at[ti] = i;
            if ti == 0 {
                break;
            }
        }
    }
    Some(at)
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
    fn positions_for_highlighting() {
        let i = idx();
        assert_eq!(i.positions(2, "saml"), Some(vec![0, 1, 2, 3]));
        assert_eq!(i.positions(0, "dply"), Some(vec![0, 2, 3, 5]));
        assert_eq!(i.positions(0, "saml"), None);
        assert_eq!(i.positions(99, "x"), None);
        let both = i.positions(0, "dep roll").unwrap();
        assert!(both.windows(2).all(|w| w[0] < w[1]), "ascending, no duplicates");
    }

    #[test]
    fn positions_follow_the_original_text_when_lowercasing_grows_it() {
        // 'İ' lowercases to two characters; positions must still point at the original
        let i = Index::new(["İstanbul Bay"]);
        let at = i.positions(0, "bay").unwrap();
        let chars: Vec<char> = "İstanbul Bay".chars().collect();
        assert_eq!(at.iter().map(|&p| chars[p]).collect::<String>(), "Bay");
    }

    #[test]
    fn search_with_positions_pairs_each_result() {
        let r = idx().search_with_positions("react", 3);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].0.index, 4);
        assert_eq!(r[0].1.len(), 5);
    }

    #[test]
    fn matcher_agrees_with_index() {
        let m = Matcher::default();
        let i = idx();
        for q in ["dep roll", "saml", "dev", "195", "lmas"] {
            for (n, line) in LINES.iter().enumerate() {
                let from_index = i.matches(q).into_iter().find(|x| x.index == n).map(|x| x.score);
                assert_eq!(m.fuzzy_match(line, q), from_index, "{q:?} vs {line:?}");
            }
        }
        assert_eq!(m.fuzzy_match("anything", "   "), Some(0), "blank pattern matches all, like fuzzy-matcher");
        assert_eq!(m.fuzzy_indices("anything", ""), Some((0, vec![])));
    }

    #[test]
    fn string_mask_equals_char_mask() {
        for s in ["", "Deployment Pipeline", "İstanbul", "Café système → déploiement", "A-Z 0-9 _./"] {
            assert_eq!(CharMask::of_str(s), CharMask::of(&lower(s)), "{s:?}");
        }
    }

    #[test]
    fn matcher_pattern_cache_follows_the_pattern() {
        let m = Matcher::default();
        assert!(m.fuzzy_match("deployment", "dep").is_some());
        assert!(m.fuzzy_match("deployment", "xyz").is_none());
        assert!(m.fuzzy_match("deployment", "dep").is_some());
        assert_eq!(m.fuzzy_match("deployment", ""), Some(0));
        assert!(m.fuzzy_match("deployment", "xyz").is_none(), "cache refreshed after the empty pattern");
    }

    #[test]
    fn highlights_land_on_the_tight_match() {
        let line = "Rebuilt the deployment pipeline before an automatic rollback";
        let at = Index::new([line]).positions(0, "rlbk").unwrap();
        let chars: Vec<char> = line.chars().collect();
        let picked: String = at.iter().map(|&p| chars[p]).collect();
        assert_eq!(picked, "rlbk");
        assert!(at[0] > 40, "inside 'rollback', not the first r of 'Rebuilt': {at:?}");
    }

    #[test]
    fn tight_window_is_never_wider_than_first_in_order() {
        for (term, hay) in [("dply", "deployment"), ("ab", "a b ab"), ("aaa", "a a aaa"), ("xyz", "x y z")] {
            let t: Vec<char> = term.chars().collect();
            let h: Vec<char> = hay.chars().collect();
            let a = tight(&t, &h).unwrap();
            assert_eq!(a.len(), t.len());
            assert!(a.windows(2).all(|w| w[0] < w[1]));
            assert!(a.iter().zip(&t).all(|(&i, c)| h[i] == *c));
        }
        assert_eq!(tight(&['q'], &['a', 'b']), None);
    }

    #[test]
    fn empty_index() {
        let i = Index::default();
        assert!(i.is_empty());
        assert_eq!(i.rank("x"), [] as [usize; 0]);
    }
}
