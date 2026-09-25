#!/usr/bin/env bash
# Requests (or renews) a Let's Encrypt certificate via DNS-01, for any of a small set of DNS
# providers, for one or more hostnames. DNS-01 needs no inbound port 80/443 exposure, which fits
# a firewall-restricted server like this one's.
#
# Usage:
#   request-cert.sh --provider <ovh|cloudflare|route53|digitalocean> \
#                    --email you@example.com \
#                    [--credentials /path/to/credentials.ini] \
#                    hostname1.example.com [hostname2.example.com ...]
#
# Examples:
#   request-cert.sh --provider ovh --email me@lasea.com \
#                    --credentials /etc/letsencrypt/ovh.ini laseask.lasea.com
#
#   request-cert.sh --provider cloudflare --email me@lasea.com \
#                    --credentials /etc/letsencrypt/cloudflare.ini laseask.lasea.com chat.lasea.com
#
# route53 doesn't take a credentials file — it reads AWS credentials from the environment or
# ~/.aws/credentials, so --credentials is ignored for that provider.
#
# See deploy/tls/credentials.*.example.ini for what each provider's credentials file needs.

set -euo pipefail

PROVIDER=""
EMAIL=""
CREDENTIALS=""
HOSTNAMES=()

usage() {
  sed -n '2,20p' "$0" | sed 's/^# \{0,1\}//'
  exit 1
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --provider) PROVIDER="$2"; shift 2 ;;
    --email) EMAIL="$2"; shift 2 ;;
    --credentials) CREDENTIALS="$2"; shift 2 ;;
    -h|--help) usage ;;
    --*) echo "Unknown flag: $1"; usage ;;
    *) HOSTNAMES+=("$1"); shift ;;
  esac
done

if [[ -z "$PROVIDER" || -z "$EMAIL" || ${#HOSTNAMES[@]} -eq 0 ]]; then
  usage
fi

case "$PROVIDER" in
  ovh)
    PLUGIN_FLAG="--dns-ovh"
    CRED_FLAG="--dns-ovh-credentials"
    PROPAGATION_FLAGS=(--dns-ovh-propagation-seconds 30)
    ;;
  cloudflare)
    PLUGIN_FLAG="--dns-cloudflare"
    CRED_FLAG="--dns-cloudflare-credentials"
    PROPAGATION_FLAGS=(--dns-cloudflare-propagation-seconds 30)
    ;;
  digitalocean)
    PLUGIN_FLAG="--dns-digitalocean"
    CRED_FLAG="--dns-digitalocean-credentials"
    PROPAGATION_FLAGS=(--dns-digitalocean-propagation-seconds 30)
    ;;
  route53)
    PLUGIN_FLAG="--dns-route53"
    CRED_FLAG=""
    PROPAGATION_FLAGS=()
    ;;
  *)
    echo "Unknown provider '$PROVIDER'. Supported: ovh, cloudflare, route53, digitalocean"
    exit 1
    ;;
esac

if [[ -n "$CRED_FLAG" && -z "$CREDENTIALS" ]]; then
  echo "Provider '$PROVIDER' needs --credentials <path>"
  exit 1
fi

DOMAIN_ARGS=()
for h in "${HOSTNAMES[@]}"; do
  DOMAIN_ARGS+=(-d "$h")
done

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DEPLOY_HOOK="$SCRIPT_DIR/copy-to-app.sh"

CMD=(certbot certonly --non-interactive --agree-tos -m "$EMAIL" "$PLUGIN_FLAG")
if [[ -n "$CRED_FLAG" ]]; then
  CMD+=("$CRED_FLAG" "$CREDENTIALS")
fi
CMD+=("${PROPAGATION_FLAGS[@]}" "${DOMAIN_ARGS[@]}" --deploy-hook "$DEPLOY_HOOK")

echo "Running: ${CMD[*]}"
sudo "${CMD[@]}"

# --deploy-hook only fires on an actual (re)issuance; on a brand new cert it does run, but run it
# explicitly too so the very first `opt/laseask/tls` copy exists even if certbot's hook timing
# differs, and so this script is idempotent to re-run.
sudo env RENEWED_LINEAGE="/etc/letsencrypt/live/${HOSTNAMES[0]}" "$DEPLOY_HOOK"

echo
echo "Certificate issued for: ${HOSTNAMES[*]}"
echo "Copied to /opt/laseask/tls/ (readable by the laseask service user) — see"
echo "deploy/tls/copy-to-app.sh. Enable TLS in backend/config.toml:"
echo
echo "  [tls]"
echo "  enabled = true"
echo "  cert_path = \"/opt/laseask/tls/fullchain.pem\""
echo "  key_path  = \"/opt/laseask/tls/privkey.pem\""
echo
echo "Then restart the backend once. Renewals after that are picked up automatically —"
echo "the deploy-hook above re-copies the files and restarts the service for you."
