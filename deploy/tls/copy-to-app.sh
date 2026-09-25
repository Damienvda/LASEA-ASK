#!/usr/bin/env bash
# certbot --deploy-hook target. certbot's own /etc/letsencrypt/live/*/privkey.pem is root-only,
# which the unprivileged laseask service user can't read, so this copies the renewed cert/key
# somewhere it can, then restarts the service to pick them up immediately (the backend also
# re-reads them periodically on its own, so this restart is a nice-to-have, not load-bearing).
#
# certbot runs this as root (it's invoked from request-cert.sh's `sudo certbot ...`) and sets
# $RENEWED_LINEAGE to the live directory for the domain that was just issued/renewed, e.g.
# /etc/letsencrypt/live/laseask.lasea.com — see https://eff-certbot.readthedocs.io/en/stable/using.html#renewal

set -euo pipefail

APP_DIR="/opt/laseask"
DEST="$APP_DIR/tls"
APP_USER="laseask"
SERVICE="laseask"

mkdir -p "$DEST"
cp "$RENEWED_LINEAGE/fullchain.pem" "$DEST/fullchain.pem"
cp "$RENEWED_LINEAGE/privkey.pem" "$DEST/privkey.pem"
chown -R "$APP_USER:$APP_USER" "$DEST"
chmod 644 "$DEST/fullchain.pem"
chmod 600 "$DEST/privkey.pem"

if systemctl is-active --quiet "$SERVICE" 2>/dev/null; then
  systemctl restart "$SERVICE"
fi
