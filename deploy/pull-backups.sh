#!/usr/bin/env bash
# Copy the server's nightly database backups that are not here yet, so a lost server is not lost data.
#   deploy/pull-backups.sh [destination]     (default: /c/portfolio-backups, outside any synced folder)
# Keeps everything it has copied; the server itself keeps 14 days.
set -euo pipefail

HOST=${PORTFOLIO_HOST:-root@95.216.187.226}
PORT=${PORTFOLIO_SSH_PORT:-2222}
KEY=${PORTFOLIO_SSH_KEY:-$HOME/.ssh/portfolio_ed25519}
DEST=${1:-/c/portfolio-backups}
SSH=(ssh -i "$KEY" -o IdentitiesOnly=yes -o ConnectTimeout=15 -p "$PORT")

mkdir -p "$DEST"
copied=0
for f in $("${SSH[@]}" "$HOST" 'ls /var/backups/portfolio/ | grep -E "^portfolio-[0-9-]+\.sql\.gz$"'); do
  [ -s "$DEST/$f" ] && continue
  scp -q -i "$KEY" -o IdentitiesOnly=yes -P "$PORT" "$HOST:/var/backups/portfolio/$f" "$DEST/$f.part"
  gzip -t "$DEST/$f.part" && mv "$DEST/$f.part" "$DEST/$f"
  copied=$((copied + 1))
done
echo "$copied new backup(s) copied; $(ls "$DEST"/portfolio-*.sql.gz 2>/dev/null | wc -l) kept in $DEST"
