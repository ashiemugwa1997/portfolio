#!/usr/bin/env bash
# Web ports (80, 443) open to Cloudflare only, so nobody can skip its proxy by using the server's IP.
# SSH is untouched. Safe to rerun: run weekly to pick up changes to https://www.cloudflare.com/ips/
set -euo pipefail

ranges=$(curl -sf --max-time 20 https://www.cloudflare.com/ips-v4; echo; curl -sf --max-time 20 https://www.cloudflare.com/ips-v6)
count=$(echo "$ranges" | grep -c /)
# a short or empty answer must never close the site
[ "$count" -ge 15 ] || { echo "only $count Cloudflare ranges fetched; leaving the firewall as it is"; exit 1; }

# drop the old web rules (open to all, or to an older Cloudflare list), newest first so numbers stay valid
ufw status numbered | grep -E ' (80|443|80,443)(/tcp|/udp)? ' | grep -oE '^\[ *[0-9]+\]' | tr -d '[] ' | sort -rn |
  while read -r n; do ufw --force delete "$n" >/dev/null; done

for r in $ranges; do
  ufw allow proto tcp from "$r" to any port 80,443 comment cloudflare >/dev/null
done
echo "web ports open to $count Cloudflare ranges"
