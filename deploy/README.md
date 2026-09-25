# Deploying LASEASK on Ubuntu

The service has no login/auth — it's meant to be reachable only from trusted hosts. Restrict
access at the firewall (ufw/iptables/security group), not in the app.

## Option A: systemd (bare binary)

On the Ubuntu server:

```bash
sudo useradd --system --home /opt/laseask --shell /usr/sbin/nologin laseask
sudo mkdir -p /opt/laseask
```

From your build machine (or build directly on the server if it has the Rust toolchain):

```bash
cd backend
cargo build --release
```

Copy to the server:

```bash
scp target/release/laseask-backend  server:/opt/laseask/
scp -r ../frontend                  server:/opt/laseask/frontend
scp config.toml                     server:/opt/laseask/config.toml   # your real keys, not the example
```

Install and start the service:

```bash
sudo cp deploy/laseask.service /etc/systemd/system/laseask.service
sudo chown -R laseask:laseask /opt/laseask
sudo systemctl daemon-reload
sudo systemctl enable --now laseask
sudo systemctl status laseask
```

Restrict access with ufw, e.g. only from your desktop's IP:

```bash
sudo ufw allow from <your-desktop-ip> to any port 8787 proto tcp
```

## Option B: Docker

From the repo root:

```bash
docker build -f backend/Dockerfile -t laseask-backend .
docker run -d --name laseask \
  -p 8787:8787 \
  -v /opt/laseask/config.toml:/app/config.toml:ro \
  --restart unless-stopped \
  laseask-backend
```

## Either way

Point your browser at `http://<server-ip>:8787`.

## HTTPS (optional)

Want a real hostname + a trusted cert instead of a bare IP over plain HTTP? See
`deploy/tls/README.md` — it requests a Let's Encrypt cert via DNS-01 (OVH, Cloudflare, Route53 or
DigitalOcean) and the backend serves HTTPS directly once `[tls].enabled = true`.
