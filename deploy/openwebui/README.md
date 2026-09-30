# LASEASK = Open WebUI + LASEASK's tools (beginner guide)

The tool is **[Open WebUI](https://github.com/open-webui/open-webui)**. LASEASK adds to it what
Open WebUI doesn't have on its own:

| Added by LASEASK | How it plugs into Open WebUI |
|---|---|
| The **FortiAnalyzer tools** (and any other `[mcp.*]` server), with the dangerous ones removed (`exclude_tools`), a shorter list if wanted (`include_tools`), and **every result compacted and capped** so a big log pull doesn't eat the context | MCP tool server **LASEASK** (Admin Panel > Settings > External Tools) |
| **Threat intelligence** on public IPs and domains (RDAP, AbuseIPDB, VirusTotal, OTX), cached, private addresses never sent out | The `intel__lookup` tool of the same server, and the **🛡 Réputation** button under answers |
| The **investigation prompt**: current date, rules to investigate thoroughly (explicit time windows, aggregate first, find them all, check both directions, verify) and to answer briefly from tool results only, plus the per-server notes (FortiAnalyzer field quirks) | Function **LASEASK · Investigation** |
| **MCP only**: the AI must consult a tool before answering | Button **LASEASK · MCP uniquement** in the message bar |

Everything else is Open WebUI's own: models (Claude, Mistral, GPT), accounts, history, knowledge
bases (RAG), notes, channels, statistics, memory, prompts, voice, mobile, the tool calls shown in
each answer. LASEASK itself runs as an internal service with no interface of its own.

---

## Step 1: Prerequisites

1. The repo on the server in `~/LASEASK`, branch `ASK` (main `README.md`, section 2, steps 1-3).
2. Docker Compose: `docker compose version` must print a version. If not:
   ```bash
   sudo apt install -y docker-compose-plugin
   ```
3. A Let's Encrypt certificate for the server's name, made with `deploy/tls/request-cert.sh`
   (main `README.md`, section 6). Check: `sudo ls /etc/letsencrypt/live/` shows your hostname.

## Step 2: Update the code

```bash
cd ~/LASEASK && git pull
```

## Step 3: `config.toml` (the LASEASK tool service)

```bash
cd ~/LASEASK/backend
cp -n config.example.toml config.toml
nano config.toml
```

1. Generate a secret and keep it for step 8: `openssl rand -hex 32`
2. In `[tool_server]`: `enabled = true`, and the secret as `api_key`.
3. `[mcp.fortianalyzer]`: your MCP URL and token (as before). `exclude_tools` keeps the tools that
   change things away from the AI; `include_tools` shortens the list (fewer tokens per question).
4. `[intel]`: the free API keys you have (optional).
5. `[tls]`: `enabled = false` (nginx does HTTPS).
6. `default_provider` and `[providers.*]` are **not needed** any more (the models are set up in Open
   WebUI). Leave them or delete them.

## Step 4: `.env`

```bash
cd ~/LASEASK/deploy/openwebui
cp .env.example .env
nano .env
```

| Line | Value |
|---|---|
| `LASEASK_HOSTNAME` | The name users type, the one the certificate is for |
| `ANTHROPIC_API_KEY` | Your Claude API key |
| `WEBUI_SECRET_KEY` | A new secret: `openssl rand -hex 32` |
| `OPENWEBUI_VERSION` | `main`, or better a tested tag (at least v0.6.31, for MCP) |

## Step 5: Stop the old container (if there is one)

```bash
docker stop laseask && docker rm laseask
```

## Step 6: Start

```bash
cd ~/LASEASK/deploy/openwebui
docker compose up -d --build
docker compose ps
docker compose logs laseask | grep -E "MCP"
```

The three services must be `running`, and the log must show
`MCP 'fortianalyzer': connected, ...` and `MCP tool server enabled on /mcp: ... tool(s)`.

## Step 7: Admin account

Open `https://<your hostname>` and create an account: **the first one is the administrator.**
Colleagues create theirs; they stay *pending* until approved in **Admin Panel > Users**.

## Step 8: Connect the LASEASK tools

**Admin Panel > Settings > External Tools > +**:

| Field | Value |
|---|---|
| Type | **MCP (Streamable HTTP)** |
| URL | `http://laseask:8787/mcp` |
| Auth | **Bearer**, the `[tool_server] api_key` from step 3 |
| Name / ID | `LASEASK` |

Save: the connection turns green and lists the tools (`fortianalyzer__...`, `intel__lookup`).

## Step 9: The models

**Admin Panel > Settings > Connections**: the Anthropic connection (`https://api.anthropic.com/v1`)
is there. Add **Mistral** (`https://api.mistral.ai/v1`) or **OpenAI** (`https://api.openai.com/v1`)
with their keys if you use them. If a connection lists no models, add their ids by hand in the
connection's *Model IDs* (e.g. `claude-opus-5-5`, `claude-haiku-4-5-20251001`).

Then for each model you want to investigate with, **Workspace > Models > the model > edit**:

- **Tools**: tick **LASEASK**, so it is on by default in new chats.
- **Advanced Params > Function Calling**: **Native**.
- Optional: a clear name ("Claude Opus – investigation") and a description.

**Admin Panel > Settings > Interface**: *Task model (external)* = `claude-haiku-4-5-20251001`.

## Step 10: The LASEASK functions

For each file in `deploy/openwebui/functions/` (see a file with `cat` on the server):
**Admin Panel > Functions > +**, paste it, **Save**, switch it **on**, then **⋯ > Global**.

| File | Adds |
|---|---|
| `laseask_investigation.py` | The investigation prompt on every chat (valve `model_ids` to limit it to some models) |
| `laseask_mcp_only.py` | The **MCP uniquement** button |
| `laseask_reputation.py` | The **🛡** button under answers |

## Step 11: Firewall and certificate renewal

- Open port **443** to the users (port 80 only redirects). Nothing else is published. Do it on the
  network firewall: `ufw` doesn't filter Docker's ports.
- Let nginx reload renewed certificates (replace `YOUR-USER`):
  ```bash
  sudo tee /etc/letsencrypt/renewal-hooks/deploy/laseask-nginx.sh >/dev/null <<'EOF'
  #!/bin/sh
  cd /home/YOUR-USER/LASEASK/deploy/openwebui && docker compose exec -T nginx nginx -s reload
  EOF
  sudo chmod +x /etc/letsencrypt/renewal-hooks/deploy/laseask-nginx.sh
  ```

---

## Costs: read this

Claude is reached through **Anthropic's OpenAI-compatible API**, which Open WebUI speaks. That API
**does not do prompt caching** and **doesn't return Claude's thinking** (Anthropic's documentation).
Every tool call resends the whole conversation, tool descriptions included (the 73 FortiAnalyzer
tools are roughly 30k tokens), at full price. To keep costs down:

- shorten the tool list with `include_tools` in `[mcp.fortianalyzer]` (the example list in
  `config.example.toml` keeps the 17 most useful ones);
- keep `max_tool_result_chars` at 20000 or less;
- use Haiku for simple questions.

## Updating

```bash
cd ~/LASEASK && git pull
cd deploy/openwebui
docker compose build laseask
docker compose pull open-webui
docker compose up -d
```

If a file in `functions/` changed, paste it again over the existing function. Back up Open WebUI's
data first (accounts, conversations, knowledge bases, functions):

```bash
docker run --rm -v laseask_open-webui:/data -v "$PWD":/backup alpine \
  tar czf /backup/open-webui-$(date +%F).tgz -C /data .
```

## Common errors

| Symptom | Cause | Fix |
|---|---|---|
| The LASEASK connection stays red | Wrong token, or `[tool_server] enabled = false` | Same secret in `config.toml` and the connection; `docker compose logs laseask` |
| `laseask` restarts in a loop | Error in `config.toml` | `docker compose logs laseask`: the first line names the problem |
| The AI never calls the tools | Tools not ticked for the chat/model, or Function Calling not Native | Step 9 |
| "prompt d'investigation indisponible" | The function can't reach LASEASK | Valve `laseask_url` = `http://laseask:8787` |
| `nginx` exits: `cannot load certificate` | `LASEASK_HOSTNAME` doesn't match `/etc/letsencrypt/live/` | Fix `.env`, `docker compose up -d` |
| Answer cut off after 5 minutes | Timeout overridden | `AIOHTTP_CLIENT_TIMEOUT` in `docker-compose.yml`, recreate `open-webui` |
| `MCP ... dns error` in LASEASK's logs | The MCP server's name isn't resolvable inside Docker | `extra_hosts` in `docker-compose.yml` |

## Limits

- The Open WebUI licence requires keeping its name and logo: the title reads
  **LASEASK (Open WebUI)**.
- The older LASEASK modes (its own console in `frontend/`, its own agent, the `/v1` API) are still
  in the code but not used by this install.
