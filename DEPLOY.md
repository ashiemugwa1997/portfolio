# Deploying the portfolio

The server is built to sit behind a TLS-terminating proxy on a Linux box. Everything here is in `deploy/`.

## 0. Domain and server

The domain is `samugwambi.site` (Cloudflare Registrar). In the Cloudflare dashboard → DNS:

| Type | Name | Content | Proxy |
|---|---|---|---|
| A | `@` | the server's IPv4 address | DNS only (grey cloud) |
| AAAA | `@` | the server's IPv6 address, if it has one | DNS only |
| CNAME | `www` | `samugwambi.site` | DNS only |

Keep the cloud grey so Caddy can get its own certificate; it can be turned orange later (then SSL/TLS mode **Full (strict)**). Leave iCloud's MX, TXT and CNAME mail records as they are.

A small server is enough (Hetzner CX22 or Oracle Cloud Always Free, Ubuntu 24.04): install MySQL 8 and Caddy, run `server/scripts/setup-mysql.sh`, copy the release binary, `server/content` and `web/dist` to `/opt/portfolio`, then follow the steps below. `deploy/provision.sh` does all of this on a fresh server in one run.

## 1. HTTPS

Pick one:

- **Caddy** (`deploy/Caddyfile`): certificates are automatic. Replace the domain and reload.
- **nginx** (`deploy/nginx.conf`): get a certificate with certbot, then reload. Its `limit_req` is a request limit shared by every copy of the server behind it.

Then run the server with `HTTPS=1` (sends HSTS) and `TRUST_PROXY=1` (reads the visitor's address from the proxy). Both proxies **overwrite** `X-Forwarded-For`, so a visitor cannot pretend to be someone else. Never set `TRUST_PROXY=1` when the server is reachable directly.

`deploy/portfolio.service` runs it under systemd as an unprivileged user, sandboxed, listening only on `127.0.0.1`.

## 2. Database on another machine

Connections are always encrypted. When the database is not on the same machine, the server also checks its certificate names the host (`VerifyIdentity`). If the database uses a private certificate authority, set `MYSQL_SSL_CA=/path/to/ca.pem`. An explicit `ssl-mode=` in the connection string overrides this.

After an upgrade, run `portfolio migrate` (admin account): it adds missing tables and columns and never touches content.

## 3. Email account and secrets

### iCloud+ with samugwambi.site (optional)

1. iPhone: Settings → your name → iCloud → Custom Email Domain → add `samugwambi.site` (bought through Cloudflare). Choose to add the records automatically with Cloudflare, or add the MX, SPF (TXT) and DKIM (CNAME) records it lists in the Cloudflare dashboard yourself.
2. Create the address the site sends from, e.g. `hello@samugwambi.site`.
3. account.apple.com → Sign-In and Security → App-Specific Passwords → create one called "Portfolio".
4. Settings:

   ```
   SMTP_HOST=smtp.mail.me.com
   SMTP_PORT=587
   SMTP_USER=<your Apple ID email>
   MAIL_FROM=hello@samugwambi.site
   MAIL_TO=<where you want messages>
   ```

   and the app-specific password as the `smtp_pass` credential (below), never in `.env`.

iCloud signs in with the Apple ID; `MAIL_FROM` is the custom-domain address the message comes from. If the server is ever breached, revoke the app-specific password at account.apple.com: the Apple ID itself is not exposed.

### Gmail (what the live site uses)

Use a separate Gmail account just for the site: turn on 2-Step Verification, create an App password called "Portfolio", set `SMTP_USER` to that account and `MAIL_TO` to your real address. `SMTP_HOST` defaults to Gmail.

Any setting can come from a file instead of the environment: `SMTP_PASS_FILE`, `MYSQL_DSN_FILE`, `IP_HASH_SALT_FILE` and so on. The systemd unit uses this with encrypted credentials, so the app password is never in `.env` or the process environment.

`IP_HASH_SALT` (64 hex characters, `openssl rand -hex 32`) keeps the stored sender hashes unlinkable to addresses. Keep it stable: changing it resets the contact-form limits.

## 4. Limits across restarts and servers

Contact-form limits are counted in MySQL (3 an hour per sender, 50 a day for the site), so they survive restarts and are shared by every copy of the server. Senders are stored only as a salted SHA-256 hash, never as an address. The general request limit (120 a minute per address) is per process; the nginx `limit_req` covers all of them.

## 5. Testing

- `cargo test --workspace` runs about 120 tests, including property tests that throw tens of thousands of random and hostile inputs at the search, the limiter, the security layer, the contact form and the HTTP routes.
- `TEST_MYSQL_URL=mysql://root@127.0.0.1:3306 cargo test -p portfolio-server` adds a real MySQL round trip.
- Before going public, have the live site tested by someone independent. A basic scan you can run yourself: [Mozilla Observatory](https://developer.mozilla.org/en-US/observatory) and [SSL Labs](https://www.ssllabs.com/ssltest/) against your domain; both should score A or better with this setup.

## Checklist

- [ ] Domain points at the server; Caddy or nginx installed with the config from `deploy/`
- [ ] `server/.env` (mode 600) with `MYSQL_DSN`, `IP_HASH_SALT`, `SMTP_HOST`, `SMTP_USER`, `MAIL_FROM`, `MAIL_TO`; app-specific password as an encrypted credential
- [ ] `portfolio seed` once, `portfolio migrate` after each upgrade
- [ ] `HTTPS=1`, `TRUST_PROXY=1`, `ADDR=127.0.0.1:8080`
- [ ] Firewall: only 80 and 443 open; MySQL not reachable from outside
- [ ] Back up the `portfolio` database (contact messages live there)
