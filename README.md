# Ashley Mugwambi: portfolio

Three parts, each doing the job it suits:

| Part | Language | Job |
|---|---|---|
| `web/` | React + TypeScript (Vite) | The site itself: profile, release pipeline, experience, case studies, contact form, and the Ctrl+K search palette |
| `api/` | Go (standard library only) | Serves the profile (`GET /api/profile`, with ETag), takes contact messages (`POST /api/contact`), serves the built site with security headers |
| `engine/` | Rust (no dependencies) | The fuzzy search behind Ctrl+K, compiled to WebAssembly and run in the browser |

Everything the site says lives in one file: `api/content/profile.json`. Edit it and rebuild the Go binary.

## Build

Needs Go 1.22+, Rust (with the `wasm32-unknown-unknown` target) and Node 20+.

```bash
rustup target add wasm32-unknown-unknown   # once
./build.sh                                 # tests and builds all three
cd api && ./portfolio -static ../web/dist  # http://localhost:8080
```

## Develop

```bash
cd api && go run .          # API on :8080
cd web && npm run dev       # site on :5173, /api forwarded to :8080
```

Without `web/public/engine.wasm` the palette falls back to a plain search, so the site works before Rust is built.

## Contact messages

Stored, one JSON object per line, in `api/data/messages.jsonl` (owner-only permissions), and emailed to you when
`api/.env` has a Gmail app password (`SMTP_USER`, `SMTP_PASS`, `MAIL_TO`; see `api/.env.example`). Restart the
server after editing `.env`. Without a password, messages go to the file only.
Limits: 3 messages an hour per address; a hidden field catches bots; length and email checks on the server.

## Tests

- Rust: `cd engine && cargo test` (ranking, Unicode, the wasm memory interface)
- Go: `cd api && go test ./...` (profile, ETag, validation, honeypot, rate limit, static fallback, path traversal, headers)
- React: `cd web && npm test` (rendering, tabs, palette, contact form, corpus anchors)

## Deploy

One Go binary plus the `web/dist` folder. Behind a reverse proxy with HTTPS, set the proxy to pass the
real client address if you want the rate limit per visitor (`clientIP` in `api/main.go` reads `RemoteAddr`).
