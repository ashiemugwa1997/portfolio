# fuzzrank

Small fuzzy search for command palettes and quick filters. No dependencies, no unsafe code, compiles to WebAssembly as is.

```rust
use fuzzrank::Index;

let index = Index::new(["Deployment pipeline with rollback", "SAML single sign-on", "Budget approvals"]);

assert_eq!(index.rank("dep roll"), vec![0]);   // every word must match
assert_eq!(index.rank("dply"), vec![0]);       // letters in order
for m in index.search("budget", 5) {
    println!("{} scored {}", m.index, m.score);
}
```

## How it ranks

- Every word of the query must match the line, in any order.
- A word matches when its letters appear in order (`dply` finds "Deployment").
- An exact substring beats scattered letters.
- Letters that start a word or follow the previous match score higher.
- Shorter lines win ties; equal scores keep the input order.
- Case-insensitive and Unicode-aware.

It runs in the browser behind the Ctrl+K search of [my portfolio](https://github.com/ashiemugwa1997/portfolio), through a thin wasm wrapper (`engine/`).

## Licence

MIT or Apache-2.0, at your option.
