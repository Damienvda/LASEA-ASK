#!/usr/bin/env bash
# Installs certbot plus the DNS-01 plugin for one provider, via apt (Ubuntu).
#
# Usage: install-certbot.sh <ovh|cloudflare|route53|digitalocean>

set -euo pipefail

PROVIDER="${1:-}"

case "$PROVIDER" in
  ovh)          PKG="python3-certbot-dns-ovh" ;;
  cloudflare)   PKG="python3-certbot-dns-cloudflare" ;;
  route53)      PKG="python3-certbot-dns-route53" ;;
  digitalocean) PKG="python3-certbot-dns-digitalocean" ;;
  *)
    echo "Usage: $0 <ovh|cloudflare|route53|digitalocean>"
    echo "Unknown or missing provider: '${PROVIDER}'"
    exit 1
    ;;
esac

sudo apt-get update
sudo apt-get install -y certbot "$PKG"

echo
echo "Installed certbot + $PKG."
echo "Next: create a credentials file (see deploy/tls/credentials.*.example.ini) and run"
echo "deploy/tls/request-cert.sh."
