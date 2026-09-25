# LASEASK

A personal multi-provider LLM chat interface:

- **`backend/`** — a Rust (Axum) web service that serves a small chat UI and proxies chat requests
  to whichever LLM provider you pick (Anthropic, OpenAI, ...), streaming the reply back live
- **`frontend/`** — the plain HTML/CSS/JS chat UI, served by the backend (no build step)
- **`deploy/`** — systemd unit + Docker option for running the backend on Ubuntu

You bring your own API key(s), one per provider, entered once in the server's config file. There is
**no login system** — the backend is meant to sit behind your firewall, reachable only to a handful
of trusted IPs, and access control is handled there instead.

Optionally, the backend can also connect to **MCP servers** (e.g. a FortiAnalyzer MCP endpoint) and
give Claude tool access to them, the same way tools work in Claude Code — see step 3b.

This guide walks through everything from a clean machine to a working chat: installing tools,
configuring keys, running the backend and deploying it. You then use it from any browser.

---

## 1. Install prerequisites

### On your dev machine (Windows, for building/testing)

- **Rust** — https://rustup.rs, or:
  ```powershell
  winget install Rustlang.Rustup
  ```
  Then restart your terminal and confirm: `cargo --version`

- **Git** — you're already using GitHub Desktop, which is enough for cloning/pushing. Command-line
  `git` isn't required for anything in this guide.

### On the Ubuntu server (for deployment)

Either:
- **Rust** (to build the binary directly on the server), via https://rustup.rs, or
- **Docker**, if you'd rather build/run as a container (see step 5, Option B)

---

## 2. Get the code

Clone/pull the repo with GitHub Desktop as usual:

```
https://github.com/<your-org-or-user>/LASEASK
```

Everything below assumes your local checkout path, e.g.
`C:\Users\<you>\...\GitHub\LASEASK`.

---

## 3. Configure the backend

Copy the example config and fill in your real API key(s):

```powershell
cd backend
Copy-Item config.example.toml config.toml
```

Edit `backend/config.toml`:

```toml
default_provider = "anthropic"

[server]
host = "0.0.0.0"
port = 8787
static_dir = "frontend"     # relative to where the binary runs — see step 5 for deployment layout

[providers.anthropic]
api_key = "sk-ant-your-real-key"
default_model = "claude-sonnet-4-5"

[providers.openai]
api_key = "sk-your-real-key"
default_model = "gpt-4.1"

[providers.mistral]
api_key = "your-real-mistral-key"
default_model = "mistral-large-latest"
```

Notes:
- You only need a `[providers.X]` section for providers you actually plan to use — delete the ones
  you don't have a key for. Just make sure `default_provider` points at one that exists.
- Get an Anthropic key at https://console.anthropic.com (Settings → API Keys) and an OpenAI key at
  https://platform.openai.com/api-keys, and a Mistral key at https://console.mistral.ai/api-keys
  (other model names, e.g. `mistral-small-latest` or `codestral-latest`, can be typed in the UI's
  model box). Billing for these is separate from any Claude.ai/ChatGPT
  subscription — this app calls the pay-per-token API, not a consumer subscription.
- `backend/config.toml` is gitignored on purpose. **Never commit it** — it holds your real keys.

---

## 3b. Connect an MCP server (optional — tool use)

If you want the model (Claude, GPT or Mistral) to be able to call tools — e.g. your FortiAnalyzer MCP server, the same one
Claude Code itself uses — add an `[mcp.<name>]` block per server:

```toml
[mcp.fortianalyzer]
url = "https://your-mcp-server/mcp"
bearer_token = "your-bearer-token"
```

- The key you pick (`fortianalyzer` above) becomes the prefix Claude sees on every tool from that
  server, e.g. a `get_alerts` tool shows up as `fortianalyzer__get_alerts`. You can add as many
  `[mcp.X]` blocks as you have servers.
- MCP tools are offered to every tool-capable provider: `anthropic` (Claude's tool-use format,
  `backend/src/agent.rs`) and `openai` / `mistral` (the OpenAI-compatible function-calling format,
  `backend/src/agent_openai.rs`). Make sure the model you pick supports function calling — e.g.
  `mistral-large-latest`, `mistral-medium-latest`, `mistral-small-latest`.
- On startup the backend connects to each configured server, lists its tools, and logs how many it
  found (`MCP 'fortianalyzer': connected, N tool(s) available`). A server that's unreachable is
  logged as a warning and skipped — it won't stop the rest of the app from starting.
- In the UI's sidebar, **Tools (MCP)** lists every configured server with a checkbox, a status
  dot (green = connected) and its tool count. Only the ticked servers' tools are offered to the
  AI for your next message (the request's `mcp_servers` field); untick them all for a plain chat.
  A server that failed to connect is shown greyed out as "offline". The list comes from
  `GET /api/providers`' `mcp_servers` field.
- When the model uses a tool mid-answer, a small "🔧 fortianalyzer__X" chip appears above the
  reply, and the answer keeps streaming once the tool result comes back.
- The **"MCP only"** switch forces the model to actually call a tool before answering, instead of
  possibly answering from its own knowledge — useful when you specifically want a grounded,
  tool-backed answer. It's only enabled when the selected AI is Claude, GPT or Mistral and at
  least one ticked server has tools; the backend rejects it under any other condition.
- Where to find your existing FortiAnalyzer MCP URL/token if you already use it in Claude Code:
  it's in `~/.claude.json`, under your project's `mcpServers.fortianalyzer` entry (`url` and the
  `Authorization: Bearer ...` header). Copy those two values into `config.toml` — don't paste the
  token anywhere that gets committed.

---

## 4. Run it locally (sanity check before deploying)

```powershell
cd backend
cargo run --release
```

You should see a log line like:

```
LASEASK backend listening on http://0.0.0.0:8787 (static: frontend)
```

Open http://localhost:8787 in a browser — you should get the chat UI, with your configured
provider(s) as buttons in the sidebar's **AI** section. Send a message and confirm you get a streamed reply.

If it fails to start, the most common cause is `config.toml` missing or malformed — the error
message will say which file it looked for and why.

---

## 5. Deploy the backend on your Ubuntu server

Full details, including firewall lockdown, are in **`deploy/README.md`**. Short version:

### Option A — plain systemd service

```bash
# on the server
sudo useradd --system --home /opt/laseask --shell /usr/sbin/nologin laseask
sudo mkdir -p /opt/laseask
```

> **Warning:** a binary compiled on Windows is a Windows `.exe` and will not run on Ubuntu. Either
> build on the server itself (the simplest: install Rust and clone the repo as in section 8,
> step 3, run `cargo build --release` in `backend/`, then use `cp` on the server instead of `scp`),
> or cross-compile for `x86_64-unknown-linux-gnu`. The `scp` commands below assume you already
> have a **Linux** build.

```powershell
# from your dev machine, with a Linux build of the binary
scp backend\target\release\laseask-backend  server:/opt/laseask/
scp -r frontend                             server:/opt/laseask/frontend
scp backend\config.toml                     server:/opt/laseask/config.toml
```

```bash
# on the server
sudo cp deploy/laseask.service /etc/systemd/system/laseask.service
sudo chown -R laseask:laseask /opt/laseask
sudo systemctl daemon-reload
sudo systemctl enable --now laseask
sudo systemctl status laseask

# lock the port down to trusted hosts only
sudo ufw allow from <your-desktop-ip> to any port 8787 proto tcp
```

### Option B — Docker, built straight from the GitHub repo

This is the self-contained path: start from a bare Ubuntu server with nothing on it, end with the
app running in Docker, having built directly from `github.com/Damienvda/LASEA-ASK` — no scp'ing a
compiled binary over from your dev machine. Rust itself is never installed on the server: the
Dockerfile's build stage pulls a `rust:1-slim` image and compiles the backend *inside* that
throwaway build container, so `docker build` is the only "compiler install" step there is.

**1. Install Docker, with BuildKit** (if it's not already on the server). The Dockerfile uses
BuildKit cache mounts to make rebuilds fast, so this isn't optional — plain `docker build` (the
old, non-BuildKit engine) can't build it at all:

```bash
curl -fsSL https://get.docker.com | sudo sh
sudo apt-get update && sudo apt-get install -y docker-buildx-plugin
sudo usermod -aG docker "$USER"
# log out and back in (or `newgrp docker`) for the group change to take effect
```

With `docker-buildx-plugin` installed, plain `docker build` automatically runs on BuildKit under
the hood — no different command, no deprecation warning, and it's what makes the cache-mount
speedup in the Dockerfile work.

**2. Install git and clone the repo:**

```bash
sudo apt-get update && sudo apt-get install -y git
git clone https://github.com/Damienvda/LASEA-ASK.git LASEASK
cd LASEASK
```

**3. Create the real config** (this file is gitignored — it doesn't come from the clone):

```bash
cp backend/config.example.toml backend/config.toml
nano backend/config.toml     # fill in your provider API key(s), and [mcp.*] / [tls] if you use them
```

See step 3 and 3b above for what goes in it.

`.dockerignore` at the repo root keeps `backend/config.toml` out of the build entirely, so even
though it now sits right next to the Dockerfile, it never ends up baked into the image — it's only
ever supplied at container-run time via the `-v` mount in step 5.

**4. Build the image** — this is the step that compiles Rust, entirely inside Docker:

```bash
docker build -f backend/Dockerfile -t laseask-backend .
```

**5. Run it:**

```bash
docker run -d --name laseask -p 8787:8787 \
  -v "$(pwd)/backend/config.toml:/app/config.toml:ro" \
  --restart unless-stopped \
  laseask-backend
```

**6. Verify and lock it down:**

```bash
docker logs -f laseask                 # confirm it started, and check the MCP connection log line
sudo ufw allow from <your-desktop-ip> to any port 8787 proto tcp
```

**Updating later**: see **section 7** for the full step-by-step guide (push from your PC, pull on
the server, rebuild, swap the container, check it, roll back if needed).

Either option, you should end up able to open `http://<server-ip>:8787` from your desktop and get
the same chat UI as the local test in step 4. For HTTPS with Docker, see the note in
`deploy/README.md`'s HTTPS section — the cert-request tooling assumes the systemd layout by
default and needs a small path adjustment for Docker.

---

## 5b. HTTPS with a real hostname (optional)

Want `https://laseask.yourdomain.com` instead of a bare IP over plain HTTP? `deploy/tls/` requests
a Let's Encrypt certificate via a DNS-01 challenge — no inbound port 80/443 needed, so it fits the
firewall-restricted setup above — using your DNS provider's API (OVH out of the box, plus
Cloudflare/Route53/DigitalOcean; more can be added). Full walkthrough: `deploy/tls/README.md`.
Short version:

```bash
chmod +x deploy/tls/*.sh
deploy/tls/install-certbot.sh ovh
# fill in deploy/tls/credentials.ovh.example.ini -> /etc/letsencrypt/ovh.ini
deploy/tls/request-cert.sh --provider ovh --email you@lasea.com \
  --credentials /etc/letsencrypt/ovh.ini laseask.lasea.com
```

Then set `[tls] enabled = true` in `config.toml` (the script prints the exact `cert_path`/
`key_path`) and restart the backend once. Renewals after that are automatic and self-updating —
nothing more to run.

---

## 6. Day-to-day use

- **Add a provider later**: add a `[providers.X]` block to `config.toml` on the server, restart the
  service (`sudo systemctl restart laseask` or `docker restart laseask`) — it'll show up as a new
  button in the sidebar's **AI** section automatically.
- **Rotate a key**: same — edit `config.toml`, restart.
- **Add/remove an MCP server**: add, edit, or delete an `[mcp.X]` block, restart the service —
  check the startup logs (`journalctl -u laseask -f` or `docker logs -f laseask`) to confirm it
  connected and see how many tools it found.
- **Update the app**: Docker → follow **section 7**. Bare metal / systemd → follow **section 8**.
- **Conversation history** lives in the browser's `localStorage`, per device/browser — it is not
  stored server-side.

---

## Updating the app: which guide?

Two separate step-by-step guides, depending on how the server runs the app. Follow **only one**.

| The server runs LASEASK with... | How to tell | Guide |
|---|---|---|
| **Docker** (step 5, Option B) | `docker ps` shows a `laseask` line | **Section 7** |
| **Bare metal / systemd** (step 5, Option A) | `systemctl status laseask` shows `active (running)` | **Section 8** |

---

## 7. Updating the app: Docker (beginner guide)

Use this every time the code changed (a fix, a new feature) and you want the server to run the new
version. It takes about 5 minutes. You don't need to know Rust or Docker: just copy the commands one
at a time, in order, and check the result after each step.

**Only the config changed** (new API key, new model, new `[providers.X]` or `[mcp.X]` block)? You
don't need this guide: edit `backend/config.toml` on the server, then run `docker restart laseask`.
The config file is read from the server's disk at start-up, not built into the image.

### The big picture

```
 Your PC                    GitHub                     Ubuntu server
 (edit code)  -- push -->   (stores the code)  -- pull -->  (build image, run container)
```

- **Image** (`laseask-backend`): the compiled app, like an installer. Built by `docker build`.
- **Container** (`laseask`): a running copy of the image. Started by `docker run`.
- Rebuilding the image does **not** change the running container. You have to stop the old
  container and start a new one from the new image (steps 5 and 6).

### Step 1: Send your changes to GitHub (on your PC)

1. Open **GitHub Desktop** and select the `LASEASK` repository.
2. On the left, check the list of changed files. **`backend/config.toml` must never appear here.**
   It holds your API keys and is gitignored, so it normally doesn't.
3. Bottom left: type a short summary (e.g. `Cap MCP tool result size`), click **Commit to main**.
4. Top: click **Push origin**. Wait until the button goes back to **Fetch origin**.

If someone else pushed the change, skip this step.

### Step 2: Connect to the server

From a terminal on your PC (PowerShell works):

```bash
ssh <your-user>@<server-ip>
```

Type the password when asked (nothing shows while you type, that's normal). The prompt now shows
something like `<your-user>@<server>:~$`: every command below runs **on the server**.

### Step 3: Go to the project folder

```bash
cd ~/LASEASK
pwd
```

`pwd` must print `/home/<your-user>/LASEASK`. **Always run the next steps from this folder**: the
build and the config mount both depend on it.

### Step 4: Download the new code

```bash
git status
git pull
```

- `git status` should say `nothing to commit, working tree clean` (or only list untracked files).
- `git pull` prints the files that changed, or `Already up to date.` If it says `Already up to
  date` but you expected changes, you probably forgot to **Push** in step 1.

**Error: `Your local changes to the following files would be overwritten by merge`**: someone
edited a file directly on the server. To throw those server-side edits away and take GitHub's
version (your `config.toml` is gitignored, so it's not affected):

```bash
git stash
git pull
```

### Step 5: Build the new image (keep a backup of the old one first)

```bash
docker tag laseask-backend laseask-backend:previous
docker build -f backend/Dockerfile -t laseask-backend .
```

- The first line keeps the current image under the name `laseask-backend:previous`, so you can go
  back to it if the new version is broken (see "Rollback" below).
- **Don't forget the final `.`** in the build command. It means "the current folder", which must
  be the repo root (step 3).
- The build takes from about 30 seconds to several minutes (the Rust compile). It succeeded if it
  ends with `naming to docker.io/library/laseask-backend` and no red `ERROR`.
- **If the build fails, stop here.** The old container is still running and nothing is broken.
  Copy the error and fix the code first.

### Step 6: Replace the running container

```bash
docker stop laseask
docker rm laseask
docker run -d --name laseask -p 8787:8787 \
  -v "$HOME/LASEASK/backend/config.toml:/app/config.toml:ro" \
  --restart unless-stopped \
  laseask-backend
```

- `stop` + `rm` delete the old container (not the image, not your config).
- `docker run` starts a new container from the image you just built. It prints a long ID: that's
  success. The site is down for just a few seconds between `stop` and `run`.
- Paste the `docker run` command as a whole: the `\` at the end of each line means "the command
  continues on the next line".

### Step 7: Check it works

```bash
docker ps
docker logs -f laseask
```

- `docker ps` must show a line for `laseask` with a status like `Up 10 seconds`. If the status is
  `Restarting`, the app crashes at start-up: read the logs.
- In the logs, look for:
  - `LASEASK backend listening on http://0.0.0.0:8787`: the app started.
  - `MCP 'fortianalyzer': connected, N tool(s) available`: the FortiAnalyzer tools are available.
- Press **Ctrl+C** to stop reading the logs. This does **not** stop the app.
- Open `http://<server-ip>:8787` in your browser (**Ctrl+F5** to bypass the browser cache) and send
  a test message.

### Step 8 (optional): Clean up

Old images pile up and use disk space. Once the new version works:

```bash
docker image prune -f
```

This removes unused, untagged images only. It keeps `laseask-backend` and `laseask-backend:previous`.

### Rollback: the new version is broken

Go back to the image saved in step 5:

```bash
docker stop laseask
docker rm laseask
docker run -d --name laseask -p 8787:8787 \
  -v "$HOME/LASEASK/backend/config.toml:/app/config.toml:ro" \
  --restart unless-stopped \
  laseask-backend:previous
```

Only the last `:previous` changes compared to step 6.

### Common errors

| Message | Cause | Fix |
|---|---|---|
| `The container name "/laseask" is already in use` | The old container still exists | `docker stop laseask && docker rm laseask`, then `docker run` again |
| `"/frontend": not found` during the build | Build run from the wrong folder (e.g. from `backend/`) | `cd ~/LASEASK`, then build again |
| `docker build requires 1 argument` | The final `.` is missing | Add ` .` at the end of the build command |
| `permission denied ... docker.sock` | Your user isn't in the `docker` group | `sudo usermod -aG docker $USER`, log out and back in |
| `Bind for 0.0.0.0:8787 failed: port is already allocated` | Another container or process uses port 8787 | `docker ps` to find it, stop it |
| `could not read config file` in the logs | `config.toml` missing, or wrong path in `-v` | `ls ~/LASEASK/backend/config.toml` must list the file |
| `default_provider 'X' has no matching [providers.X] section` | Config error | Fix `config.toml`, then `docker restart laseask` |
| `mistral error 400 ... maximum context length` in the chat | The conversation (tool results included) is too long for the model | Start a new conversation, or ask for fewer/narrower tool calls |

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

## 8. Updating the app: bare metal / systemd (beginner guide)

Use this if the server runs LASEASK as a **systemd service** (step 5, Option A), not in Docker. It
takes about 5-10 minutes. Copy the commands one at a time, in order, and check the result after
each step.

**Only the config changed** (new API key, new model, new `[providers.X]` or `[mcp.X]` block)? You
don't need this guide: edit `/opt/laseask/config.toml`, then run `sudo systemctl restart laseask`.

### The big picture

```
 Your PC                   GitHub                    Ubuntu server
 (edit code) -- push -->   (stores the code) -- pull -->  ~/LASEASK   (source code, build here)
                                                            | copy binary + frontend
                                                            v
                                                          /opt/laseask (what systemd runs)
```

Two folders on the server, don't mix them up:

- **`~/LASEASK`**: the git clone. You pull and build here.
- **`/opt/laseask`**: what actually runs. It contains `laseask-backend` (the program),
  `frontend/` (the web page) and `config.toml` (your keys). systemd starts the program from here.

**Always build on the server.** A `cargo build` on Windows produces a Windows `.exe`, which can't
run on Ubuntu.

### Step 1: Send your changes to GitHub (on your PC)

1. Open **GitHub Desktop** and select the `LASEASK` repository.
2. On the left, check the list of changed files. **`backend/config.toml` must never appear here.**
3. Bottom left: type a short summary, click **Commit to main**.
4. Top: click **Push origin**. Wait until the button goes back to **Fetch origin**.

If someone else pushed the change, skip this step.

### Step 2: Connect to the server

```bash
ssh <your-user>@<server-ip>
```

Every command below runs **on the server**.

### Step 3 (first time only): Install the build tools and the source code

Skip this step if `cargo --version` prints a version and `~/LASEASK` exists.

```bash
sudo apt-get update && sudo apt-get install -y build-essential git curl
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source "$HOME/.cargo/env"
cargo --version
cd ~
git clone https://github.com/Damienvda/LASEA-ASK.git LASEASK
```

- `build-essential` provides the C linker Rust needs. No OpenSSL is needed (the app uses rustls).
- `cargo --version` must print something like `cargo 1.xx.x`. If it says `command not found`, log
  out and back in, then try again.

### Step 4: Download the new code

```bash
cd ~/LASEASK
git status
git pull
```

- `git status` should say `nothing to commit, working tree clean`.
- `git pull` lists the changed files, or `Already up to date.` (did you forget **Push** in step 1?).
- **Error `Your local changes ... would be overwritten by merge`**: run `git stash`, then
  `git pull` again. This throws away edits made directly in the server's clone.

### Step 5: Compile

```bash
cd ~/LASEASK/backend
cargo build --release
```

- The first build takes several minutes. Later builds only recompile what changed.
- It succeeded if it ends with `Finished release [optimized] target(s)` (or
  `Finished `release` profile`) and no red `error`.
- The new program is now at `backend/target/release/laseask-backend`.
- **If the build fails, stop here.** The service is still running the old version and nothing is
  broken. Copy the error and fix the code first.

### Step 6: Back up the current version

```bash
sudo cp /opt/laseask/laseask-backend /opt/laseask/laseask-backend.previous
sudo rm -rf /opt/laseask/frontend.previous
sudo cp -r /opt/laseask/frontend /opt/laseask/frontend.previous
```

This keeps the running version so you can go back to it (see "Rollback" below).

### Step 7: Install the new version and restart

```bash
sudo systemctl stop laseask
sudo cp ~/LASEASK/backend/target/release/laseask-backend /opt/laseask/laseask-backend
sudo rm -rf /opt/laseask/frontend
sudo cp -r ~/LASEASK/frontend /opt/laseask/frontend
sudo chown -R laseask:laseask /opt/laseask
sudo systemctl start laseask
```

- The service is stopped first: Linux refuses to overwrite a program that's running (`Text file
  busy`).
- **`config.toml` is not touched**: it stays in `/opt/laseask`.
- `chown` gives the files back to the `laseask` user that the service runs as. Without it, the
  service can fail with `Permission denied`.
- If `deploy/laseask.service` itself changed in the update (rare), also run:
  `sudo cp ~/LASEASK/deploy/laseask.service /etc/systemd/system/ && sudo systemctl daemon-reload`

### Step 8: Check it works

```bash
sudo systemctl status laseask
sudo journalctl -u laseask -f
```

- `status` must show **`active (running)`** in green. Press **q** to leave that screen.
- In the logs, look for:
  - `LASEASK backend listening on http://0.0.0.0:8787`: the app started.
  - `MCP 'fortianalyzer': connected, N tool(s) available`: the FortiAnalyzer tools are available.
- Press **Ctrl+C** to stop reading the logs. This does **not** stop the app.
- Open `http://<server-ip>:8787` in your browser (**Ctrl+F5** to bypass the cache) and send a test
  message.

### Rollback: the new version is broken

Put back the version saved in step 6:

```bash
sudo systemctl stop laseask
sudo cp /opt/laseask/laseask-backend.previous /opt/laseask/laseask-backend
sudo rm -rf /opt/laseask/frontend
sudo cp -r /opt/laseask/frontend.previous /opt/laseask/frontend
sudo chown -R laseask:laseask /opt/laseask
sudo systemctl start laseask
```

### Common errors

| Message | Cause | Fix |
|---|---|---|
| `cargo: command not found` | Rust not installed, or the shell doesn't know it yet | Step 3, or `source "$HOME/.cargo/env"` |
| `linker 'cc' not found` | C build tools missing | `sudo apt-get install -y build-essential` |
| `Text file busy` on `cp` | The service is still running | `sudo systemctl stop laseask`, then copy again |
| `Permission denied` in the logs | Files not owned by the `laseask` user | `sudo chown -R laseask:laseask /opt/laseask`, restart |
| `exec format error` / `status=203/EXEC` | A Windows-built binary was copied | Build on the server (step 5) |
| `could not read config file` in the logs | `/opt/laseask/config.toml` missing | `ls -l /opt/laseask/config.toml`; copy `backend/config.example.toml` there and fill it in |
| `Address already in use` | Something else uses port 8787 (e.g. an old Docker container) | `docker ps` / `sudo ss -ltnp \| grep 8787`, stop it |
| `mistral error 400 ... maximum context length` in the chat | Conversation too long for the model | Start a new conversation, or ask for fewer/narrower tool calls |

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
