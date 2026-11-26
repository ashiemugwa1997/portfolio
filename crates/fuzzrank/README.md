# fuzzrank

[![crates.io](https://img.shields.io/crates/v/fuzzrank.svg)](https://crates.io/crates/fuzzrank)
[![docs.rs](https://docs.rs/fuzzrank/badge.svg)](https://docs.rs/fuzzrank)
[![npm](https://img.shields.io/npm/v/fuzzrank-wasm.svg)](https://www.npmjs.com/package/fuzzrank-wasm)

Fast fuzzy search for command palettes, pickers and filters. No dependencies, no unsafe code, compiles to WebAssembly as is (also on npm as [`fuzzrank-wasm`](../../npm/fuzzrank-wasm)).

```rust
use fuzzrank::Index;

let index = Index::new(["Deployment pipeline with rollback", "SAML single sign-on", "Budget approvals"]);

assert_eq!(index.rank("dep roll"), vec![0]);   // every word must match, any order
assert_eq!(index.rank("dply"), vec![0]);       // letters in order

for (m, at) in index.search_with_positions("budget", 10) {
    println!("line {} scored {}, highlight chars {:?}", m.index, m.score, at);
}
```

## How it ranks

- Every word of the query must match the line, in any order.
- A word matches when its letters appear in order (`dply` finds "Deployment").
- An exact substring beats scattered letters; letters that start a word or follow the previous match score higher.
- Shorter lines win ties; equal scores keep the input order.
- Case-insensitive and Unicode-aware; highlight positions point at the original characters even where lowercasing changes length (`İ`).
- Scattered letters are aligned to the tightest stretch of text, so highlights land where a person would expect ("rlbk" marks "rollback").
- Ranking is pluggable: implement `Scorer` (two built in: `Subsequence`, `Substring`).

## Speed

Top-10 search over **about 16,000 real file paths** (every `.rs` file in a Cargo registry), 8 queries, release build, one core. All three libraries returned exactly the same matching lines for every query. Over several runs fuzzrank's `Index` was **11–14× faster than fuzzy-matcher** and **1.3–1.6× faster than nucleo-matcher**; one quiet run:

| | total, 8 queries | vs fuzzy-matcher |
|---|---:|---:|
| **fuzzrank `Index`** | **3.0 ms** | **14× faster** |
| nucleo-matcher 0.3.1 | 4.8 ms | 9× faster |
| **fuzzrank `Matcher`** (one string at a time) | **32 ms** | **1.3× faster** |
| fuzzy-matcher 0.3.7 (skim v2) | 43 ms | 1× |

Why it is fast: each line keeps a 64-bit fingerprint of its characters, so most lines are rejected with one AND before any scoring; the top results come from a bounded heap instead of a full sort; `Matcher` caches the parsed pattern per thread and reuses its buffer. Reproduce with `cargo run -p fuzzrank-bench --release` from the repository.

## Moving from fuzzy-matcher

`fuzzy-matcher` has not been updated since 2020. `fuzzrank::Matcher` has the same method names and signatures:

```rust
// before
use fuzzy_matcher::{skim::SkimMatcherV2, FuzzyMatcher};
let matcher = SkimMatcherV2::default();

// after
let matcher = fuzzrank::Matcher::default();

let score: Option<i64> = matcher.fuzzy_match("Deployment pipeline", "dply");
let hit: Option<(i64, Vec<usize>)> = matcher.fuzzy_indices("Deployment pipeline", "dply");
```

Differences to know:

- A pattern with spaces is several words that must all match, in any order (fuzzy-matcher treats a space as a character). Most palettes want this.
- Like fuzzy-matcher, an empty pattern matches everything with score 0.
- Scores are on a different scale; only compare scores from the same library.
- If you search the same list repeatedly, build an `Index` once instead: it is about 10× faster than calling `Matcher` per item.

## Licence

MIT or Apache-2.0, at your option.
