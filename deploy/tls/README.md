# TLS via Let's Encrypt (DNS-01, multi-provider)

Certificates are requested with `certbot` using a DNS-01 challenge, which only needs your DNS
provider's API — no inbound port 80/443 exposure required, so it works fine with the
firewall-restricted deployment described in the main README.

The backend itself never talks to Let's Encrypt or your DNS provider; it just loads a cert/key file
pair and re-reads them periodically so renewals are picked up automatically (see `[tls]` in
`backend/config.rs` / `config.example.toml`).

Note: `certbot`'s own `/etc/letsencrypt/live/*/privkey.pem` is root-only, which the unprivileged
`laseask` service user can't read. `request-cert.sh` wires up a `--deploy-hook`
(`copy-to-app.sh`) that copies the cert/key to `/opt/laseask/tls/` with the right ownership every
time a cert is issued or renewed — point `[tls]` at that copy, not at `/etc/letsencrypt` directly.
This assumes the systemd deployment layout from the main `deploy/README.md` (`/opt/laseask`,
`laseask` user/service); adjust `APP_DIR`/`APP_USER`/`SERVICE` at the top of `copy-to-app.sh` if
yours differs, or if you're on the Docker deployment option (see its note below).

## One-time setup

0. Make the scripts executable once (git doesn't reliably preserve the executable bit from a
   Windows checkout):
   ```bash
   chmod +x deploy/tls/*.sh
   ```

1. Install certbot + the plugin for your DNS provider:
   ```bash
   deploy/tls/install-certbot.sh ovh          # or: cloudflare, route53, digitalocean
   ```

2. Set up credentials for that provider:
   - **ovh / cloudflare / digitalocean**: copy the matching
     `deploy/tls/credentials.<provider>.example.ini`, e.g. to `/etc/letsencrypt/ovh.ini`, fill in
     real values, then `chmod 600` it.
   - **route53**: no file needed — set `AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY` in the
     environment, or use `~/.aws/credentials`, for a user with `route53:GetChange` and
     `route53:ChangeResourceRecordSets` permission on the relevant hosted zone.

3. Request the certificate, for one or more hostnames:
   ```bash
   deploy/tls/request-cert.sh --provider ovh --email you@lasea.com \
     --credentials /etc/letsencrypt/ovh.ini laseask.lasea.com
   ```
   Add more hostnames to cover several names with one cert:
   ```bash
   deploy/tls/request-cert.sh --provider ovh --email you@lasea.com \
     --credentials /etc/letsencrypt/ovh.ini laseask.lasea.com chat.lasea.com
   ```

4. Enable TLS in `backend/config.toml` (the script already copied the cert/key to
   `/opt/laseask/tls/`):
   ```toml
   [tls]
   enabled = true
   cert_path = "/opt/laseask/tls/fullchain.pem"
   key_path  = "/opt/laseask/tls/privkey.pem"
   ```

5. Restart the backend once (`sudo systemctl restart laseask`, or restart the container). It now
   serves HTTPS on the same port from `[server].port`.

## Renewal

`certbot` (installed via apt) sets up its own systemd timer (`certbot.timer`) that checks for
renewal twice a day and renews automatically starting ~30 days before expiry — nothing to run
manually. On a successful renewal, the `--deploy-hook` re-copies the cert/key to
`/opt/laseask/tls/` and restarts the `laseask` service. Even if that restart didn't happen for some
reason, the backend also re-reads the cert files on its own every `[tls].reload_check_seconds`
(default 6h), so it's picked up either way.

To test the renewal path without waiting: `sudo certbot renew --dry-run`.

## If you're using the Docker deployment option instead

`copy-to-app.sh` assumes the systemd layout. For Docker, either mount `/etc/letsencrypt` (or the
`/opt/laseask/tls` copy) into the container read-only and point `[tls]` at the mounted path, or
adjust the script's restart step to `docker restart laseask` instead of `systemctl restart`.

## Adding another provider

Certbot has DNS-01 plugins for most major providers beyond the four wired up here (e.g. Azure,
Google Cloud DNS, GoDaddy, Namecheap via third-party plugins). To add one: install its
`python3-certbot-dns-<provider>` package, add a case to `deploy/tls/request-cert.sh` with its
plugin flag and credentials flag (check `certbot <plugin-flag> --help` for the exact names), and a
matching `credentials.<provider>.example.ini` documenting what it expects.
