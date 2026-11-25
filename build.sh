#!/usr/bin/env bash
# Build and test everything: Rust engine -> wasm, React site, Go API.
# Then run:  cd api && ./portfolio  (or go run . ) and open http://localhost:8080
set -euo pipefail
cd "$(dirname "$0")"

echo "== Rust: test, then build the search engine for the browser"
(cd engine && cargo test --quiet && cargo build --release --target wasm32-unknown-unknown)
cp engine/target/wasm32-unknown-unknown/release/engine.wasm web/public/engine.wasm
ls -l web/public/engine.wasm

echo "== React: test and build"
(cd web && npm ci --no-audit --no-fund && npm test && npm run build)

echo "== Go: test and build"
(cd api && go vet ./... && go test ./... && go build -o portfolio$( [ "${OS:-}" = Windows_NT ] && echo .exe ) .)

echo "Done. Start it with: cd api && ./portfolio -static ../web/dist"
