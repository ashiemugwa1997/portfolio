//! cargo run -p fuzzrank-bench --release -- [corpus.txt]
//!
//! Top-10 search over a list of file paths with fuzzrank, fuzzy-matcher (skim v2) and
//! nucleo-matcher. Each library is used the way its docs suggest; preprocessing that a real
//! app does once (fuzzrank's index, nucleo's UTF-32 strings) is timed separately.
use std::hint::black_box;
use std::time::{Duration, Instant};

use fuzzy_matcher::skim::SkimMatcherV2;
use fuzzy_matcher::FuzzyMatcher;
use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32String};

const QUERIES: [&str; 8] = ["lib", "srcmod", "tcpstream", "jsonvalue", "rtime", "hyperbody", "zzqx", "d"];
const RUNS: u32 = 20;

fn corpus() -> Vec<String> {
    if let Some(path) = std::env::args().nth(1) {
        return std::fs::read_to_string(path).expect("corpus file").lines().map(str::to_owned).collect();
    }
    let root = std::env::var("CARGO_HOME").map_or_else(
        |_| std::path::PathBuf::from(std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).unwrap()).join(".cargo"),
        std::path::PathBuf::from,
    );
    let mut out = Vec::new();
    let mut stack = vec![root.join("registry").join("src")];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "rs") {
                let s = p.to_string_lossy().replace('\\', "/");
                // drop the registry prefix: keep "crate-1.2.3/src/..."
                out.push(s.split("/registry/src/").nth(1).and_then(|r| r.split_once('/').map(|(_, rest)| rest.to_owned())).unwrap_or(s));
            }
        }
    }
    out.sort();
    out
}

fn time(f: impl Fn() -> usize) -> (Duration, usize) {
    let hits = f();
    let t = Instant::now();
    for _ in 0..RUNS {
        black_box(f());
    }
    (t.elapsed() / RUNS, hits)
}

fn top10<T>(mut scored: Vec<(T, i64)>) -> usize {
    let n = scored.len();
    if n > 10 {
        scored.select_nth_unstable_by(9, |a, b| b.1.cmp(&a.1));
        scored.truncate(10);
    }
    scored.sort_unstable_by_key(|s| std::cmp::Reverse(s.1));
    black_box(&scored);
    n
}

fn main() {
    let lines = corpus();
    println!("corpus: {} file paths\n", lines.len());

    let t = Instant::now();
    let index = fuzzrank::Index::new(&lines);
    let fz_build = t.elapsed();
    let t = Instant::now();
    let utf32: Vec<Utf32String> = lines.iter().map(|l| Utf32String::from(l.as_str())).collect();
    let nu_build = t.elapsed();
    println!("one-time preprocessing: fuzzrank index {fz_build:.2?}, nucleo utf-32 {nu_build:.2?}\n");

    let fz_matcher = fuzzrank::Matcher::default();
    let skim = SkimMatcherV2::default();
    let mut nucleo = Matcher::new(Config::DEFAULT.match_paths());

    println!("{:<11} {:>15} {:>17} {:>15} {:>15}   matches (fz / skim / nucleo)", "query", "fuzzrank Index", "fuzzrank Matcher", "fuzzy-matcher", "nucleo");
    let mut totals = [Duration::ZERO; 4];
    for q in QUERIES {
        let n_index = index.search(q, usize::MAX).len();
        let (t_index_only, _) = time(|| black_box(index.search(q, 10)).len());
        let (t_matcher, _) = time(|| top10(lines.iter().filter_map(|l| fz_matcher.fuzzy_match(l, q).map(|s| (l, s))).collect()));
        let (t_skim, n_skim) = time(|| top10(lines.iter().filter_map(|l| skim.fuzzy_match(l, q).map(|s| (l, s))).collect()));
        let pattern = Pattern::parse(q, CaseMatching::Ignore, Normalization::Smart);
        let (t_nucleo, n_nucleo) = {
            let mut run = || {
                top10(utf32.iter().filter_map(|u| pattern.score(u.slice(..), &mut nucleo).map(|s| (u, i64::from(s)))).collect())
            };
            let hits = run();
            let t = Instant::now();
            for _ in 0..RUNS {
                black_box(run());
            }
            (t.elapsed() / RUNS, hits)
        };
        for (total, t) in totals.iter_mut().zip([t_index_only, t_matcher, t_skim, t_nucleo]) {
            *total += t;
        }
        println!("{q:<11} {t_index_only:>15.2?} {t_matcher:>17.2?} {t_skim:>15.2?} {t_nucleo:>15.2?}   {n_index} / {n_skim} / {n_nucleo}");
    }
    let [index_total, matcher_total, skim_total, nucleo_total] = totals;
    println!("{:<11} {index_total:>15.2?} {matcher_total:>17.2?} {skim_total:>15.2?} {nucleo_total:>15.2?}", "total");
}
