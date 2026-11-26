#!/usr/bin/env bash
# Test and build everything: Rust crates and server, the wasm search engine, the React site.
set -euo pipefail
cd "$(dirname "$0")"

echo "== Rust: lint, test"
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

echo "== Rust: search engine for the browser"
cargo build --release -p engine --target wasm32-unknown-unknown
cp target/wasm32-unknown-unknown/release/engine.wasm web/public/engine.wasm

echo "== React: test, build, precompress"
(cd web && npm ci --no-audit --no-fund && npm test && npm run build)

echo "== Rust: server"
cargo build --release -p portfolio-server

echo "== Audit"
cargo audit || echo "cargo audit not installed: cargo install cargo-audit"
(cd web && npm audit --omit=dev)

echo "Done. cd server && ../target/release/portfolio seed && ../target/release/portfolio"
