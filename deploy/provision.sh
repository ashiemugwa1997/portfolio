#!/usr/bin/env bash
# One-time setup of a fresh Ubuntu 24.04 / 26.04 server for the portfolio, run as root.
# Safe to run again: every step checks before it changes anything.
#   scp -P 2222 -r web/dist root@HOST:/tmp/dist && ssh -p 2222 root@HOST 'bash -s' < deploy/provision.sh
# The server must already listen on 2222 as well as 22 (Hetzner cloud config, sshd_config.d/05-ports.conf).
# Secrets are not handled here: the SMTP app password goes in afterwards with
#   systemd-creds encrypt --name=smtp_pass - /etc/credstore.encrypted/smtp_pass
set -euo pipefail

DOMAIN=${DOMAIN:-samugwambi.site}
REPO=${REPO:-https://github.com/ashiemugwa1997/portfolio.git}
SRC=/opt/src/portfolio
APP=/opt/portfolio
export DEBIAN_FRONTEND=noninteractive

echo "== swap (a 1 GB server needs it to build)"
if ! swapon --show | grep -q /swapfile; then
  fallocate -l 2G /swapfile && chmod 600 /swapfile && mkswap -q /swapfile && swapon /swapfile
  grep -q '^/swapfile' /etc/fstab || echo '/swapfile none swap sw 0 0' >> /etc/fstab
fi
sysctl -q vm.swappiness=10 && echo 'vm.swappiness=10' > /etc/sysctl.d/90-swap.conf

echo "== packages"
apt-get update -q
apt-get -yq full-upgrade
apt-get -yq install mysql-server caddy ufw fail2ban unattended-upgrades \
  build-essential pkg-config libssl-dev git curl ca-certificates
dpkg-reconfigure -f noninteractive unattended-upgrades

echo "== ssh: keys only"
cat > /etc/ssh/sshd_config.d/10-portfolio.conf <<'EOF'
PasswordAuthentication no
KbdInteractiveAuthentication no
PermitRootLogin prohibit-password
MaxAuthTries 3
X11Forwarding no
AllowTcpForwarding no
AllowAgentForwarding no
EOF
sshd -t && systemctl reload ssh

echo "== fail2ban: both SSH ports, longer bans for repeat offenders"
cat > /etc/fail2ban/jail.d/portfolio.conf <<'EOF'
[sshd]
enabled = true
port = 22,2222
maxretry = 4
findtime = 10m
bantime = 1h
bantime.increment = true
bantime.maxtime = 1w

[recidive]
enabled = true
banaction = nftables[type=allports]
EOF
systemctl restart fail2ban

echo "== updates reboot at 02:00 UTC when needed"
cat > /etc/apt/apt.conf.d/52portfolio-reboot <<'EOF'
Unattended-Upgrade::Automatic-Reboot "true";
Unattended-Upgrade::Automatic-Reboot-Time "02:00";
EOF

echo "== firewall: ssh, http, https"
ufw allow OpenSSH >/dev/null
ufw allow 2222/tcp >/dev/null   # SSH for networks that block 22
ufw allow 80/tcp >/dev/null
ufw allow 443 >/dev/null
ufw --force enable >/dev/null

echo "== MySQL: local only"
cat > /etc/mysql/mysql.conf.d/zz-portfolio.cnf <<'EOF'
[mysqld]
bind-address = 127.0.0.1
mysqlx = OFF
require_secure_transport = ON
# sized for a 1 GB server: one small site
performance_schema = OFF
innodb_buffer_pool_size = 64M
innodb_log_buffer_size = 8M
key_buffer_size = 8M
tmp_table_size = 16M
max_heap_table_size = 16M
max_connections = 40
EOF
systemctl restart mysql

echo "== build"
if ! command -v cargo >/dev/null && [ ! -x /root/.cargo/bin/cargo ]; then
  curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
fi
export PATH=/root/.cargo/bin:$PATH
if [ -d "$SRC/.git" ]; then git -C "$SRC" pull -q --ff-only; else git clone -q "$REPO" "$SRC"; fi
cargo build --release -q --manifest-path "$SRC/Cargo.toml" -p portfolio-server

echo "== app files"
id portfolio >/dev/null 2>&1 || useradd --system --home "$APP" --shell /usr/sbin/nologin portfolio
install -d -o root -g root -m 755 "$APP" "$APP/web"
install -d -o portfolio -g portfolio -m 750 "$APP/server" "$APP/server/data"
install -m 755 "$SRC/target/release/portfolio" "$APP/portfolio"
rm -rf "$APP/server/content" && cp -r "$SRC/server/content" "$APP/server/content"
install -d "$APP/server/scripts" && install -m 755 "$SRC/server/scripts/setup-mysql.sh" "$APP/server/scripts/"
if [ -d /tmp/dist ]; then rm -rf "$APP/web/dist" && mv /tmp/dist "$APP/web/dist"; fi
[ -f "$APP/web/dist/index.html" ] || { echo "web/dist missing: scp it to /tmp/dist first"; exit 1; }

echo "== database"
if ! grep -q '^MYSQL_DSN=' "$APP/server/.env" 2>/dev/null; then
  # root signs in over the local socket (auth_socket): "-h localhost" means the socket to the client;
  # the app itself connects over TCP to 127.0.0.1
  (cd "$APP/server" && PORTFOLIO_DB_HOST=localhost scripts/setup-mysql.sh -u root)
  sed -i 's#@localhost:3306/#@127.0.0.1:3306/#' "$APP/server/.env"
fi
chown portfolio:portfolio "$APP/server/.env" && chmod 600 "$APP/server/.env"
(cd "$APP/server" && sudo -u portfolio "$APP/portfolio" migrate)

echo "== nightly database backup, kept 14 days"
install -d -m 700 /var/backups/portfolio
cat > /usr/local/sbin/portfolio-backup <<'EOF'
#!/bin/sh
set -eu
f=/var/backups/portfolio/portfolio-$(date +%F).sql.gz
mysqldump --single-transaction --routines --no-tablespaces portfolio | gzip -9 > "$f.tmp" && mv "$f.tmp" "$f"
chmod 600 "$f"
find /var/backups/portfolio -name 'portfolio-*.sql.gz' -mtime +14 -delete
EOF
chmod 700 /usr/local/sbin/portfolio-backup
echo '17 3 * * * root /usr/local/sbin/portfolio-backup' > /etc/cron.d/portfolio-backup

echo "== service and HTTPS"
install -m 644 "$SRC/deploy/portfolio.service" /etc/systemd/system/portfolio.service
install -d -m 700 /etc/credstore.encrypted
sed "s/samugwambi.site/$DOMAIN/g" "$SRC/deploy/Caddyfile" > /etc/caddy/Caddyfile
systemctl daemon-reload
systemctl reload caddy || systemctl restart caddy

echo "Done. Next: SMTP_USER / MAIL_TO in $APP/server/.env, the smtp_pass credential, 'portfolio seed' once, then systemctl enable --now portfolio"
