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
const versionEl = document.getElementById("version");
const convStats = document.getElementById("conv-stats");

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

// ---------- language ----------

/** The UI follows the browser's language: French if it prefers French, English otherwise. */
const LANG = (navigator.languages?.[0] || navigator.language || "en").toLowerCase().startsWith("fr")
  ? "fr"
  : "en";

const STRINGS = {
  en: {
    settings: "Settings",
    closeSettings: "Close settings",
    openSettings: "Open settings",
    newChat: "+ New chat",
    ai: "AI",
    aiProvider: "AI provider",
    model: "Model",
    tools: "Tools (MCP)",
    mcpOnly: "MCP only",
    mcpOnlyTitle: "Force the AI to consult a selected MCP tool before answering",
    conversation: "Conversation",
    exportMd: "Export .md",
    exportHtml: "Export .html",
    storedLocally: "Conversations are stored in this browser only.",
    emptyTitle: "How can I help?",
    placeholder: "Message LASEASK... (Enter to send, Shift+Enter for a new line)",
    send: "Send",
    stop: "Stop (Esc)",
    you: "You",
    toolsCount: (n) => `${n} tool${n === 1 ? "" : "s"}`,
    toolsAvailable: (n) => `${n} tool${n === 1 ? "" : "s"} available`,
    offline: "offline",
    offlineTitle: "Not connected: the server was unreachable when the backend started. Check the backend logs.",
    noMcp: "No MCP server configured. Add an [mcp.<name>] block in config.toml.",
    cantUseTools: (label) => `${label} can't use tools: MCP is ignored with this AI.`,
    noTools: "no tools",
    talkingTo: (label, model, tools) =>
      `You're talking to ${label} (${model}), ${tools ? `with ${tools}` : "without tools"}.`,
    backendDown: (msg) => `Could not reach the backend: ${msg}`,
    thinking: "Thinking",
    thinkingLive: "Thinking…",
    running: "running",
    interrupted: "interrupted",
    arguments: "Arguments",
    resultExcerpt: (shown, total) => `Result (first ${shown} of ${total} characters)`,
    result: "Result",
    chars: (n) => `${fmtInt(n)} chars`,
    copy: "Copy",
    copied: "Copied",
    stopped: "*(stopped)*",
    error: "Error",
    tokIn: "in",
    tokCache: "cache",
    tokOut: "out",
    messages: (n) => `${n} message${n === 1 ? "" : "s"}`,
    exportTitle: "LASEASK conversation",
    exportedOn: "Exported on",
    toolCalls: "Tool calls",
    duration: "Duration",
    nothingToExport: "Nothing to export yet.",
  },
  fr: {
    settings: "Réglages",
    closeSettings: "Fermer les réglages",
    openSettings: "Ouvrir les réglages",
    newChat: "+ Nouvelle conversation",
    ai: "IA",
    aiProvider: "Fournisseur d'IA",
    model: "Modèle",
    tools: "Outils (MCP)",
    mcpOnly: "MCP uniquement",
    mcpOnlyTitle: "Oblige l'IA à consulter un outil MCP sélectionné avant de répondre",
    conversation: "Conversation",
    exportMd: "Exporter .md",
    exportHtml: "Exporter .html",
    storedLocally: "Les conversations sont stockées dans ce navigateur uniquement.",
    emptyTitle: "Que puis-je faire pour vous ?",
    placeholder: "Message pour LASEASK… (Entrée pour envoyer, Maj+Entrée pour aller à la ligne)",
    send: "Envoyer",
    stop: "Arrêter (Échap)",
    you: "Vous",
    toolsCount: (n) => `${n} outil${n > 1 ? "s" : ""}`,
    toolsAvailable: (n) => `${n} outil${n > 1 ? "s" : ""} disponible${n > 1 ? "s" : ""}`,
    offline: "hors ligne",
    offlineTitle: "Non connecté : le serveur était injoignable au démarrage du backend. Voir les logs du backend.",
    noMcp: "Aucun serveur MCP configuré. Ajoutez un bloc [mcp.<nom>] dans config.toml.",
    cantUseTools: (label) => `${label} ne peut pas utiliser d'outils : le MCP est ignoré avec cette IA.`,
    noTools: "sans outils",
    talkingTo: (label, model, tools) =>
      `Vous parlez à ${label} (${model}), ${tools ? `avec ${tools}` : "sans outils"}.`,
    backendDown: (msg) => `Backend injoignable : ${msg}`,
    thinking: "Réflexion",
    thinkingLive: "Réflexion…",
    running: "en cours",
    interrupted: "interrompu",
    arguments: "Arguments",
    resultExcerpt: (shown, total) => `Résultat (${shown} premiers caractères sur ${total})`,
    result: "Résultat",
    chars: (n) => `${fmtInt(n)} car.`,
    copy: "Copier",
    copied: "Copié",
    stopped: "*(arrêté)*",
    error: "Erreur",
    tokIn: "entrée",
    tokCache: "cache",
    tokOut: "sortie",
    messages: (n) => `${n} message${n > 1 ? "s" : ""}`,
    exportTitle: "Conversation LASEASK",
    exportedOn: "Exportée le",
    toolCalls: "Appels d'outils",
    duration: "Durée",
    nothingToExport: "Rien à exporter pour l'instant.",
  },
};

function t(key, ...args) {
  const value = STRINGS[LANG][key] ?? STRINGS.en[key] ?? key;
  return typeof value === "function" ? value(...args) : value;
}

function applyStaticStrings() {
  document.documentElement.lang = LANG;
  for (const el of document.querySelectorAll("[data-i18n]")) el.textContent = t(el.dataset.i18n);
  for (const el of document.querySelectorAll("[data-i18n-placeholder]")) el.placeholder = t(el.dataset.i18nPlaceholder);
  for (const el of document.querySelectorAll("[data-i18n-title]")) el.title = t(el.dataset.i18nTitle);
  for (const el of document.querySelectorAll("[data-i18n-aria]")) el.setAttribute("aria-label", t(el.dataset.i18nAria));
}

// ---------- formatting ----------

const numberFmt = new Intl.NumberFormat(LANG === "fr" ? "fr-BE" : "en-US");

function fmtInt(n) {
  return numberFmt.format(n || 0);
}

/** 1234 -> "1.2k", 1234567 -> "1.2M". */
function fmtTokens(n) {
  if (!n) return "0";
  if (n < 1000) return String(n);
  if (n < 1e6) return `${(n / 1000).toFixed(n < 10000 ? 1 : 0)}k`;
  return `${(n / 1e6).toFixed(1)}M`;
}

function fmtDuration(ms) {
  if (ms == null) return "";
  if (ms < 1000) return `${ms} ms`;
  const s = ms / 1000;
  if (s < 60) return `${s.toFixed(s < 10 ? 1 : 0)} s`;
  const m = Math.floor(s / 60);
  return `${m}m${String(Math.floor(s % 60)).padStart(2, "0")}s`;
}

function fmtCost(usd) {
  if (usd == null) return "";
  return `≈ $${usd < 0.01 ? usd.toFixed(4) : usd.toFixed(2)}`;
}

function fmtDate(d) {
  return d.toLocaleString(LANG === "fr" ? "fr-BE" : "en-GB", { dateStyle: "medium", timeStyle: "short" });
}

// ---------- state ----------

/**
 * One entry per message. Only role/content go to the API; assistant entries also keep what the
 * UI shows: which AI answered, the timeline of `steps` (text, thinking, tool calls in the order
 * they happened), token usage and how long it took. Entries saved by version 1 have `tools`
 * (names only) instead of `steps`.
 * @type {{role: string, content: string, provider?: string, model?: string, tools?: string[],
 *   steps?: object[], usage?: object, elapsed_ms?: number, error?: string}[]}
 */
let history = load(HISTORY_KEY, []);
/** @type {{provider?: string, models: Record<string, string>, mcp: Record<string, boolean>, mcpOnly: boolean}} */
let prefs = { models: {}, mcp: {}, mcpOnly: false, ...load(PREFS_KEY, {}) };
let providers = [];
let mcpServers = [];
let pricing = {};
let defaultProvider = "";
let backendVersion = "";
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
    // storage unavailable or full (private window, blocked site data): the app still works, it just forgets
  }
}

// ---------- usage & cost ----------

function emptyUsage() {
  return { input_tokens: 0, output_tokens: 0, cache_read: 0, cache_write: 0 };
}

function addUsage(total, u) {
  for (const k of Object.keys(emptyUsage())) total[k] = (total[k] || 0) + (u[k] || 0);
}

/** Estimated cost in USD from `[pricing]` in config.toml, or null when the model has no prices. */
function costOf(model, usage) {
  const p = pricing[model];
  if (!p || !usage) return null;
  const cacheRead = p.cache_read ?? p.input;
  const cacheWrite = p.cache_write ?? p.input;
  return (
    (usage.input_tokens * p.input +
      usage.output_tokens * p.output +
      usage.cache_read * cacheRead +
      usage.cache_write * cacheWrite) /
    1e6
  );
}

function usageText(model, usage, elapsed) {
  const parts = [];
  if (elapsed != null) parts.push(`⏱ ${fmtDuration(elapsed)}`);
  if (usage && (usage.input_tokens || usage.output_tokens || usage.cache_read)) {
    parts.push(`${t("tokIn")} ${fmtTokens(usage.input_tokens + usage.cache_write)}`);
    if (usage.cache_read) parts.push(`${t("tokCache")} ${fmtTokens(usage.cache_read)}`);
    parts.push(`${t("tokOut")} ${fmtTokens(usage.output_tokens)}`);
    const cost = costOf(model, usage);
    if (cost != null) parts.push(fmtCost(cost));
  }
  return parts.join(" · ");
}

function renderConvStats() {
  let cost = 0;
  let priced = false;
  const usage = emptyUsage();
  for (const m of history) {
    if (m.role !== "assistant" || !m.usage) continue;
    addUsage(usage, m.usage);
    const c = costOf(m.model, m.usage);
    if (c != null) {
      cost += c;
      priced = true;
    }
  }
  const parts = [t("messages", history.length)];
  if (usage.input_tokens || usage.output_tokens) {
    parts.push(`${fmtTokens(usage.input_tokens + usage.cache_write + usage.cache_read + usage.output_tokens)} tokens`);
  }
  if (priced) parts.push(fmtCost(cost));
  convStats.textContent = parts.join(" · ");
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
    item.title = s.connected ? t("toolsAvailable", s.tool_count) : t("offlineTitle");

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
    count.textContent = s.connected ? t("toolsCount", s.tool_count) : t("offline");

    item.append(box, dot, name, count);
    mcpList.appendChild(item);
  }
}

mcpOnlyToggle.addEventListener("change", () => {
  prefs.mcpOnly = mcpOnlyToggle.checked;
  save(PREFS_KEY, prefs);
  updateToolState();
});

function tag(text, cls = "") {
  const el = document.createElement("span");
  el.className = `tag ${cls}`.trim();
  el.textContent = text;
  return el;
}

/** Keeps the MCP hint, the "MCP only" switch and the status lines in line with the selection. */
function updateToolState() {
  const p = currentProvider();
  const label = p ? providerMeta(p.name).label : "";
  const toolCapable = p && TOOL_PROVIDERS.includes(p.name);
  const selected = selectedServers();

  let hint = "";
  if (mcpServers.length === 0) hint = t("noMcp");
  else if (!toolCapable) hint = t("cantUseTools", label);
  mcpHint.textContent = hint;
  mcpHint.hidden = !hint;

  // Same rule the backend enforces for mcp_only (routes.rs chat()).
  const mcpOnlyAvailable = toolCapable && selected.length > 0;
  mcpOnlyToggle.disabled = !mcpOnlyAvailable;
  mcpOnlyToggle.checked = mcpOnlyAvailable && prefs.mcpOnly;

  const model = modelInput.value.trim() || (p ? p.default_model : "");
  const tools = toolCapable && selected.length ? selected.join(", ") : "";

  composerMeta.innerHTML = "";
  if (p) {
    composerMeta.append(tag(label, `p-${p.name} tag-provider`), tag(model));
    composerMeta.append(tools ? tag(`🔧 ${tools}`, "tag-ok") : tag(t("noTools")));
    if (mcpOnlyToggle.checked) composerMeta.append(tag(t("mcpOnly"), "tag-accent"));
  }
  mobileTitle.textContent = p ? `${label} · ${model}` : "";
  emptySub.textContent = p ? t("talkingTo", label, model, tools) : "";
}

// ---------- rendering ----------

/** Markdown when marked + DOMPurify loaded from the CDN, plain text otherwise. */
function markdownHtml(text) {
  if (window.marked && window.DOMPurify) {
    return DOMPurify.sanitize(marked.parse(text, { gfm: true, breaks: true }));
  }
  return null;
}

function renderBody(el, text) {
  const html = markdownHtml(text);
  if (html != null) {
    el.classList.remove("plain");
    el.innerHTML = html;
  } else {
    el.classList.add("plain");
    el.textContent = text;
  }
}

function updateEmptyState() {
  emptyState.hidden = history.length > 0 || thread.childElementCount > 0;
}

/** Follows the output only when the reader is already at the bottom, so scrolling up to read a
 * tool result isn't yanked back down by the next token. */
function isNearBottom() {
  return messagesEl.scrollHeight - messagesEl.scrollTop - messagesEl.clientHeight < 80;
}

function scrollToBottom(force = false) {
  if (force || isNearBottom()) messagesEl.scrollTop = messagesEl.scrollHeight;
}

function appendUser(text) {
  const div = document.createElement("div");
  div.className = "msg user";
  const label = document.createElement("div");
  label.className = "msg-label user-label";
  label.textContent = t("you");
  const body = document.createElement("div");
  body.className = "user-body";
  body.textContent = text;
  div.append(label, body);
  thread.appendChild(div);
  updateEmptyState();
  scrollToBottom(true);
}

/** "fortianalyzer__query_logs" -> ["fortianalyzer", "query_logs"]. */
function splitToolName(name) {
  const i = name.indexOf("__");
  return i === -1 ? ["", name] : [name.slice(0, i), name.slice(i + 2)];
}

/** One-line summary of a tool call's arguments for the collapsed timeline row. */
function argsSummary(args) {
  if (!args || typeof args !== "object") return "";
  const parts = [];
  for (const [k, v] of Object.entries(args)) {
    let value = typeof v === "string" ? v : JSON.stringify(v);
    if (value.length > 40) value = value.slice(0, 39) + "…";
    parts.push(`${k}=${value}`);
  }
  const line = parts.join(" ");
  return line.length > 140 ? line.slice(0, 139) + "…" : line;
}

/** Builds a tool row of the timeline: a clickable summary, and the details (arguments, result
 * excerpt) that fold out. Returns an update function for when the call ends. */
function toolRow(step) {
  const el = document.createElement("details");
  el.className = "step tool";

  const summary = document.createElement("summary");
  const status = document.createElement("span");
  status.className = "tool-status";
  const [server, tool] = splitToolName(step.name);
  const name = document.createElement("span");
  name.className = "tool-name";
  if (server) {
    const s = document.createElement("span");
    s.className = "tool-server";
    s.textContent = `${server}.`;
    name.appendChild(s);
  }
  name.append(tool);
  const args = document.createElement("span");
  args.className = "tool-args";
  args.textContent = argsSummary(step.args);
  const meta = document.createElement("span");
  meta.className = "tool-meta";
  summary.append(status, name, args, meta);

  const details = document.createElement("div");
  details.className = "tool-details";
  el.append(summary, details);

  function update() {
    el.dataset.status = step.status;
    status.textContent = { running: "●", ok: "✓", error: "✕", interrupted: "■" }[step.status] || "·";
    status.title = step.status === "running" ? t("running") : step.status === "interrupted" ? t("interrupted") : "";
    const bits = [];
    if (step.status === "running") bits.push(t("running"));
    else if (step.status === "interrupted") bits.push(t("interrupted"));
    if (step.duration_ms != null) bits.push(fmtDuration(step.duration_ms));
    if (step.chars != null) bits.push(t("chars", step.chars));
    meta.textContent = bits.join(" · ");

    details.innerHTML = "";
    if (step.args !== undefined) {
      const h = document.createElement("div");
      h.className = "detail-label";
      h.textContent = t("arguments");
      const pre = document.createElement("pre");
      pre.textContent = JSON.stringify(step.args, null, 2);
      details.append(h, pre);
    }
    if (step.excerpt) {
      const h = document.createElement("div");
      h.className = "detail-label";
      const shown = step.excerpt.length;
      h.textContent = step.chars > shown ? t("resultExcerpt", fmtInt(shown), fmtInt(step.chars)) : t("result");
      const pre = document.createElement("pre");
      pre.className = step.status === "error" ? "err" : "";
      pre.textContent = step.excerpt;
      details.append(h, pre);
    }
  }
  update();
  return { el, update };
}

function thinkingRow(step) {
  const el = document.createElement("details");
  el.className = "step thinking";
  const summary = document.createElement("summary");
  const label = document.createElement("span");
  label.className = "thinking-label";
  const preview = document.createElement("span");
  preview.className = "thinking-preview";
  summary.append(label, preview);
  const body = document.createElement("div");
  body.className = "thinking-body";
  el.append(summary, body);

  function update(live) {
    el.classList.toggle("live", !!live);
    label.textContent = live ? t("thinkingLive") : t("thinking");
    // The last line gives a sense of progress without opening the block.
    const lines = step.text.trim().split("\n").filter(Boolean);
    preview.textContent = lines.length ? lines[lines.length - 1].replace(/[*#_`]/g, "").slice(0, 120) : "";
    if (el.open) body.textContent = step.text;
  }
  el.addEventListener("toggle", () => {
    if (el.open) body.textContent = step.text;
  });
  update(false);
  return { el, update };
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
  const state = document.createElement("span");
  state.className = "msg-state";
  label.append(providerDot(provider), name, modelEl, state);

  const stepsEl = document.createElement("div");
  stepsEl.className = "steps";

  const footer = document.createElement("div");
  footer.className = "msg-footer";
  const stats = document.createElement("span");
  stats.className = "msg-stats";
  const actions = document.createElement("span");
  actions.className = "msg-actions";
  footer.append(stats, actions);
  footer.hidden = true;

  wrap.append(label, stepsEl, footer);
  thread.appendChild(wrap);
  updateEmptyState();
  scrollToBottom(true);

  /** Everything the model wrote, as sent back to the API next time. */
  let text = "";
  const steps = [];
  const usage = emptyUsage();
  let elapsed = null;
  let errorMessage = null;
  let frame = 0;
  let current = null; // { step, el, update } of the last step
  let dirty = null; // text/thinking step waiting for the next frame

  function renderStats() {
    stats.textContent = usageText(model, usage, elapsed);
  }

  function push(step, view) {
    steps.push(step);
    stepsEl.appendChild(view.el);
    current = { step, ...view };
    return current;
  }

  function textStep() {
    if (current?.step.kind === "text") return current;
    if (current?.step.kind === "thinking") current.update(false);
    const step = { kind: "text", text: "" };
    const body = document.createElement("div");
    body.className = "step text msg-body";
    return push(step, { el: body, update: () => renderBody(body, step.text.trim()) });
  }

  function thinkingStep() {
    if (current?.step.kind === "thinking") return current;
    const step = { kind: "thinking", text: "" };
    return push(step, thinkingRow(step));
  }

  function schedule(view) {
    // A different step is waiting for the frame (text, then thinking in the same frame): draw it
    // now, or it would never be drawn.
    if (dirty && dirty !== view) dirty.update(false);
    dirty = view;
    // Re-render at most once per frame: parsing markdown on every token is wasteful.
    if (!frame) {
      frame = requestAnimationFrame(() => {
        frame = 0;
        if (dirty) dirty.update(dirty.step.kind === "thinking");
        dirty = null;
        scrollToBottom();
      });
    }
  }

  function flush() {
    if (frame) cancelAnimationFrame(frame);
    frame = 0;
    if (dirty) dirty.update(false);
    dirty = null;
  }

  const toolViews = new Map();

  return {
    wrap,
    steps,
    usage,
    get text() {
      return text;
    },
    get errorMessage() {
      return errorMessage;
    },
    get elapsed() {
      return elapsed;
    },
    setState(value) {
      state.textContent = value;
    },
    setElapsed(ms) {
      elapsed = ms;
      renderStats();
    },
    append(chunk) {
      text += chunk;
      const view = textStep();
      view.step.text += chunk;
      schedule(view);
    },
    thinking(chunk) {
      const view = thinkingStep();
      view.step.text += chunk;
      schedule(view);
    },
    toolStart(ev) {
      flush();
      if (current?.step.kind === "thinking") current.update(false);
      const step = { kind: "tool", id: ev.id, name: ev.name, args: ev.args, status: "running" };
      const view = push(step, toolRow(step));
      toolViews.set(step, view);
      scrollToBottom();
    },
    toolEnd(ev) {
      // Pair by id; the oldest call still running wins if a provider reuses (or omits) ids.
      const step = steps.find((s) => s.kind === "tool" && s.status === "running" && s.id === ev.id)
        || steps.find((s) => s.kind === "tool" && s.status === "running");
      if (!step) return;
      step.status = ev.ok ? "ok" : "error";
      step.duration_ms = ev.duration_ms;
      step.chars = ev.chars;
      step.excerpt = ev.excerpt;
      toolViews.get(step)?.update();
    },
    addUsage(u) {
      addUsage(usage, u);
      footer.hidden = false;
      renderStats();
    },
    error(message) {
      errorMessage = message;
      const err = document.createElement("div");
      err.className = "msg-error";
      err.textContent = `${t("error")}: ${message}`;
      stepsEl.appendChild(err);
      current = null;
      scrollToBottom();
    },
    /** Rebuilds a saved reply. */
    restore(msg) {
      for (const s of msg.steps || []) {
        if (s.kind === "text") {
          const view = textStep();
          view.step.text = s.text;
          view.update();
        } else if (s.kind === "thinking") {
          const view = thinkingStep();
          view.step.text = s.text;
          view.update(false);
        } else if (s.kind === "tool") {
          const step = { ...s };
          const view = push(step, toolRow(step));
          toolViews.set(step, view);
        }
      }
      // Version 1 entries: tool names only, then the text.
      if (!msg.steps) {
        for (const name of msg.tools || []) {
          const step = { kind: "tool", name, status: "ok" };
          push(step, toolRow(step));
        }
        const view = textStep();
        view.step.text = msg.content;
        view.update();
      }
      text = msg.content || "";
      if (msg.usage) addUsage(usage, msg.usage);
      elapsed = msg.elapsed_ms ?? null;
      if (msg.error) this.error(msg.error);
    },
    finish() {
      flush();
      if (current?.step.kind === "thinking") current.update(false);
      wrap.classList.remove("streaming");
      state.textContent = "";
      for (const s of steps) {
        if (s.kind === "tool" && s.status === "running") {
          s.status = "interrupted";
          toolViews.get(s)?.update();
        }
      }
      // The backend's completion check asks the model to reply "DONE" when finished; some
      // models tack it onto the end of their answer instead.
      const strip = (s) => s.replace(/\s*\**DONE\**\.?\s*$/, "");
      text = strip(text);
      const lastText = [...steps].reverse().find((s) => s.kind === "text");
      if (lastText) lastText.text = strip(lastText.text);
      // Redraw every text step from its data, and drop the ones left empty (e.g. only the blank
      // line the backend puts between turns).
      const textViews = [...stepsEl.querySelectorAll(".step.text")];
      const textSteps = steps.filter((s) => s.kind === "text");
      textSteps.forEach((s, i) => {
        const el = textViews[i];
        if (s.text.trim()) {
          if (el) renderBody(el, s.text.trim());
        } else {
          el?.remove();
          steps.splice(steps.indexOf(s), 1);
        }
      });
      renderStats();
      footer.hidden = !text && !stats.textContent;
      if (text) this.addCopy();
    },
    addCopy() {
      const btn = document.createElement("button");
      btn.type = "button";
      btn.textContent = t("copy");
      btn.addEventListener("click", async () => {
        await copyText(text);
        btn.textContent = t("copied");
        setTimeout(() => (btn.textContent = t("copy")), 1500);
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
      reply.restore(msg);
      reply.finish();
    }
  }
  updateEmptyState();
  renderConvStats();
  scrollToBottom(true);
}

// ---------- sending ----------

function setSending(sending) {
  sendBtn.classList.toggle("stop", sending);
  sendBtn.textContent = sending ? "■" : "↑";
  sendBtn.setAttribute("aria-label", sending ? t("stop") : t("send"));
  sendBtn.title = sending ? t("stop") : "";
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
  renderConvStats();

  const provider = prefs.provider;
  const model = modelInput.value.trim() || currentProvider().default_model;
  const toolCapable = TOOL_PROVIDERS.includes(provider);
  const reply = appendAssistant(provider, model);
  reply.wrap.classList.add("streaming");

  const started = Date.now();
  const tick = () => reply.setState(`● ${fmtDuration(Math.floor((Date.now() - started) / 1000) * 1000)}`);
  tick();
  const timer = setInterval(tick, 1000);

  abortController = new AbortController();
  setSending(true);

  try {
    await streamChat(
      {
        provider,
        model,
        // Replies with no text (an error before any answer) can't go back to the API.
        messages: history.filter((m) => m.content).map(({ role, content }) => ({ role, content })),
        mcp_servers: toolCapable ? selectedServers() : [],
        mcp_only: mcpOnlyToggle.checked,
      },
      abortController.signal,
      (event) => {
        switch (event.type) {
          case "delta":
            if (event.text) reply.append(event.text);
            break;
          case "thinking":
            reply.thinking(event.text);
            break;
          case "tool_start":
            reply.toolStart(event);
            break;
          case "tool_end":
            reply.toolEnd(event);
            break;
          case "usage":
            reply.addUsage(event);
            break;
          case "error":
            reply.error(event.message);
            break;
        }
      },
    );
  } catch (err) {
    if (err.name === "AbortError") reply.append(`\n\n${t("stopped")}`);
    else reply.error(err.message || String(err));
  }

  clearInterval(timer);
  reply.setElapsed(Date.now() - started);
  reply.finish();
  if (reply.text || reply.steps.length || reply.errorMessage) {
    history.push({
      role: "assistant",
      content: reply.text,
      provider,
      model,
      steps: reply.steps,
      usage: reply.usage,
      elapsed_ms: reply.elapsed,
      ...(reply.errorMessage ? { error: reply.errorMessage } : {}),
    });
    save(HISTORY_KEY, history);
  }
  renderConvStats();

  abortController = null;
  setSending(false);
  input.focus();
}

/**
 * POSTs to /api/chat and parses the text/event-stream response manually
 * (EventSource can't do POST bodies), invoking onEvent for each normalized StreamEvent the
 * backend emits: {type: "delta"|"thinking"|"tool_start"|"tool_end"|"usage"|"done"|"error", ...}.
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

// ---------- export ----------

function exportFileName(ext) {
  const d = new Date();
  const pad = (n) => String(n).padStart(2, "0");
  return `laseask-${d.getFullYear()}${pad(d.getMonth() + 1)}${pad(d.getDate())}-${pad(d.getHours())}${pad(d.getMinutes())}.${ext}`;
}

function download(name, content, type) {
  const url = URL.createObjectURL(new Blob([content], { type }));
  const a = document.createElement("a");
  a.href = url;
  a.download = name;
  document.body.appendChild(a);
  a.click();
  a.remove();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}

/** The steps of a saved reply, version 1 entries included. */
function stepsOf(msg) {
  if (msg.steps) return msg.steps;
  return [...(msg.tools || []).map((name) => ({ kind: "tool", name, status: "ok" })), { kind: "text", text: msg.content }];
}

function toolLine(s) {
  const status = { ok: "✓", error: "✕", interrupted: "■", running: "●" }[s.status] || "·";
  const bits = [s.duration_ms != null ? fmtDuration(s.duration_ms) : "", s.chars != null ? t("chars", s.chars) : ""].filter(Boolean);
  return `${status} ${s.name}${bits.length ? ` (${bits.join(" · ")})` : ""}`;
}

function fence(text, lang = "") {
  // A fence longer than any backtick run inside, so tool output can't break out of it.
  const longest = Math.max(2, ...(text.match(/`+/g) || []).map((m) => m.length));
  const f = "`".repeat(longest + 1);
  return `${f}${lang}\n${text}\n${f}`;
}

function exportMarkdown() {
  const out = [`# ${t("exportTitle")}`, "", `*${t("exportedOn")} ${fmtDate(new Date())} · LASEASK ${backendVersion}*`, ""];
  for (const msg of history) {
    if (msg.role === "user") {
      out.push(`## ${t("you")}`, "", msg.content, "");
      continue;
    }
    out.push(`## ${providerMeta(msg.provider || "").label} · ${msg.model || ""}`, "");
    for (const s of stepsOf(msg)) {
      if (s.kind === "text" && s.text.trim()) out.push(s.text.trim(), "");
      else if (s.kind === "thinking" && s.text.trim()) {
        out.push(`> **${t("thinking")}**`, ...s.text.trim().split("\n").map((l) => `> ${l}`), "");
      } else if (s.kind === "tool") {
        out.push(`**🔧 ${toolLine(s)}**`, "");
        if (s.args !== undefined) out.push(`${t("arguments")}:`, "", fence(JSON.stringify(s.args, null, 2), "json"), "");
        if (s.excerpt) {
          const label = s.chars > s.excerpt.length ? t("resultExcerpt", fmtInt(s.excerpt.length), fmtInt(s.chars)) : t("result");
          out.push(`${label}:`, "", fence(s.excerpt), "");
        }
      }
    }
    if (msg.error) out.push(`**${t("error")}:** ${msg.error}`, "");
    const stats = usageText(msg.model, msg.usage, msg.elapsed_ms);
    if (stats) out.push(`*${stats}*`, "");
  }
  return out.join("\n");
}

function esc(s) {
  return String(s).replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);
}

function mdOrPre(text) {
  return markdownHtml(text) ?? `<pre class="plain">${esc(text)}</pre>`;
}

function exportHtml() {
  const parts = [];
  for (const msg of history) {
    if (msg.role === "user") {
      parts.push(`<section class="msg user"><div class="who">${esc(t("you"))}</div><div class="body">${esc(msg.content)}</div></section>`);
      continue;
    }
    const inner = [];
    for (const s of stepsOf(msg)) {
      if (s.kind === "text" && s.text.trim()) inner.push(`<div class="text">${mdOrPre(s.text.trim())}</div>`);
      else if (s.kind === "thinking" && s.text.trim()) {
        inner.push(`<details class="thinking"><summary>${esc(t("thinking"))}</summary><pre>${esc(s.text.trim())}</pre></details>`);
      } else if (s.kind === "tool") {
        const body = [];
        if (s.args !== undefined) body.push(`<div class="lbl">${esc(t("arguments"))}</div><pre>${esc(JSON.stringify(s.args, null, 2))}</pre>`);
        if (s.excerpt) {
          const label = s.chars > s.excerpt.length ? t("resultExcerpt", fmtInt(s.excerpt.length), fmtInt(s.chars)) : t("result");
          body.push(`<div class="lbl">${esc(label)}</div><pre>${esc(s.excerpt)}</pre>`);
        }
        inner.push(`<details class="tool ${esc(s.status || "")}"><summary>${esc(toolLine(s))}</summary>${body.join("")}</details>`);
      }
    }
    if (msg.error) inner.push(`<div class="error">${esc(t("error"))}: ${esc(msg.error)}</div>`);
    const stats = usageText(msg.model, msg.usage, msg.elapsed_ms);
    parts.push(
      `<section class="msg assistant"><div class="who">${esc(providerMeta(msg.provider || "").label)} <span class="dim">${esc(msg.model || "")}</span></div>${inner.join("")}${stats ? `<div class="stats">${esc(stats)}</div>` : ""}</section>`,
    );
  }

  return `<!doctype html>
<html lang="${LANG}">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>${esc(t("exportTitle"))}</title>
<style>
:root { color-scheme: light dark; --bg: #fff; --fg: #1a1d21; --dim: #667; --line: #d8dbe0; --code: #f3f4f6; --ok: #1f9d63; --err: #c62828; --acc: #2f6fd6; }
@media (prefers-color-scheme: dark) { :root { --bg: #0e1116; --fg: #d6dae0; --dim: #7d8590; --line: #262c35; --code: #161b22; --ok: #3fb950; --err: #f85149; --acc: #58a6ff; } }
body { background: var(--bg); color: var(--fg); font: 14px/1.55 "Segoe UI", system-ui, sans-serif; max-width: 860px; margin: 0 auto; padding: 24px 16px; }
h1 { font-size: 20px; margin: 0 0 4px; } .meta { color: var(--dim); font-size: 12px; margin-bottom: 24px; }
.msg { border-top: 1px solid var(--line); padding: 14px 0; } .who { font-weight: 700; font-size: 12px; text-transform: uppercase; letter-spacing: .06em; margin-bottom: 8px; }
.user .body { white-space: pre-wrap; } .dim { color: var(--dim); font-weight: 400; text-transform: none; letter-spacing: 0; }
pre, code { font-family: "Cascadia Mono", Consolas, ui-monospace, monospace; font-size: 12.5px; }
pre { background: var(--code); border: 1px solid var(--line); border-radius: 6px; padding: 10px 12px; overflow-x: auto; white-space: pre-wrap; word-break: break-word; }
details { border-left: 2px solid var(--line); padding: 2px 0 2px 10px; margin: 6px 0; } summary { cursor: pointer; font-family: "Cascadia Mono", Consolas, monospace; font-size: 12.5px; }
.tool.ok summary { color: var(--ok); } .tool.error summary { color: var(--err); } .thinking summary { color: var(--dim); font-style: italic; }
.lbl { color: var(--dim); font-size: 11px; text-transform: uppercase; letter-spacing: .06em; margin-top: 8px; }
table { border-collapse: collapse; } th, td { border: 1px solid var(--line); padding: 4px 8px; text-align: left; }
.stats { color: var(--dim); font-size: 12px; font-family: "Cascadia Mono", Consolas, monospace; margin-top: 8px; } .error { color: var(--err); }
a { color: var(--acc); }
</style>
</head>
<body>
<h1>${esc(t("exportTitle"))}</h1>
<div class="meta">${esc(t("exportedOn"))} ${esc(fmtDate(new Date()))} · LASEASK ${esc(backendVersion)}</div>
${parts.join("\n")}
</body>
</html>`;
}

document.getElementById("export-md").addEventListener("click", () => {
  if (!history.length) return alert(t("nothingToExport"));
  download(exportFileName("md"), exportMarkdown(), "text/markdown;charset=utf-8");
});
document.getElementById("export-html").addEventListener("click", () => {
  if (!history.length) return alert(t("nothingToExport"));
  download(exportFileName("html"), exportHtml(), "text/html;charset=utf-8");
});

// ---------- wiring ----------

sendBtn.addEventListener("click", () => {
  if (abortController) abortController.abort();
  else send();
});

input.addEventListener("input", () => {
  input.style.height = "auto";
  input.style.height = Math.min(input.scrollHeight, 240) + "px";
  updateSendEnabled();
});

input.addEventListener("keydown", (e) => {
  if (e.key === "Enter" && !e.shiftKey && !e.isComposing) {
    e.preventDefault();
    send();
  }
});

document.addEventListener("keydown", (e) => {
  if (e.key === "Escape" && abortController) abortController.abort();
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
  applyStaticStrings();
  setSending(false);
  renderConvStats();
  try {
    const res = await fetch("/api/providers");
    const data = await res.json();
    providers = data.providers || [];
    mcpServers = data.mcp_servers || [];
    pricing = data.pricing || {};
    defaultProvider = data.default_provider;
    backendVersion = data.version ? `v${data.version}` : "";
    versionEl.textContent = backendVersion;
  } catch (err) {
    emptySub.textContent = t("backendDown", err.message || err);
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
