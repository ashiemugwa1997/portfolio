# Security policy

This policy covers the crates published from this repository (fuzzrank, windowgate, tower-cross-origin, tower-bulwark, contact-guard), the `fuzzrank-wasm` npm package and the portfolio server.

## Reporting a vulnerability

Please do not open a public issue. Use GitHub's private reporting instead: **Security → Report a vulnerability** on this repository.

You will get an answer within 3 working days. Confirmed issues are fixed in a patch release, announced in the crate's CHANGELOG, and filed with the RustSec advisory database where relevant. Reporters are credited unless they prefer otherwise.

## Supported versions

The latest minor version of each crate gets security fixes.

## What is checked on every change

- `cargo clippy` with warnings as errors, on Linux, Windows and macOS
- Unit, integration and property tests (tens of thousands of random and hostile inputs per run)
- `cargo audit` against the RustSec database and `npm audit`
- The minimum supported Rust version (1.75) for the dependency-free crates
