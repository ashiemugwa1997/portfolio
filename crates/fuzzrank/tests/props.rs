//! Random lines and queries against a plain reference model.
use fuzzrank::{Index, Matcher};
use proptest::prelude::*;

fn lower(s: &str) -> Vec<char> {
    s.chars().flat_map(char::to_lowercase).collect()
}

/// the definition of a match, written the slow, obvious way
fn reference_match(line: &str, query: &str) -> bool {
    let hay = lower(line);
    let terms: Vec<Vec<char>> = query.split_whitespace().map(lower).collect();
    !terms.is_empty()
        && terms.iter().all(|t| {
            let mut it = hay.iter();
            t.iter().all(|c| it.any(|h| h == c))
        })
}

fn text() -> impl Strategy<Value = String> {
    // mostly letters so matches happen, plus spaces, digits, punctuation and any Unicode
    prop_oneof![
        8 => "[a-eA-E0-9 ]{0,24}",
        1 => "[a-z ,.:/-]{0,30}",
        1 => any::<String>(),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(4096))]

    #[test]
    fn finds_exactly_the_lines_the_definition_says(lines in prop::collection::vec(text(), 0..40), query in text()) {
        let index = Index::new(&lines);
        let mut got = index.rank(&query);
        got.sort_unstable();
        let want: Vec<usize> = (0..lines.len()).filter(|&i| reference_match(&lines[i], &query)).collect();
        prop_assert_eq!(got, want);
    }

    #[test]
    fn search_is_the_head_of_matches(lines in prop::collection::vec(text(), 0..40), query in text(), k in 0usize..12) {
        let index = Index::new(&lines);
        let mut all = index.matches(&query);
        all.truncate(k);
        prop_assert_eq!(index.search(&query, k), all);
    }

    #[test]
    fn results_are_ordered_best_first(lines in prop::collection::vec(text(), 0..40), query in text()) {
        let m = Index::new(&lines).matches(&query);
        prop_assert!(m.windows(2).all(|w| w[0].score > w[1].score || (w[0].score == w[1].score && w[0].index < w[1].index)));
    }

    /// a line has positions exactly when it matches, and they point at real, ascending characters
    #[test]
    fn positions_agree_with_matching(line in text(), query in text()) {
        let index = Index::new([&line]);
        let matched = !index.rank(&query).is_empty();
        let at = index.positions(0, &query);
        prop_assert_eq!(at.is_some(), matched);
        if let Some(at) = at {
            let n = line.chars().count();
            prop_assert!(at.iter().all(|&p| p < n));
            prop_assert!(at.windows(2).all(|w| w[0] < w[1]));
            let (score, same) = Matcher::default().fuzzy_indices(&line, &query).unwrap();
            prop_assert_eq!(Some(score), Matcher::default().fuzzy_match(&line, &query));
            prop_assert_eq!(same, at);
        }
    }
}
