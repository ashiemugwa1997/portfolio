# fuzzrank-wasm

Fast fuzzy search for command palettes and filters: the Rust crate [fuzzrank](https://crates.io/crates/fuzzrank) compiled to 47 KB of WebAssembly, with types and no dependencies.

```js
import { createIndex, highlight } from 'fuzzrank-wasm'

const items = ['Deployment pipeline with rollback', 'SAML single sign-on', 'Budget approvals']
const index = await createIndex(items)

for (const i of index.search('dep roll', 10)) {
  el.innerHTML += `<li>${highlight(items[i], index.positions(i, 'dep roll'))}</li>`
}
```

- Every word of the query must match, in any order; letters may be scattered (`dply` finds "Deployment").
- Substrings, word starts and runs of letters rank higher; shorter items win ties.
- Case-insensitive and Unicode-aware.
- `highlight()` wraps matches in `<mark>` and HTML-escapes the rest.

Works in browsers, Node 18+, Deno and Bun. Bundlers pick up `fuzzrank.wasm` through `new URL(..., import.meta.url)`; to host it yourself, pass `createIndex(items, { wasm: '/path/fuzzrank.wasm' })` (a URL, the bytes, or a compiled `WebAssembly.Module`).

With a strict Content-Security-Policy, allow `'wasm-unsafe-eval'` in `script-src`.

MIT licence.
