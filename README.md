# Ashley Mugwambi: portfolio

My portfolio site. The front end is React; everything behind it is Rust, split into small crates that are useful on their own and published on crates.io.

| Path | What |
|---|---|
| [`crates/fuzzrank`](crates/fuzzrank) | Fuzzy search: every word must match, highlight positions, `fuzzy-matcher`-compatible API. 14× faster than fuzzy-matcher and 1.6× faster than nucleo on 16k real paths. No dependencies. |
| [`crates/windowgate`](crates/windowgate) | Sliding-window rate limiter with bounded memory; optional MySQL backend shared across servers and a tower layer. |
| [`crates/tower-cross-origin`](crates/tower-cross-origin) | CSRF protection without tokens: Sec-Fetch-Site and Origin rules (as in Go's `CrossOriginProtection`), tower layer. |
| [`crates/tower-bulwark`](crates/tower-bulwark) | Tower/axum layer: strict security headers, CSRF, trusted-proxy client IP, rate limiting, pluggable rules. |
| [`crates/contact-guard`](crates/contact-guard) | Contact-form validation, honeypot, email composition that cannot be header-injected; optional `lettre` output. |
| [`npm/fuzzrank-wasm`](npm/fuzzrank-wasm) | fuzzrank for JavaScript: 47 KB of WebAssembly with types and a `highlight()` helper. |
| `benches/fuzzrank-bench` | fuzzrank against fuzzy-matcher and nucleo-matcher on real file paths. |
| `engine/` | fuzzrank compiled to WebAssembly for the Ctrl+K search (38 KB, no wasm-bindgen). |
| `server/` | The web server (axum + sqlx): profile from MySQL, contact form, the built site served from memory. |
| `web/` | React + TypeScript (Vite). |

## How the server is put together

- **Content lives in MySQL.** `server/content/profile.json` is only the seed: `portfolio seed` loads it into normalised tables in one transaction; the site reads from the tables.
- **Two database accounts.** The website's account can read content and insert contact messages, nothing else; a separate admin account loads content. `server/scripts/setup-mysql.sh` creates both with random passwords written straight to `server/.env`.
- **Stores and mailers are interfaces.** `Store` has a MySQL implementation, a caching decorator (one query per 30 s however busy the site is, and the last good copy if the database hiccups) and an in-memory one for tests. `Mailer` has an SMTP implementation; a bounded notifier sends in the background.
- **Static files from memory.** The built site is loaded at start-up into a map keyed by URL path, with precompressed Brotli/gzip copies, so requests never touch the filesystem and path traversal has nothing to reach.

## Security

| Threat | Defence |
|---|---|
| SQL injection | Prepared statements only; no multi-statements; the web account cannot write content. |
| XSS | Strict CSP (self only, `wasm-unsafe-eval` for the search engine), React escaping, only `https`/`http`/`mailto` links accepted (server and browser). |
| CSRF | Same-origin check on every write (`Origin` / `Sec-Fetch-Site`). |
| Clickjacking, sniffing | `frame-ancestors 'none'`, `X-Frame-Options: DENY`, `nosniff`, COOP/CORP. |
| Email header injection | Control characters refused in names; headers folded to one line; RFC 2047 subjects. |
| Spam and floods | Honeypot; 3 messages an hour per sender and 50 a day for the site, counted in MySQL (survive restarts, shared by every server; senders kept only as salted hashes); 120 requests a minute per address; bounded limiter memory. |
| Slow or idle clients | 5 s header deadline, 15 s request timeout, 16 KB body limit, 512 connections at most. |
| Path traversal, dotfiles | Static files answered from an in-memory map; dotfiles never loaded and refused by a rule. |
| Secrets | `.env` is git-ignored; any secret can come from a file; database traffic is always encrypted and a remote database must present a valid certificate. |
| Unknown inputs | Property tests: tens of thousands of random and hostile inputs against every crate and the HTTP routes. |
| Vulnerable dependencies | `cargo audit` (one documented, non-compiled exception in `.cargo/audit.toml`) and `npm audit`: 0 findings. |

## Performance (Windows laptop, Python client as the bottleneck)

| | |
|---|---|
| `/api/profile` from MySQL | 5,100 req/s, p99 3.5 ms |
| Home page and assets | 6,300 req/s, p99 2.9 ms |
| Page weight | 60 KB Brotli for the whole site; the app's own code is 4.8 KB, React cached separately |
| fuzzrank, 50,000 lines | 55–215 µs per query |
| Server binary | 3 MB |

## Run it

Needs Rust (with `wasm32-unknown-unknown`), Node 20+ and MySQL 8.

```bash
rustup target add wasm32-unknown-unknown       # once
server/scripts/setup-mysql.sh -u root -p       # once: database, accounts, .env
./build.sh                                     # tests and builds everything
cd server && ../target/release/portfolio seed  # load the content
../target/release/portfolio                    # http://127.0.0.1:8080
```

Settings (in `server/.env`, the environment, or a file named by `KEY_FILE`): `MYSQL_DSN`, `MYSQL_ADMIN_DSN`, `MYSQL_SSL_CA`, `IP_HASH_SALT`, `SMTP_USER`, `SMTP_PASS`, `MAIL_TO`, `ADDR`, `STATIC_DIR`, `TRUST_PROXY`, `HTTPS`, `RATE_LIMIT_PER_MIN`, `MAX_CONNECTIONS`. `portfolio migrate` brings an existing database up to date. Hosting: see [DEPLOY.md](DEPLOY.md).

## Tests

```bash
cargo test --workspace                                      # crates + server (in-memory store)
TEST_MYSQL_URL=mysql://root@127.0.0.1:3306 cargo test -p portfolio-server   # plus a real MySQL round trip
cd web && npm test
```
