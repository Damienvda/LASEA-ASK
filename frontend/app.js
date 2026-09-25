const appEl = document.getElementById("app");
const messagesEl = document.getElementById("messages");
const emptyState = document.getElementById("empty-state");
const emptySub = document.getElementById("empty-sub");
const providerButtons = document.getElementById("provider-buttons");
const modelInput = document.getElementById("model-input");
const mcpList = document.getElementById("mcp-list");
const mcpHint = document.getElementById("mcp-hint");
const mcpOnlyToggle = document.getElementById("mcp-only-toggle");
const newChatBtn = document.getElementById("new-chat-btn");
const input = document.getElementById("input");
const sendBtn = document.getElementById("send-btn");
const composerMeta = document.getElementById("composer-meta");
const mobileTitle = document.getElementById("mobile-title");
const backdrop = document.getElementById("backdrop");

const HISTORY_KEY = "laseask.history";
const PREFS_KEY = "laseask.prefs";

/** Providers with a tool-use loop in the backend (agent.rs / agent_openai.rs). */
const TOOL_PROVIDERS = ["anthropic", "openai", "mistral"];

/** How each backend provider name is shown. Unknown providers fall back to their raw name. */
const PROVIDER_META = {
  anthropic: { label: "Claude", initial: "C" },
  openai: { label: "GPT", initial: "G" },
  mistral: { label: "Mistral", initial: "M" },
};

/**
 * One entry per message. Assistant entries also remember which AI/model answered and which
 * tools it used, so the thread still shows that after a reload. Only role/content go to the API.
 * @type {{role: string, content: string, provider?: string, model?: string, tools?: string[]}[]}
 */
let history = load(HISTORY_KEY, []);
/** @type {{provider?: string, models: Record<string, string>, mcp: Record<string, boolean>, mcpOnly: boolean}} */
let prefs = { models: {}, mcp: {}, mcpOnly: false, ...load(PREFS_KEY, {}) };
let providers = [];
let mcpServers = [];
let defaultProvider = "";
let abortController = null;

const thread = document.createElement("div");
thread.className = "thread";
messagesEl.appendChild(thread);

// ---------- storage ----------

function load(key, fallback) {
  try {
    const raw = localStorage.getItem(key);
    return raw ? JSON.parse(raw) : fallback;
  } catch {
    return fallback;
  }
}

function save(key, value) {
  try {
    localStorage.setItem(key, JSON.stringify(value));
  } catch {
    // storage unavailable (private window, blocked site data): the app still works, it just forgets
  }
}

// ---------- providers ----------

function providerMeta(name) {
  return PROVIDER_META[name] || { label: name, initial: (name[0] || "?").toUpperCase() };
}

function currentProvider() {
  return providers.find((p) => p.name === prefs.provider);
}

function providerDot(name) {
  const dot = document.createElement("span");
  dot.className = "provider-dot";
  dot.textContent = providerMeta(name).initial;
  return dot;
}

function renderProviders() {
  providerButtons.innerHTML = "";
  for (const p of providers) {
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = `provider-btn p-${p.name}`;
    btn.setAttribute("role", "radio");
    btn.setAttribute("aria-checked", String(p.name === prefs.provider));
    btn.dataset.provider = p.name;

    const text = document.createElement("span");
    text.className = "provider-text";
    const name = document.createElement("span");
    name.className = "provider-name";
    name.textContent = providerMeta(p.name).label;
    const model = document.createElement("span");
    model.className = "provider-model";
    model.textContent = prefs.models[p.name] || p.default_model;
    text.append(name, model);

    btn.append(providerDot(p.name), text);
    btn.addEventListener("click", () => selectProvider(p.name));
    providerButtons.appendChild(btn);
  }
}

function selectProvider(name) {
  prefs.provider = name;
  const p = currentProvider();
  modelInput.value = prefs.models[name] || (p ? p.default_model : "");
  save(PREFS_KEY, prefs);
  renderProviders();
  updateToolState();
}

modelInput.addEventListener("input", () => {
  const p = currentProvider();
  if (!p) return;
  const value = modelInput.value.trim();
  // An empty box or the default model = no override, so a new default in config.toml applies.
  if (!value || value === p.default_model) delete prefs.models[p.name];
  else prefs.models[p.name] = value;
  save(PREFS_KEY, prefs);
  const label = providerButtons.querySelector(`[data-provider="${p.name}"] .provider-model`);
  if (label) label.textContent = value || p.default_model;
  updateToolState();
});

// ---------- MCP servers ----------

function usable(server) {
  return server.connected && server.tool_count > 0;
}

/** Servers whose box is ticked. A usable server nobody has touched yet starts ticked. */
function selectedServers() {
  return mcpServers.filter((s) => usable(s) && prefs.mcp[s.name] !== false).map((s) => s.name);
}

function renderMcp() {
  mcpList.innerHTML = "";
  for (const s of mcpServers) {
    const item = document.createElement("label");
    item.className = "mcp-item" + (usable(s) ? "" : " disabled");
    item.title = s.connected
      ? `${s.tool_count} tool${s.tool_count === 1 ? "" : "s"} available`
      : "Not connected: the server was unreachable when the backend started. Check the backend logs.";

    const box = document.createElement("input");
    box.type = "checkbox";
    box.disabled = !usable(s);
    box.checked = usable(s) && prefs.mcp[s.name] !== false;
    box.addEventListener("change", () => {
      prefs.mcp[s.name] = box.checked;
      save(PREFS_KEY, prefs);
      updateToolState();
    });

    const dot = document.createElement("span");
    dot.className = "status-dot" + (s.connected ? " on" : "");
    const name = document.createElement("span");
    name.className = "mcp-name";
    name.textContent = s.name;
    const count = document.createElement("span");
    count.className = "mcp-count";
    count.textContent = s.connected ? `${s.tool_count} tools` : "offline";

    item.append(box, dot, name, count);
    mcpList.appendChild(item);
  }
}

mcpOnlyToggle.addEventListener("change", () => {
  prefs.mcpOnly = mcpOnlyToggle.checked;
  save(PREFS_KEY, prefs);
  updateToolState();
});

/** Keeps the MCP hint, the "MCP only" switch and the status lines in line with the selection. */
function updateToolState() {
  const p = currentProvider();
  const label = p ? providerMeta(p.name).label : "";
  const toolCapable = p && TOOL_PROVIDERS.includes(p.name);
  const selected = selectedServers();

  let hint = "";
  if (mcpServers.length === 0) hint = "No MCP server configured. Add an [mcp.<name>] block in config.toml.";
  else if (!toolCapable) hint = `${label} can't use tools: MCP is ignored with this AI.`;
  mcpHint.textContent = hint;
  mcpHint.hidden = !hint;

  // Same rule the backend enforces for mcp_only (routes.rs chat()).
  const mcpOnlyAvailable = toolCapable && selected.length > 0;
  mcpOnlyToggle.disabled = !mcpOnlyAvailable;
  mcpOnlyToggle.checked = mcpOnlyAvailable && prefs.mcpOnly;

  const model = modelInput.value.trim() || (p ? p.default_model : "");
  const tools = toolCapable && selected.length ? `🔧 ${selected.join(", ")}` : "no tools";
  const only = mcpOnlyToggle.checked ? " · MCP only" : "";
  composerMeta.textContent = p ? `${label} · ${model} · ${tools}${only}` : "";
  mobileTitle.textContent = p ? `${label} · ${model}` : "";
  emptySub.textContent = p ? `You're talking to ${label} (${model}), ${tools === "no tools" ? "without tools" : `with ${tools}`}.` : "";
}

// ---------- rendering ----------

/** Markdown when marked + DOMPurify loaded from the CDN, plain text otherwise. */
function renderBody(el, text) {
  if (window.marked && window.DOMPurify) {
    el.classList.remove("plain");
    el.innerHTML = DOMPurify.sanitize(marked.parse(text, { gfm: true, breaks: true }));
  } else {
    el.classList.add("plain");
    el.textContent = text;
  }
}

function updateEmptyState() {
  emptyState.hidden = history.length > 0 || thread.childElementCount > 0;
}

function scrollToBottom() {
  messagesEl.scrollTop = messagesEl.scrollHeight;
}

function appendUser(text) {
  const div = document.createElement("div");
  div.className = "msg user";
  div.textContent = text;
  thread.appendChild(div);
  updateEmptyState();
  scrollToBottom();
}

/** Builds an assistant reply block and returns handles to fill it in while it streams. */
function appendAssistant(provider, model) {
  const wrap = document.createElement("div");
  wrap.className = "msg assistant";

  const label = document.createElement("div");
  label.className = `msg-label p-${provider}`;
  const name = document.createElement("span");
  name.textContent = providerMeta(provider).label;
  const modelEl = document.createElement("span");
  modelEl.className = "model";
  modelEl.textContent = model || "";
  label.append(providerDot(provider), name, modelEl);

  const tools = document.createElement("div");
  tools.className = "tool-calls";
  tools.hidden = true;

  const body = document.createElement("div");
  body.className = "msg-body";

  const actions = document.createElement("div");
  actions.className = "msg-actions";
  actions.hidden = true;

  wrap.append(label, tools, body, actions);
  thread.appendChild(wrap);
  updateEmptyState();
  scrollToBottom();

  let text = "";
  let frame = 0;

  return {
    wrap,
    get text() {
      return text;
    },
    append(chunk) {
      text += chunk;
      // Re-render at most once per frame: parsing markdown on every token is wasteful.
      if (!frame) {
        frame = requestAnimationFrame(() => {
          frame = 0;
          renderBody(body, text);
          scrollToBottom();
        });
      }
    },
    setText(value) {
      text = value;
      renderBody(body, text);
    },
    addTool(toolName) {
      const chip = document.createElement("span");
      chip.className = "tool-chip";
      chip.textContent = `🔧 ${toolName}`;
      tools.appendChild(chip);
      tools.hidden = false;
      scrollToBottom();
    },
    error(message) {
      const err = document.createElement("div");
      err.className = "msg-error";
      err.textContent = `Error: ${message}`;
      wrap.appendChild(err);
      scrollToBottom();
    },
    finish() {
      if (frame) cancelAnimationFrame(frame);
      frame = 0;
      wrap.classList.remove("streaming");
      if (text) {
        renderBody(body, text);
        actions.hidden = false;
      } else {
        body.hidden = true;
      }
    },
    addCopy() {
      const btn = document.createElement("button");
      btn.type = "button";
      btn.textContent = "Copy";
      btn.addEventListener("click", async () => {
        await copyText(text);
        btn.textContent = "Copied";
        setTimeout(() => (btn.textContent = "Copy"), 1500);
      });
      actions.appendChild(btn);
    },
  };
}

/** Clipboard API needs HTTPS (or localhost); plain-HTTP deployments fall back to execCommand. */
async function copyText(text) {
  try {
    await navigator.clipboard.writeText(text);
  } catch {
    const ta = document.createElement("textarea");
    ta.value = text;
    ta.style.position = "fixed";
    ta.style.opacity = "0";
    document.body.appendChild(ta);
    ta.select();
    document.execCommand("copy");
    ta.remove();
  }
}

function renderHistory() {
  thread.innerHTML = "";
  for (const msg of history) {
    if (msg.role === "user") {
      appendUser(msg.content);
    } else {
      const reply = appendAssistant(msg.provider || prefs.provider || "", msg.model);
      for (const t of msg.tools || []) reply.addTool(t);
      reply.setText(msg.content);
      reply.addCopy();
      reply.finish();
    }
  }
  updateEmptyState();
  scrollToBottom();
}

// ---------- sending ----------

function setSending(sending) {
  sendBtn.classList.toggle("stop", sending);
  sendBtn.textContent = sending ? "■" : "↑";
  sendBtn.setAttribute("aria-label", sending ? "Stop" : "Send");
  updateSendEnabled();
}

function updateSendEnabled() {
  sendBtn.disabled = !abortController && !input.value.trim();
}

async function send() {
  const text = input.value.trim();
  if (!text || abortController || !currentProvider()) return;

  input.value = "";
  input.style.height = "auto";
  history.push({ role: "user", content: text });
  appendUser(text);
  save(HISTORY_KEY, history);

  const provider = prefs.provider;
  const model = modelInput.value.trim() || currentProvider().default_model;
  const toolCapable = TOOL_PROVIDERS.includes(provider);
  const reply = appendAssistant(provider, model);
  reply.wrap.classList.add("streaming");
  const usedTools = [];

  abortController = new AbortController();
  setSending(true);

  try {
    await streamChat(
      {
        provider,
        model,
        messages: history.map(({ role, content }) => ({ role, content })),
        mcp_servers: toolCapable ? selectedServers() : [],
        mcp_only: mcpOnlyToggle.checked,
      },
      abortController.signal,
      (event) => {
        if (event.type === "delta") reply.append(event.text);
        else if (event.type === "tool_call") {
          usedTools.push(event.name);
          reply.addTool(event.name);
        } else if (event.type === "error") reply.error(event.message);
      },
    );
  } catch (err) {
    if (err.name === "AbortError") reply.append(reply.text ? "\n\n*(stopped)*" : "*(stopped)*");
    else reply.error(err.message || String(err));
  }

  reply.finish();
  if (reply.text) {
    reply.addCopy();
    history.push({ role: "assistant", content: reply.text, provider, model, tools: usedTools });
    save(HISTORY_KEY, history);
  }

  abortController = null;
  setSending(false);
  input.focus();
}

/**
 * POSTs to /api/chat and parses the text/event-stream response manually
 * (EventSource can't do POST bodies), invoking onEvent for each normalized
 * StreamEvent the backend emits: {type: "delta"|"tool_call"|"done"|"error", ...}.
 */
async function streamChat(body, signal, onEvent) {
  const res = await fetch("/api/chat", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
    signal,
  });

  if (!res.ok) {
    const data = await res.json().catch(() => ({}));
    throw new Error(data.error || `HTTP ${res.status}`);
  }

  const reader = res.body.getReader();
  const decoder = new TextDecoder();
  let buf = "";

  while (true) {
    const { done, value } = await reader.read();
    if (done) break;
    buf += decoder.decode(value, { stream: true });

    let sep;
    while ((sep = buf.indexOf("\n\n")) !== -1) {
      const rawEvent = buf.slice(0, sep);
      buf = buf.slice(sep + 2);

      const dataLine = rawEvent
        .split("\n")
        .filter((l) => l.startsWith("data:"))
        .map((l) => l.slice(5).trimStart())
        .join("\n");

      if (!dataLine) continue;
      onEvent(JSON.parse(dataLine));
    }
  }
}

// ---------- wiring ----------

sendBtn.addEventListener("click", () => {
  if (abortController) abortController.abort();
  else send();
});

input.addEventListener("input", () => {
  input.style.height = "auto";
  input.style.height = Math.min(input.scrollHeight, 200) + "px";
  updateSendEnabled();
});

input.addEventListener("keydown", (e) => {
  if (e.key === "Enter" && !e.shiftKey && !e.isComposing) {
    e.preventDefault();
    send();
  }
});

newChatBtn.addEventListener("click", () => {
  if (abortController) abortController.abort();
  history = [];
  save(HISTORY_KEY, history);
  renderHistory();
  closeSidebar();
  input.focus();
});

function closeSidebar() {
  appEl.classList.remove("sidebar-open");
  backdrop.hidden = true;
}
document.getElementById("open-sidebar").addEventListener("click", () => {
  appEl.classList.add("sidebar-open");
  backdrop.hidden = false;
});
document.getElementById("close-sidebar").addEventListener("click", closeSidebar);
backdrop.addEventListener("click", closeSidebar);

async function init() {
  try {
    const res = await fetch("/api/providers");
    const data = await res.json();
    providers = data.providers || [];
    mcpServers = data.mcp_servers || [];
    defaultProvider = data.default_provider;
  } catch (err) {
    emptySub.textContent = `Could not reach the backend: ${err.message || err}`;
    return;
  }

  // Keep the last AI used, unless it has since been removed from config.toml.
  if (!providers.some((p) => p.name === prefs.provider)) prefs.provider = defaultProvider;
  renderMcp();
  selectProvider(prefs.provider);
  renderHistory();
  updateSendEnabled();
  input.focus();
}

// Markdown libraries load with `defer`; re-render once they're in so saved replies get formatted.
window.addEventListener("load", () => {
  if (window.marked && window.DOMPurify && history.length) renderHistory();
});

init();
