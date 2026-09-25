# Deploying LASEASK on Ubuntu

The step-by-step guides are in the main **`README.md`**, one per install type:

- **Docker**: section 2 (first install) and section 3 (updates).
- **Bare metal / systemd**: section 4 (first install) and section 5 (updates).

This folder contains:

- **`laseask.service`**: the systemd unit used by the bare-metal install. It runs
  `/opt/laseask/laseask-backend` as the `laseask` user, with `/opt/laseask/config.toml` as config.
- **`tls/`**: optional HTTPS with a Let's Encrypt certificate (see `tls/README.md`).

The service has no login/auth: it must only be reachable from trusted machines. Restrict access at
the firewall (network firewall and/or `ufw`), not in the app. With Docker, `ufw` does not filter
published container ports, so use the network firewall.
