# LASEASK

A self-hosted, multi-provider AI chat interface:

- **`backend/`**: a Rust (Axum) web service. It serves the chat UI and forwards your messages to
  the AI you pick (Claude, GPT or Mistral), streaming the reply back live.
- **`frontend/`**: the chat UI (plain HTML/CSS/JS, no build step), served by the backend.
- **`deploy/`**: the systemd service file and the HTTPS (Let's Encrypt) tooling.

You bring your own API key(s), entered once in the server's config file. There is **no login
system**: the server must only be reachable from trusted machines, and access control is done at
the firewall.

The backend can also connect to **MCP servers** (e.g. a FortiAnalyzer MCP endpoint) so the AI can
call their tools, the same way tools work in Claude Code. In the UI you pick the AI with a button
and tick which MCP servers it may use.

---

## Which guide do I follow?

There are two ways to run LASEASK on an Ubuntu server. They are **separate**: pick one, and only
ever follow the guides for that one.

| | Docker | Bare metal (systemd) |
|---|---|---|
| What runs | A Docker container | A normal Linux service |
| Rust on the server | Not needed (compiled inside Docker) | Installed on the server |
| Config file | `~/LASEASK/backend/config.toml` | `/opt/laseask/config.toml` |
| Logs | `docker logs -f laseask` | `sudo journalctl -u laseask -f` |
| Restart | `docker restart laseask` | `sudo systemctl restart laseask` |
| **First install** | **Section 2** | **Section 4** |
| **Update to a new version** | **Section 3** | **Section 5** |

**Not sure which one your server uses?** On the server, run `docker ps`: if you see a `laseask`
line, it's Docker. Otherwise run `systemctl status laseask`: if it says `active (running)`, it's
bare metal.

Both guides use section 1 for the config file content.

---

## 1. The config file (both installs)

The config file holds your API keys and settings. It is **never** in git (it's gitignored): each
install guide tells you where to create it, from `backend/config.example.toml`.

```toml
default_provider = "anthropic"

[server]
host = "0.0.0.0"
port = 8787

[providers.anthropic]
api_key = "sk-ant-your-real-key"
default_model = "claude-sonnet-4-5"

[providers.openai]
api_key = "sk-your-real-key"
default_model = "gpt-4.1"

[providers.mistral]
api_key = "your-real-mistral-key"
default_model = "mistral-large-latest"

# Optional: one block per MCP server whose tools the AI may use.
[mcp.fortianalyzer]
url = "https://your-mcp-server/mcp"
bearer_token = "your-bearer-token"
```

### AI providers

- Keep a `[providers.X]` block only for the AIs you have a key for, and delete the others.
  `default_provider` must name one that exists.
- Each block becomes a button in the sidebar's **AI** section.
- Keys: Anthropic at https://console.anthropic.com (Settings → API Keys), OpenAI at
  https://platform.openai.com/api-keys, Mistral at https://console.mistral.ai/api-keys. These are
  pay-per-use API keys, billed separately from any Claude.ai or ChatGPT subscription.
- `default_model` is the model used by default. Any other model name (e.g. `mistral-medium-latest`,
  `mistral-small-latest`) can be typed in the UI's **Model** box.

### MCP servers (optional)

- The name after `mcp.` (`fortianalyzer` above) becomes the prefix of every tool from that server,
  e.g. `fortianalyzer__get_alerts`. Add as many `[mcp.X]` blocks as you have servers.
- Tools work with Claude, GPT and Mistral. Pick a model that supports function calling (e.g.
  `mistral-large-latest`, `mistral-medium-latest`, `mistral-small-latest`).
- At start-up the backend connects to each server and logs
  `MCP 'fortianalyzer': connected, N tool(s) available`. A server that's unreachable is logged as a
  warning and skipped: the rest of the app still starts.
- In the UI's **Tools (MCP)** section, each server has a checkbox, a status dot (green = connected)
  and its tool count. Only the ticked servers' tools are offered to the AI. An unreachable server
  shows greyed out as "offline".
- When the AI uses a tool, a "🔧 fortianalyzer__X" chip appears above its reply.
- The **MCP only** switch forces the AI to call a tool before answering, instead of answering from
  its own knowledge. It's only available with Claude, GPT or Mistral and at least one ticked server.
- Already using the FortiAnalyzer MCP in Claude Code? Its URL and token are in `~/.claude.json` on
  your PC, under `mcpServers.fortianalyzer` (`url` and the `Authorization: Bearer ...` header).

### HTTPS (optional)

Off by default (plain HTTP). To serve HTTPS with a real hostname, see section 6.

---

## 2. First install with Docker (beginner guide)

From a bare Ubuntu server to LASEASK running in Docker. About 15 minutes. Copy the commands one at a
time, in order, and check the result after each step. Rust is **not** installed on the server: the
code is compiled inside Docker.

### Step 1: Connect to the server

From a terminal on your PC (PowerShell works):

```bash
ssh <your-user>@<server-ip>
```

Type the password when asked (nothing shows while you type, that's normal). Every command below
runs **on the server**.

### Step 2: Install Docker

```bash
curl -fsSL https://get.docker.com | sudo sh
sudo apt-get update && sudo apt-get install -y docker-buildx-plugin git
sudo usermod -aG docker "$USER"
```

Then **log out and back in** (`exit`, then `ssh` again) so the last line takes effect, and check:

```bash
docker run --rm hello-world
```

It must print `Hello from Docker!`. The `docker-buildx-plugin` package is required: the build
uses BuildKit features that the old Docker build engine doesn't have.

### Step 3: Download the code

```bash
cd ~
git clone https://github.com/Damienvda/LASEA-ASK.git LASEASK
cd ~/LASEASK
```

The code is now in `~/LASEASK`. **Always run the next steps from this folder.**

### Step 4: Create the config file

```bash
cp backend/config.example.toml backend/config.toml
chmod 600 backend/config.toml
nano backend/config.toml
```

Fill it in as explained in section 1 (API keys, MCP servers). In `nano`: **Ctrl+O** then Enter to
save, **Ctrl+X** to quit.

`chmod 600` makes the file readable only by you. The file is never built into the Docker image: it
is plugged into the container when it starts (the `-v` in step 6).

### Step 5: Build the image

```bash
docker build -f backend/Dockerfile -t laseask-backend .
```

- **Don't forget the final `.`**: it means "the current folder", which must be `~/LASEASK`.
- The first build takes several minutes (it compiles Rust). It succeeded if it ends with
  `naming to docker.io/library/laseask-backend` and no red `ERROR`.

### Step 6: Start the container

```bash
docker run -d --name laseask -p 8787:8787 \
  -v "$HOME/LASEASK/backend/config.toml:/app/config.toml:ro" \
  --restart unless-stopped \
  laseask-backend
```

- Paste the command as a whole: the `\` at the end of a line means "continued on the next line".
- It prints a long ID: that's success.
- `--restart unless-stopped` restarts the app automatically after a crash or a server reboot.

### Step 7: Check it works

```bash
docker ps
docker logs -f laseask
```

- `docker ps` must show `laseask` with a status like `Up 10 seconds`. `Restarting` means the app
  crashes at start-up: read the logs.
- In the logs, look for `LASEASK backend listening on http://0.0.0.0:8787` and, if you configured
  MCP, `MCP 'fortianalyzer': connected, N tool(s) available`.
- **Ctrl+C** stops reading the logs (not the app).
- Open `http://<server-ip>:8787` in your browser and send a test message.

### Step 8: Restrict who can reach it

There is no login, so only trusted machines must reach port 8787. **With Docker, `ufw` does not
protect this port**: Docker writes its own firewall rules, which bypass `ufw`. Restrict access on
the network firewall instead (e.g. a FortiGate policy that only allows your admin machines to reach
`<server-ip>:8787`).

To update later, follow **section 3**.

---

## 3. Updating a Docker install (beginner guide)

Use this every time the code changed and you want the server to run the new version. About 5
minutes.

**Only the config changed** (new API key, new model, new `[providers.X]` or `[mcp.X]` block)? Skip
this guide: edit `~/LASEASK/backend/config.toml` on the server, then run `docker restart laseask`.

### The big picture

```
 Your PC                    GitHub                     Ubuntu server
 (edit code)  -- push -->   (stores the code)  -- pull -->  (build image, run container)
```

- **Image** (`laseask-backend`): the compiled app, like an installer. Built by `docker build`.
- **Container** (`laseask`): a running copy of the image. Started by `docker run`.
- Rebuilding the image does **not** change the running container: you have to replace the
  container (step 6).

### Step 1: Send your changes to GitHub (on your PC)

1. Open **GitHub Desktop** and select the repository.
2. On the left, check the list of changed files. **`backend/config.toml` must never appear here.**
3. Bottom left: type a short summary, click **Commit**.
4. Top: click **Push origin**. Wait until the button goes back to **Fetch origin**.

If someone else pushed the change, skip this step.

### Step 2: Connect to the server

```bash
ssh <your-user>@<server-ip>
```

### Step 3: Go to the project folder

```bash
cd ~/LASEASK
pwd
```

`pwd` must print `/home/<your-user>/LASEASK`. **Run the next steps from this folder.**

### Step 4: Download the new code

```bash
git status
git pull
```

- `git status` should say `nothing to commit, working tree clean`.
- `git pull` lists the changed files, or `Already up to date.` (did you forget **Push** in step 1?).
- **Error `Your local changes ... would be overwritten by merge`**: someone edited a file directly
  on the server. Run `git stash`, then `git pull` again. Your `config.toml` is not affected.

### Step 5: Build the new image (keeping a backup of the old one)

```bash
docker tag laseask-backend laseask-backend:previous
docker build -f backend/Dockerfile -t laseask-backend .
```

- The first line saves the current image as `laseask-backend:previous`, for the rollback below.
- **Don't forget the final `.`**
- **If the build fails, stop here.** The old container is still running and nothing is broken.

### Step 6: Replace the running container

```bash
docker stop laseask
docker rm laseask
docker run -d --name laseask -p 8787:8787 \
  -v "$HOME/LASEASK/backend/config.toml:/app/config.toml:ro" \
  --restart unless-stopped \
  laseask-backend
```

`stop` + `rm` delete the old container (not the image, not your config). The site is down for a
few seconds only.

### Step 7: Check it works

```bash
docker ps
docker logs -f laseask
```

Same checks as in section 2, step 7. In the browser, press **Ctrl+F5** to bypass the cache.

### Step 8 (optional): Clean up

```bash
docker image prune -f
```

Removes old unused images. It keeps `laseask-backend` and `laseask-backend:previous`.

### Rollback: the new version is broken

```bash
docker stop laseask
docker rm laseask
docker run -d --name laseask -p 8787:8787 \
  -v "$HOME/LASEASK/backend/config.toml:/app/config.toml:ro" \
  --restart unless-stopped \
  laseask-backend:previous
```

Only the last `:previous` differs from step 6.

### Common errors (Docker)

| Message | Cause | Fix |
|---|---|---|
| `The container name "/laseask" is already in use` | The old container still exists | `docker stop laseask && docker rm laseask`, then `docker run` again |
| `"/frontend": not found` during the build | Build run from the wrong folder (e.g. `backend/`) | `cd ~/LASEASK`, then build again |
| `docker build requires 1 argument` | The final `.` is missing | Add ` .` at the end of the build command |
| `permission denied ... docker.sock` | Your user isn't in the `docker` group | `sudo usermod -aG docker $USER`, log out and back in |
| `Bind for 0.0.0.0:8787 failed: port is already allocated` | Something else uses port 8787 | `docker ps` to find it, stop it |
| `could not read config file` in the logs | `config.toml` missing, or wrong path in `-v` | `ls ~/LASEASK/backend/config.toml` must list the file |
| `default_provider 'X' has no matching [providers.X] section` | Config error | Fix `config.toml`, then `docker restart laseask` |
| `mistral error 400 ... maximum context length` in the chat | Conversation too long for the model | Start a new chat, or ask for fewer/narrower tool calls |

### Cheat sheet (once you're used to it)

```bash
cd ~/LASEASK && git pull
docker tag laseask-backend laseask-backend:previous
docker build -f backend/Dockerfile -t laseask-backend .
docker stop laseask && docker rm laseask
docker run -d --name laseask -p 8787:8787 \
  -v "$HOME/LASEASK/backend/config.toml:/app/config.toml:ro" \
  --restart unless-stopped laseask-backend
docker logs -f laseask
```

---

## 4. First install on bare metal / systemd (beginner guide)

From a bare Ubuntu server to LASEASK running as a Linux service. About 20 minutes. Copy the
commands one at a time, in order, and check the result after each step.

Two folders are used on the server, don't mix them up:

- **`~/LASEASK`**: the source code (git clone). You pull and compile here.
- **`/opt/laseask`**: what actually runs: `laseask-backend` (the program), `frontend/` (the web
  page) and `config.toml` (your keys). systemd starts the program from here, as a dedicated
  `laseask` user.

### Step 1: Connect to the server

From a terminal on your PC (PowerShell works):

```bash
ssh <your-user>@<server-ip>
```

Every command below runs **on the server**.

### Step 2: Install the build tools and Rust

```bash
sudo apt-get update && sudo apt-get install -y build-essential git curl
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source "$HOME/.cargo/env"
cargo --version
```

- `build-essential` provides the C linker Rust needs. OpenSSL is not needed (the app uses rustls).
- `cargo --version` must print something like `cargo 1.xx.x`. If it says `command not found`, log
  out and back in, then try again.

### Step 3: Download the code

```bash
cd ~
git clone https://github.com/Damienvda/LASEA-ASK.git LASEASK
```

### Step 4: Compile

```bash
cd ~/LASEASK/backend
cargo build --release
```

- The first build takes several minutes.
- It succeeded if it ends with `Finished` and no red `error`. The program is now at
  `~/LASEASK/backend/target/release/laseask-backend`.

### Step 5: Create the service user and the install folder

```bash
sudo useradd --system --home /opt/laseask --shell /usr/sbin/nologin laseask
sudo mkdir -p /opt/laseask
```

The `laseask` user can't log in: it only exists to run the service with minimal rights.

### Step 6: Install the program and the web page

```bash
sudo cp ~/LASEASK/backend/target/release/laseask-backend /opt/laseask/laseask-backend
sudo cp -r ~/LASEASK/frontend /opt/laseask/frontend
```

### Step 7: Create the config file

```bash
sudo cp ~/LASEASK/backend/config.example.toml /opt/laseask/config.toml
sudo nano /opt/laseask/config.toml
```

Fill it in as explained in section 1 (API keys, MCP servers). In `nano`: **Ctrl+O** then Enter to
save, **Ctrl+X** to quit.

Then give everything to the `laseask` user, and make the config readable by it only:

```bash
sudo chown -R laseask:laseask /opt/laseask
sudo chmod 600 /opt/laseask/config.toml
```

### Step 8: Install and start the service

```bash
sudo cp ~/LASEASK/deploy/laseask.service /etc/systemd/system/laseask.service
sudo systemctl daemon-reload
sudo systemctl enable --now laseask
```

`enable --now` starts the service now **and** at every server boot.

### Step 9: Check it works

```bash
sudo systemctl status laseask
sudo journalctl -u laseask -f
```

- `status` must show **`active (running)`** in green. Press **q** to leave that screen.
- In the logs, look for `LASEASK backend listening on http://0.0.0.0:8787` and, if you configured
  MCP, `MCP 'fortianalyzer': connected, N tool(s) available`.
- **Ctrl+C** stops reading the logs (not the app).
- Open `http://<server-ip>:8787` in your browser and send a test message.

### Step 10: Restrict who can reach it

There is no login, so only trusted machines must reach port 8787. With `ufw` (the Ubuntu firewall):

```bash
sudo ufw allow OpenSSH
sudo ufw allow from <your-desktop-ip> to any port 8787 proto tcp
sudo ufw enable
sudo ufw status
```

**Keep the `allow OpenSSH` line first**: enabling `ufw` without it cuts your own SSH connection.
Add one `allow from` line per machine that needs access. Restricting on the network firewall
(e.g. a FortiGate policy) as well is recommended.

To update later, follow **section 5**.

---

## 5. Updating a bare-metal / systemd install (beginner guide)

Use this every time the code changed and you want the server to run the new version. About 5-10
minutes.

**Only the config changed** (new API key, new model, new `[providers.X]` or `[mcp.X]` block)? Skip
this guide: `sudo nano /opt/laseask/config.toml`, then `sudo systemctl restart laseask`.

### The big picture

```
 Your PC                   GitHub                    Ubuntu server
 (edit code) -- push -->   (stores the code) -- pull -->  ~/LASEASK    (source code, compile here)
                                                            | copy program + web page
                                                            v
                                                          /opt/laseask (what systemd runs)
```

### Step 1: Send your changes to GitHub (on your PC)

1. Open **GitHub Desktop** and select the repository.
2. On the left, check the list of changed files. **`backend/config.toml` must never appear here.**
3. Bottom left: type a short summary, click **Commit**.
4. Top: click **Push origin**. Wait until the button goes back to **Fetch origin**.

If someone else pushed the change, skip this step.

### Step 2: Connect to the server

```bash
ssh <your-user>@<server-ip>
```

### Step 3: Download the new code

```bash
cd ~/LASEASK
git status
git pull
```

- `git status` should say `nothing to commit, working tree clean`.
- `git pull` lists the changed files, or `Already up to date.` (did you forget **Push** in step 1?).
- **Error `Your local changes ... would be overwritten by merge`**: run `git stash`, then
  `git pull` again. This throws away edits made directly in the server's clone.

### Step 4: Compile

```bash
cd ~/LASEASK/backend
cargo build --release
```

- Later builds only recompile what changed, so they're faster than the first one.
- **If the build fails, stop here.** The service is still running the old version and nothing is
  broken.

### Step 5: Back up the current version

```bash
sudo cp /opt/laseask/laseask-backend /opt/laseask/laseask-backend.previous
sudo rm -rf /opt/laseask/frontend.previous
sudo cp -r /opt/laseask/frontend /opt/laseask/frontend.previous
```

### Step 6: Install the new version and restart

```bash
sudo systemctl stop laseask
sudo cp ~/LASEASK/backend/target/release/laseask-backend /opt/laseask/laseask-backend
sudo rm -rf /opt/laseask/frontend
sudo cp -r ~/LASEASK/frontend /opt/laseask/frontend
sudo chown -R laseask:laseask /opt/laseask
sudo systemctl start laseask
```

- The service is stopped first: Linux refuses to overwrite a running program (`Text file busy`).
- **`config.toml` is not touched**: it stays in `/opt/laseask`.
- `chown` gives the files back to the `laseask` user. Without it, the service can fail with
  `Permission denied`.
- If `deploy/laseask.service` itself changed in the update (rare), also run:
  `sudo cp ~/LASEASK/deploy/laseask.service /etc/systemd/system/ && sudo systemctl daemon-reload`

### Step 7: Check it works

```bash
sudo systemctl status laseask
sudo journalctl -u laseask -f
```

Same checks as in section 4, step 9. In the browser, press **Ctrl+F5** to bypass the cache.

### Rollback: the new version is broken

```bash
sudo systemctl stop laseask
sudo cp /opt/laseask/laseask-backend.previous /opt/laseask/laseask-backend
sudo rm -rf /opt/laseask/frontend
sudo cp -r /opt/laseask/frontend.previous /opt/laseask/frontend
sudo chown -R laseask:laseask /opt/laseask
sudo systemctl start laseask
```

### Common errors (bare metal)

| Message | Cause | Fix |
|---|---|---|
| `cargo: command not found` | Rust not installed, or the shell doesn't know it yet | Section 4 step 2, or `source "$HOME/.cargo/env"` |
| `linker 'cc' not found` | C build tools missing | `sudo apt-get install -y build-essential` |
| `Text file busy` on `cp` | The service is still running | `sudo systemctl stop laseask`, then copy again |
| `Permission denied` in the logs | Files not owned by the `laseask` user | `sudo chown -R laseask:laseask /opt/laseask`, restart |
| `exec format error` / `status=203/EXEC` | A program compiled on Windows was copied | Compile on the server (step 4) |
| `could not read config file` in the logs | `/opt/laseask/config.toml` missing | Section 4, step 7 |
| `Address already in use` | Something else uses port 8787 (e.g. an old Docker container) | `sudo ss -ltnp \| grep 8787`, stop it |
| `mistral error 400 ... maximum context length` in the chat | Conversation too long for the model | Start a new chat, or ask for fewer/narrower tool calls |

### Cheat sheet (once you're used to it)

```bash
cd ~/LASEASK && git pull
cd backend && cargo build --release && cd ..
sudo cp /opt/laseask/laseask-backend /opt/laseask/laseask-backend.previous
sudo systemctl stop laseask
sudo cp backend/target/release/laseask-backend /opt/laseask/laseask-backend
sudo rm -rf /opt/laseask/frontend && sudo cp -r frontend /opt/laseask/frontend
sudo chown -R laseask:laseask /opt/laseask
sudo systemctl start laseask
sudo journalctl -u laseask -f
```

---

## 6. HTTPS with a real hostname (optional)

Want `https://laseask.yourdomain.com` instead of `http://<server-ip>:8787`? `deploy/tls/` requests a
Let's Encrypt certificate with a DNS-01 challenge (no inbound port 80/443 needed), using your DNS
provider's API (OVH, Cloudflare, Route53, DigitalOcean). Full walkthrough: `deploy/tls/README.md`.

```bash
chmod +x deploy/tls/*.sh
deploy/tls/install-certbot.sh ovh
# fill in deploy/tls/credentials.ovh.example.ini -> /etc/letsencrypt/ovh.ini
deploy/tls/request-cert.sh --provider ovh --email you@example.com \
  --credentials /etc/letsencrypt/ovh.ini laseask.example.com
```

Then set `[tls] enabled = true` in the config file (the script prints the exact `cert_path` and
`key_path`) and restart once. Renewals are automatic after that. The tooling assumes the
bare-metal layout (`/opt/laseask`); with Docker, see the note in `deploy/tls/README.md`.

---

## 7. Day-to-day use

- **Pick the AI**: click Claude, GPT or Mistral in the sidebar. The model box lets you type another
  model name; the app remembers it per AI.
- **Pick the tools**: tick or untick the MCP servers under **Tools (MCP)**. Untick them all for a
  plain chat.
- **New chat**: the **+ New chat** button. Conversations are stored in your browser only (not on
  the server), per browser and per machine.
- **Stop a reply**: the send button turns into a stop button while the AI answers.
- **Add an AI, rotate a key, add an MCP server**: edit the config file, then restart
  (see the table in "Which guide do I follow?").
