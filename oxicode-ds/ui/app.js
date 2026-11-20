const tauriCore = window.__TAURI__?.core;
const tauriEvent = window.__TAURI__?.event;
const invoke = tauriCore
  ? tauriCore.invoke.bind(tauriCore)
  : async () => {
      throw new Error("Tauri core IPC unavailable");
    };
const listen = tauriEvent
  ? tauriEvent.listen.bind(tauriEvent)
  : async () => () => {};

const $ = (id) => document.getElementById(id);

const SLASH_COMMANDS = [
  { name: "/new", desc: "Start a new conversation" },
  { name: "/connect", desc: "Configure AI provider, key & base URL" },
  { name: "/models", desc: "Switch model for active provider" },
  { name: "/settings", desc: "Open preferences, themes & Fox mascot" },
  { name: "/themes", desc: "Switch workspace color theme" },
  { name: "/dir", desc: "Switch working directory" },
  { name: "/compact", desc: "Summarize context to free tokens" },
  { name: "/undo", desc: "Revert last file change" },
  { name: "/redo", desc: "Reapply reverted file change" },
  { name: "/export", desc: "Export conversation to Markdown" },
  { name: "/clear", desc: "Clear visible messages" },
];

const TOKEN_PRESETS = [4096, 8192, 16384, 32768, 65536];

const state = {
  boot: null,
  mode: "build",
  busy: false,
  ctxTokens: 0,
  maxTokens: 64000,
  activeSessionId: null,
  streamAssistantEl: null,
  streamText: "",
  activeToolCard: null,
  pendingApprovalId: null,
  acItems: [],
  acIndex: 0,
  acTrigger: null,
  acPrefixLen: 0,
  settingsCat: "general",
  animationsEnabled: localStorage.getItem("oxicode.animations") !== "0",
  foxGaze: { clientX: null, clientY: null, source: "idle", until: 0 },
  lastMouse: { x: null, y: null },
};

/* Utilities */
function escapeHtml(text) {
  return String(text ?? "")
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

function showToast(msg) {
  const el = $("toast");
  if (!el) return;
  el.textContent = msg;
  el.classList.remove("hidden");
  clearTimeout(showToast._timer);
  showToast._timer = setTimeout(() => el.classList.add("hidden"), 2800);
}

function applyThemeVars(css) {
  if (!css || typeof css !== "object") return;
  const r = document.documentElement.style;
  if (css.bg) r.setProperty("--bg", css.bg);
  if (css.bg_panel) {
    r.setProperty("--bg-panel", css.bg_panel);
    r.setProperty("--bg-sidebar", css.bg_panel);
  }
  if (css.bg_element) r.setProperty("--bg-element", css.bg_element);
  if (css.border) r.setProperty("--border", css.border);
  if (css.border_active) r.setProperty("--border-active", css.border_active);
  if (css.primary) r.setProperty("--primary", css.primary);
  if (css.accent) r.setProperty("--accent", css.accent);
  if (css.secondary) r.setProperty("--secondary", css.secondary);
  if (css.text) r.setProperty("--text", css.text);
  if (css.muted) r.setProperty("--muted", css.muted);
  if (css.green) r.setProperty("--green", css.green);
  if (css.red) r.setProperty("--red", css.red);
  if (css.yellow) r.setProperty("--yellow", css.yellow);
  if (css.cyan) r.setProperty("--cyan", css.cyan);
}

function applyAnimationPref(enabled) {
  state.animationsEnabled = Boolean(enabled);
  localStorage.setItem("oxicode.animations", state.animationsEnabled ? "1" : "0");
  document.body.classList.toggle("no-animations", !state.animationsEnabled);
}

function formatTokens(n) {
  if (n >= 1000) return `${(n / 1000).toFixed(1).replace(/\.0$/, "")}k`;
  return String(n);
}

function updateContextMeter() {
  $("ctx-meter").textContent = `${formatTokens(state.ctxTokens)} / ${formatTokens(state.maxTokens)}`;
}

function setBusy(busy) {
  state.busy = busy;
  $("btn-send").classList.toggle("hidden", busy);
  $("btn-stop").classList.toggle("hidden", !busy);
}

function setModeUi(mode) {
  state.mode = mode === "plan" ? "plan" : "build";
  document.querySelectorAll(".mode-btn").forEach((b) => {
    b.classList.toggle("active", b.dataset.mode === state.mode);
  });
}

function scrollMessagesBottom() {
  const box = $("messages-container");
  box.scrollTop = box.scrollHeight;
}

function setMascotVisible(visible) {
  const showFox = visible !== false;
  $("welcome-fox-wrap").classList.toggle("hidden", !showFox);
  $("welcome-wordmark").classList.toggle("hidden", showFox);
}

function ensureChatStarted() {
  $("center-stage").classList.remove("is-empty");
  $("messages-container").classList.remove("hidden");
}

function resetToEmptyView() {
  $("center-stage").classList.add("is-empty");
  $("messages-container").innerHTML = "";
  $("messages-container").classList.add("hidden");
  $("task-title").textContent = "New Conversation";
  state.ctxTokens = 0;
  state.activeToolCard = null;
  state.streamAssistantEl = null;
  state.streamText = "";
  updateContextMeter();
  $("prompt-input").focus();
}

function renderInlineMd(s) {
  let out = escapeHtml(s);
  out = out.replace(/`([^`]+)`/g, "<code>$1</code>");
  out = out.replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>");
  out = out.replace(/\*([^*]+)\*/g, "<em>$1</em>");
  return out;
}

function renderMarkdown(md) {
  const parts = String(md || "").split(/```([a-zA-Z0-9_-]*)\n([\s\S]*?)```/g);
  let html = "";
  for (let i = 0; i < parts.length; i++) {
    if (i % 3 === 0) {
      const blocks = parts[i].split(/\n{2,}/);
      for (const blk of blocks) {
        const trimmed = blk.trim();
        if (!trimmed) continue;
        const lines = trimmed.split("\n");
        if (lines.every((l) => /^[-*]\s+/.test(l))) {
          html += `<ul>${lines.map((l) => `<li>${renderInlineMd(l.replace(/^[-*]\s+/, ""))}</li>`).join("")}</ul>`;
        } else if (lines.every((l) => /^\d+\.\s+/.test(l))) {
          html += `<ol>${lines.map((l) => `<li>${renderInlineMd(l.replace(/^\d+\.\s+/, ""))}</li>`).join("")}</ol>`;
        } else {
          html += `<p>${renderInlineMd(trimmed).replace(/\n/g, "<br/>")}</p>`;
        }
      }
    } else if (i % 3 === 1) {
      const lang = parts[i] || "text";
      const code = parts[i + 1] || "";
      const encoded = encodeURIComponent(code);
      html += `<div class="code-block-wrap"><div class="code-block-head"><span>${escapeHtml(lang)}</span><button class="copy-code-btn" type="button" data-code="${encoded}">Copy</button></div><pre><code>${escapeHtml(code)}</code></pre></div>`;
      i += 1;
    }
  }
  return html;
}

function appendUserMessage(text) {
  ensureChatStarted();
  const wrap = document.createElement("div");
  wrap.className = "msg msg-user";
  wrap.innerHTML = `<div class="msg-user-bubble">${escapeHtml(text)}</div>`;
  $("messages-container").appendChild(wrap);
  scrollMessagesBottom();
}

function appendSystemMessage(text) {
  ensureChatStarted();
  const wrap = document.createElement("div");
  wrap.className = "msg msg-assistant";
  wrap.innerHTML = `<div class="msg-assistant-role">oxicode</div><div class="msg-assistant-body">${renderMarkdown(text)}</div>`;
  $("messages-container").appendChild(wrap);
  scrollMessagesBottom();
}

function ensureStreamingAssistant() {
  ensureChatStarted();
  if (!state.streamAssistantEl) {
    const wrap = document.createElement("div");
    wrap.className = "msg msg-assistant";
    wrap.innerHTML = `<div class="msg-assistant-role">oxicode</div><div class="msg-assistant-body"></div>`;
    $("messages-container").appendChild(wrap);
    state.streamAssistantEl = wrap.querySelector(".msg-assistant-body");
    state.streamText = "";
  }
  return state.streamAssistantEl;
}

function appendToolStart(name, preview) {
  ensureChatStarted();
  const card = document.createElement("div");
  card.className = "tool-card";
  card.innerHTML = `<div class="tool-card-head"><span class="tool-status-dot"></span><span class="tool-name">${escapeHtml(name)}</span><span class="tool-summary">${escapeHtml(preview || "running...")}</span></div><div class="tool-card-body hidden"></div>`;
  card.querySelector(".tool-card-head").onclick = () => {
    card.querySelector(".tool-card-body").classList.toggle("hidden");
  };
  $("messages-container").appendChild(card);
  state.activeToolCard = card;
  scrollMessagesBottom();
}

function finishToolCard(name, output, isError) {
  let card = state.activeToolCard;
  if (!card) {
    appendToolStart(name, "");
    card = state.activeToolCard;
  }
  const dot = card.querySelector(".tool-status-dot");
  dot.classList.add(isError ? "err" : "ok");
  const showDetails = state.boot?.show_tool_details !== false;
  card.querySelector(".tool-card-body").textContent = showDetails
    ? output || "(no output)"
    : "(tool details hidden in Settings)";
  state.activeToolCard = null;
  scrollMessagesBottom();
}

/* Bootstrap & Sidebar Sync */
function syncBootstrapUI(boot) {
  if (!boot) return;
  state.boot = boot;
  state.maxTokens = boot.context_window || 64000;
  applyThemeVars(boot.theme);

  const dirParts = String(boot.work_dir || "").split(/[/\\]/).filter(Boolean);
  const shortDir = dirParts[dirParts.length - 1] || boot.work_dir || "workspace";

  $("work-dir-name").textContent = shortDir;
  $("work-dir-name").title = boot.work_dir || "";
  $("work-dir-short").textContent = shortDir;
  $("work-dir-path").textContent = boot.work_dir || "";

  const branchEl = $("git-branch");
  const footerBranch = $("footer-git-branch");
  if (boot.git_branch) {
    branchEl.textContent = boot.git_branch;
    branchEl.classList.remove("hidden");
    footerBranch.textContent = `(${boot.git_branch})`;
    footerBranch.classList.remove("hidden");
  } else {
    branchEl.classList.add("hidden");
    footerBranch.classList.add("hidden");
  }

  $("chk-auto-approve").checked = Boolean(boot.auto_approve);
  $("composer-model-label").textContent = `${boot.provider_name || boot.provider_id} / ${boot.model}`;
  $("btn-undo").disabled = boot.undo_count === 0;
  $("btn-redo").disabled = boot.redo_count === 0;
  $("welcome-subtitle").textContent = `${boot.provider_name} / ${boot.model} · ${boot.has_key ? "connected" : "setup API key"}`;

  const statusMeta = $("sidebar-provider-status");
  if (statusMeta) {
    statusMeta.textContent = boot.has_key ? "ready" : "setup";
    statusMeta.classList.toggle("connected", Boolean(boot.has_key));
  }

  const cfgPath = $("settings-config-path");
  if (cfgPath && boot.config_path) cfgPath.textContent = boot.config_path;

  setMascotVisible(boot.show_fox_mascot !== false);
  updateContextMeter();

  const changedCount = (boot.changed_files || []).length;
  const badge = $("inspector-badge");
  badge.textContent = String(changedCount);
  badge.classList.toggle("hidden", changedCount === 0);

  renderSessions(boot.sessions || []);
  renderChanges(boot.changed_files || []);
  renderFilesList();
  renderExtensions(boot);

  if (!$("settings-view").classList.contains("hidden")) {
    renderSettingsSection();
  }
}

function renderSessions(sessions) {
  const list = $("session-list");
  list.innerHTML = "";
  if (sessions.length === 0) {
    list.innerHTML = `<div class="session-empty">No conversations yet</div>`;
    return;
  }
  for (const s of sessions) {
    const count = Array.isArray(s.messages) ? s.messages.length : 0;
    const btn = document.createElement("button");
    btn.className = `session-item ${state.activeSessionId === s.id ? "active" : ""}`;
    btn.type = "button";
    btn.innerHTML = `<span class="session-item-title">${escapeHtml(s.title || s.id)}</span><span class="session-item-meta">${count}</span>`;
    btn.onclick = () => openSavedSession(s.id);
    list.appendChild(btn);
  }
}

async function openSavedSession(sessionId) {
  try {
    const loaded = await invoke("load_session", { sessionId });
    const sess = loaded.session;
    state.activeSessionId = sess.id;
    $("messages-container").innerHTML = "";
    state.activeToolCard = null;
    state.streamAssistantEl = null;
    if (!sess.messages || sess.messages.length === 0) {
      resetToEmptyView();
    } else {
      ensureChatStarted();
      for (const m of sess.messages) {
        if (m.role === "user") appendUserMessage(m.content);
        else if (m.role === "assistant" && m.content) appendSystemMessage(m.content);
      }
      $("task-title").textContent = sess.title || sess.id;
    }
    syncBootstrapUI(loaded.bootstrap);
  } catch (e) {
    showToast(String(e));
  }
}

function renderChanges(changes) {
  const list = $("changed-files-list");
  list.innerHTML = "";
  if (changes.length === 0) {
    list.innerHTML = `<div style="padding:8px;color:var(--muted);font-size:12px;">No modified files yet</div>`;
    $("diff-viewer").classList.add("hidden");
    return;
  }
  for (const ch of changes) {
    const row = document.createElement("div");
    row.className = "list-row";
    row.innerHTML = `<span class="file-badge" style="color:${escapeHtml(ch.icon_color)}">${escapeHtml(ch.icon_badge)}</span><span style="flex:1;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;">${escapeHtml(ch.path)}</span><span class="diff-counts"><span class="diff-add">+${ch.added}</span> <span class="diff-del">-${ch.removed}</span></span>`;
    list.appendChild(row);
  }
}

function renderFilesList() {
  const list = $("workspace-files-list");
  list.innerHTML = "";
  const q = ($("file-filter").value || "").trim().toLowerCase();
  const files = (state.boot?.workspace_files || []).filter(
    (f) => !q || String(f.path || "").toLowerCase().includes(q)
  );
  for (const f of files.slice(0, 150)) {
    const row = document.createElement("div");
    row.className = "list-row";
    row.innerHTML = `<span class="file-badge" style="color:${escapeHtml(f.icon_color)}">${escapeHtml(f.icon_badge)}</span><span style="flex:1;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;">${escapeHtml(f.path)}</span>`;
    row.onclick = () => {
      const inp = $("prompt-input");
      inp.value += (inp.value && !inp.value.endsWith(" ") ? " " : "") + `@${f.path} `;
      inp.focus();
    };
    list.appendChild(row);
  }
}

function renderExtensions(boot) {
  const sk = $("skills-list");
  sk.innerHTML = (boot.skills || []).length
    ? boot.skills.map((s) => `<div class="ext-row"><span>${escapeHtml(s.name)}</span><span style="color:var(--muted)">${escapeHtml(s.description.slice(0, 26))}</span></div>`).join("")
    : `<div class="ext-row"><span style="color:var(--muted)">No skills installed</span></div>`;

  const pl = $("plugins-list");
  pl.innerHTML = "";
  if ((boot.plugins || []).length === 0) {
    pl.innerHTML = `<div class="ext-row"><span style="color:var(--muted)">No plugins installed</span></div>`;
  } else {
    for (const p of boot.plugins) {
      const row = document.createElement("div");
      row.className = "ext-row";
      row.innerHTML = `<span>${escapeHtml(p.name)}</span><button class="mini-btn" type="button">${p.enabled ? "Disable" : "Enable"}</button>`;
      row.querySelector("button").onclick = async () => {
        try {
          syncBootstrapUI(await invoke("toggle_plugin", { pluginId: p.id }));
        } catch (e) {
          showToast(String(e));
        }
      };
      pl.appendChild(row);
    }
  }

  const cmds = $("commands-list");
  cmds.innerHTML = (boot.custom_commands || []).length
    ? boot.custom_commands.map((c) => `<div class="ext-row"><span>/${escapeHtml(c.slash)}</span><span style="color:var(--muted)">${escapeHtml(c.description.slice(0, 24))}</span></div>`).join("")
    : `<div class="ext-row"><span style="color:var(--muted)">No custom commands</span></div>`;

  const mcps = $("mcp-list");
  mcps.innerHTML = (boot.mcp_servers || []).length
    ? boot.mcp_servers
        .map((m) => {
          const ok = !m.error;
          return `<div class="ext-row"><span>${escapeHtml(m.name)}</span><span style="color:${ok ? "var(--green)" : "var(--red)"}">${ok ? `${m.tool_count} tools` : "error"}</span></div>`;
        })
        .join("")
    : `<div class="ext-row"><span style="color:var(--muted)">No MCP servers configured</span></div>`;
}

function openInspectorTab(tabName) {
  $("right-sidebar").classList.remove("collapsed");
  document.querySelectorAll(".tab-btn").forEach((b) => {
    b.classList.toggle("active", b.dataset.tab === tabName);
  });
  document.querySelectorAll(".tab-pane").forEach((p) => {
    p.classList.toggle("active", p.id === `tab-${tabName}`);
  });
}

/* Antigravity (+) Add Context Popover */
function closePlusMenu() {
  $("plus-menu").classList.add("hidden");
  $("btn-plus-menu").classList.remove("open");
}

function togglePlusMenu() {
  const menu = $("plus-menu");
  const isOpen = !menu.classList.contains("hidden");
  if (isOpen) closePlusMenu();
  else {
    hideAutocomplete();
    menu.classList.remove("hidden");
    $("btn-plus-menu").classList.add("open");
  }
}

/* Autocomplete (/actions & @files) */
function hideAutocomplete() {
  state.acItems = [];
  state.acTrigger = null;
  $("ac-popup").classList.add("hidden");
}

function updateAutocomplete() {
  const val = $("prompt-input").value;
  if (val.startsWith("/")) {
    const q = val.toLowerCase();
    const custom = (state.boot?.custom_commands || []).map((c) => ({
      name: `/${c.slash}`,
      desc: c.description || "custom command",
    }));
    const all = [...SLASH_COMMANDS, ...custom].filter((c) => c.name.startsWith(q));
    if (all.length === 0) return hideAutocomplete();
    state.acTrigger = "/";
    state.acPrefixLen = val.length;
    renderAutocomplete(all.slice(0, 10));
    return;
  }
  const atMatch = val.match(/@([^\s@]*)$/);
  if (atMatch && state.boot) {
    const query = atMatch[1].toLowerCase();
    const matches = (state.boot.workspace_files || [])
      .filter((f) => !query || String(f.path).toLowerCase().includes(query))
      .slice(0, 10)
      .map((f) => ({ name: `@${f.path}`, desc: f.icon_badge || "file" }));
    if (matches.length === 0) return hideAutocomplete();
    state.acTrigger = "@";
    state.acPrefixLen = atMatch[0].length;
    renderAutocomplete(matches);
    return;
  }
  hideAutocomplete();
}

function renderAutocomplete(items) {
  state.acItems = items;
  if (state.acIndex >= items.length) state.acIndex = 0;
  const popup = $("ac-popup");
  popup.classList.remove("hidden");
  popup.innerHTML = items
    .map(
      (it, idx) =>
        `<div class="ac-item ${idx === state.acIndex ? "selected" : ""}" data-idx="${idx}"><span class="ac-name">${escapeHtml(it.name)}</span><span class="ac-desc">${escapeHtml(it.desc)}</span></div>`
    )
    .join("");
  popup.querySelectorAll(".ac-item").forEach((el) => {
    el.onclick = () => applyAutocomplete(Number(el.dataset.idx));
  });
}

function applyAutocomplete(idx) {
  const item = state.acItems[idx];
  if (!item) return;
  const inp = $("prompt-input");
  if (state.acTrigger === "/") {
    inp.value = item.name;
    hideAutocomplete();
    if (SLASH_COMMANDS.some((c) => c.name === item.name)) {
      sendCurrentPrompt();
    } else {
      inp.value += " ";
    }
  } else if (state.acTrigger === "@") {
    inp.value = inp.value.slice(0, inp.value.length - state.acPrefixLen) + item.name + " ";
    hideAutocomplete();
  }
  inp.focus();
}

/* Agent Stream Events (exact match to Rust AgentStreamPayload) */
function handleAgentEvent(ev) {
  if (!ev || !ev.type) return;
  switch (ev.type) {
    case "assistant_delta": {
      const body = ensureStreamingAssistant();
      state.streamText += ev.text || "";
      body.innerHTML = renderMarkdown(state.streamText);
      scrollMessagesBottom();
      break;
    }
    case "thinking_delta": {
      break;
    }
    case "tool_started": {
      state.streamAssistantEl = null;
      appendToolStart(ev.name, ev.preview);
      break;
    }
    case "tool_finished": {
      finishToolCard(ev.name, ev.output, ev.is_error);
      break;
    }
    case "approval_request": {
      state.pendingApprovalId = ev.id;
      $("approval-text").textContent = `${ev.name}: ${ev.preview}`;
      $("approval-bar").classList.remove("hidden");
      break;
    }
    case "file_changed": {
      $("btn-undo").disabled = ev.undo_count === 0;
      $("btn-redo").disabled = ev.redo_count === 0;
      break;
    }
    case "usage": {
      state.ctxTokens += (ev.input_tokens || 0) + (ev.output_tokens || 0);
      updateContextMeter();
      break;
    }
    case "turn_finished": {
      setBusy(false);
      state.streamAssistantEl = null;
      state.activeToolCard = null;
      $("approval-bar").classList.add("hidden");
      invoke("get_bootstrap").then(syncBootstrapUI).catch(() => {});
      break;
    }
    case "error": {
      setBusy(false);
      state.streamAssistantEl = null;
      appendSystemMessage(`**Error:** ${ev.message || "Unknown error"}`);
      break;
    }
  }
}

/* Full Two-Pane Settings Page */
async function persistSettings(patch = {}) {
  const b = state.boot || {};
  const next = {
    theme: patch.theme ?? b.theme?.id ?? "opencode",
    autoApprove: patch.autoApprove ?? Boolean(b.auto_approve),
    defaultPlanMode: patch.defaultPlanMode ?? Boolean(b.default_plan_mode),
    showToolDetails: patch.showToolDetails ?? (b.show_tool_details !== false),
    showFoxMascot: patch.showFoxMascot ?? (b.show_fox_mascot !== false),
    maxTokens: patch.maxTokens ?? b.max_tokens ?? 16384,
  };
  try {
    const updated = await invoke("save_settings", next);
    syncBootstrapUI(updated);
    setModeUi(updated.default_plan_mode ? "plan" : "build");
  } catch (e) {
    showToast(String(e));
  }
}

async function switchThemeById(themeId) {
  try {
    const css = await invoke("set_theme", { name: themeId });
    applyThemeVars(css);
    if (state.boot) state.boot.theme = css;
    if (!$("settings-view").classList.contains("hidden")) {
      renderSettingsSection();
    }
    showToast(`Theme: ${css.label || themeId}`);
  } catch (e) {
    showToast(String(e));
  }
}

function makeSwitchRow(id, title, desc, checked, onChange) {
  const row = document.createElement("div");
  row.className = "settings-row";
  row.innerHTML = `
    <div>
      <div class="settings-row-title">${escapeHtml(title)}</div>
      <div class="settings-row-desc">${escapeHtml(desc)}</div>
    </div>
    <label class="switch-toggle">
      <input type="checkbox" id="${escapeHtml(id)}" ${checked ? "checked" : ""} />
      <span class="switch-slider"></span>
    </label>
  `;
  row.querySelector("input").onchange = (e) => onChange(e.target.checked);
  return row;
}

function openSettingsView(cat = "general") {
  state.settingsCat = cat;
  $("settings-search").value = "";
  document.querySelectorAll(".settings-cat-btn").forEach((b) => {
    b.classList.toggle("active", b.dataset.cat === cat);
  });
  $("settings-view").classList.remove("hidden");
  renderSettingsSection();
}

function closeSettingsView() {
  $("settings-view").classList.add("hidden");
  $("prompt-input").focus();
}

function renderSettingsSection() {
  const cat = state.settingsCat;
  const q = ($("settings-search").value || "").trim().toLowerCase();
  const b = state.boot || {};
  const meta = {
    general: ["General", "Configure agent execution mode, permissions, and token limits."],
    appearance: ["Appearance", "Themes, animations, and interactive Fox mascot preferences."],
    providers: ["Providers & Models", "Configure AI providers, API keys, endpoints, and models."],
    customizations: ["Customizations", "Manage workspace skills, plugins, custom commands, and MCP servers."],
    shortcuts: ["Shortcuts", "Keyboard shortcuts and slash commands across oxicode."],
  };

  const titleEl = $("settings-section-title");
  const descEl = $("settings-section-desc");
  const body = $("settings-content");
  body.innerHTML = "";

  if (q) {
    titleEl.textContent = "Search Results";
    descEl.textContent = `Showing settings matching "${q}"`;
    renderAllSettingsFiltered(body, q, b);
    return;
  }

  titleEl.textContent = meta[cat]?.[0] || "Settings";
  descEl.textContent = meta[cat]?.[1] || "";

  if (cat === "general") renderSettingsGeneral(body, b);
  else if (cat === "appearance") renderSettingsAppearance(body, b);
  else if (cat === "providers") renderSettingsProviders(body, b);
  else if (cat === "customizations") renderSettingsCustomizations(body, b);
  else if (cat === "shortcuts") renderSettingsShortcuts(body);
}

function renderSettingsGeneral(body, b) {
  const layoutLbl = document.createElement("div");
  layoutLbl.className = "settings-group-label";
  layoutLbl.textContent = "Default Agent Mode";
  body.appendChild(layoutLbl);

  const grid = document.createElement("div");
  grid.className = "settings-card-grid";
  const modes = [
    { key: "build", plan: false, title: "Build Mode", desc: "Full coding agent with file edits, tool execution, and terminal commands." },
    { key: "plan", plan: true, title: "Plan Mode", desc: "Read-only architecture analysis and step-by-step planning before edits." },
  ];
  for (const m of modes) {
    const card = document.createElement("button");
    card.type = "button";
    const active = Boolean(b.default_plan_mode) === m.plan;
    card.className = `layout-choice-card ${active ? "active" : ""}`;
    card.innerHTML = `<div class="layout-choice-title">${escapeHtml(m.title)}</div><div class="layout-choice-desc">${escapeHtml(m.desc)}</div>`;
    card.onclick = () => persistSettings({ defaultPlanMode: m.plan });
    grid.appendChild(card);
  }
  body.appendChild(grid);

  const permLbl = document.createElement("div");
  permLbl.className = "settings-group-label";
  permLbl.textContent = "Execution & Output";
  body.appendChild(permLbl);

  const box = document.createElement("div");
  box.className = "settings-box";
  box.appendChild(
    makeSwitchRow(
      "set-auto-approve",
      "Auto-Approve Tools",
      "Approve file writes and shell tool requests automatically without confirmation prompts.",
      Boolean(b.auto_approve),
      (checked) => persistSettings({ autoApprove: checked })
    )
  );
  box.appendChild(
    makeSwitchRow(
      "set-tool-details",
      "Show Tool Call Details",
      "Display complete stdout/file payloads inside expandable tool execution cards.",
      b.show_tool_details !== false,
      (checked) => persistSettings({ showToolDetails: checked })
    )
  );

  const tokenRow = document.createElement("div");
  tokenRow.className = "settings-row";
  tokenRow.innerHTML = `<div><div class="settings-row-title">Max Response Tokens</div><div class="settings-row-desc">Maximum output length per assistant turn.</div></div>`;
  const tokenSel = document.createElement("select");
  tokenSel.className = "field-select";
  for (const p of TOKEN_PRESETS) {
    const opt = document.createElement("option");
    opt.value = String(p);
    opt.textContent = `${p} tokens`;
    if (p === (b.max_tokens || 16384)) opt.selected = true;
    tokenSel.appendChild(opt);
  }
  tokenSel.onchange = () => persistSettings({ maxTokens: Number(tokenSel.value) });
  tokenRow.appendChild(tokenSel);
  box.appendChild(tokenRow);

  body.appendChild(box);
}

function renderSettingsAppearance(body, b) {
  const mLbl = document.createElement("div");
  mLbl.className = "settings-group-label";
  mLbl.textContent = "Mascot & Motion";
  body.appendChild(mLbl);

  const box = document.createElement("div");
  box.className = "settings-box";
  box.appendChild(
    makeSwitchRow(
      "set-show-fox",
      "Fox Mascot",
      "Display the interactive 3D Fox mascot on the empty welcome screen that follows your cursor & typing.",
      b.show_fox_mascot !== false,
      (checked) => {
        setMascotVisible(checked);
        persistSettings({ showFoxMascot: checked });
      }
    )
  );
  box.appendChild(
    makeSwitchRow(
      "set-animations",
      "Interface Animations",
      "Enable smooth transitions for menus, modals, message entrance, and Settings views.",
      state.animationsEnabled,
      (checked) => {
        applyAnimationPref(checked);
        showToast(checked ? "Animations enabled" : "Reduced motion enabled");
      }
    )
  );
  body.appendChild(box);

  const tLbl = document.createElement("div");
  tLbl.className = "settings-group-label";
  tLbl.textContent = "Theme";
  body.appendChild(tLbl);

  const grid = document.createElement("div");
  grid.className = "theme-grid";
  const activeId = b.theme?.id || "opencode";
  for (const th of b.themes || []) {
    const card = document.createElement("button");
    card.type = "button";
    card.className = `theme-swatch-card ${th.id === activeId ? "active" : ""}`;
    card.dataset.themeId = th.id;
    card.innerHTML = `
      <div class="theme-dots">
        <span class="theme-dot" style="background:${escapeHtml(th.bg)}"></span>
        <span class="theme-dot" style="background:${escapeHtml(th.bg_panel)}"></span>
        <span class="theme-dot" style="background:${escapeHtml(th.accent)}"></span>
        <span class="theme-dot" style="background:${escapeHtml(th.secondary)}"></span>
      </div>
      <div class="theme-swatch-name">${escapeHtml(th.label || th.id)}</div>
    `;
    card.onclick = () => switchThemeById(th.id);
    grid.appendChild(card);
  }
  body.appendChild(grid);
}

function renderSettingsProviders(body, b) {
  const box = document.createElement("div");
  box.className = "settings-box";
  box.style.padding = "16px";
  box.style.display = "flex";
  box.style.flexDirection = "column";
  box.style.gap = "12px";

  const provSel = document.createElement("select");
  provSel.className = "field-select";
  for (const p of b.providers || []) {
    const opt = document.createElement("option");
    opt.value = p.id;
    opt.textContent = `${p.display_name}${p.has_key ? " (connected)" : ""}`;
    if (p.id === b.provider_id) opt.selected = true;
    provSel.appendChild(opt);
  }

  const urlInp = document.createElement("input");
  urlInp.className = "field-input";
  const keyInp = document.createElement("input");
  keyInp.className = "field-input";
  keyInp.type = "password";
  const modelInp = document.createElement("input");
  modelInp.className = "field-input";

  const fillFromProvider = (pid) => {
    const found = (b.providers || []).find((p) => p.id === pid);
    if (found) {
      urlInp.value = found.base_url || "";
      modelInp.value = found.model || "";
      keyInp.placeholder = found.has_key ? `Current key: ${found.masked_key} (leave blank to keep)` : "Paste API key...";
    }
  };
  fillFromProvider(b.provider_id);
  provSel.onchange = () => fillFromProvider(provSel.value);

  const wrap = (lbl, el) => {
    const l = document.createElement("label");
    l.className = "field-label";
    l.textContent = lbl;
    l.appendChild(el);
    return l;
  };

  const actions = document.createElement("div");
  actions.className = "modal-actions";

  const fetchBtn = document.createElement("button");
  fetchBtn.type = "button";
  fetchBtn.className = "btn-secondary";
  fetchBtn.textContent = "Browse Models...";
  fetchBtn.onclick = openModelModal;

  const saveBtn = document.createElement("button");
  saveBtn.type = "button";
  saveBtn.className = "btn-primary";
  saveBtn.textContent = "Save Provider";
  saveBtn.onclick = async () => {
    try {
      const updated = await invoke("save_provider", {
        providerId: provSel.value,
        apiKey: keyInp.value,
        baseUrl: urlInp.value,
        model: modelInp.value,
      });
      syncBootstrapUI(updated);
      showToast(`Saved ${updated.provider_name} (${updated.model})`);
    } catch (e) {
      showToast(String(e));
    }
  };

  actions.append(fetchBtn, saveBtn);
  box.append(wrap("Active Provider", provSel), wrap("Base URL", urlInp), wrap("API Key", keyInp), wrap("Model", modelInp), actions);
  body.appendChild(box);
}

function renderSettingsCustomizations(body, b) {
  const mkGroup = (title, itemsHtml, btnLabel, onBtn) => {
    const lbl = document.createElement("div");
    lbl.className = "section-header-row";
    lbl.innerHTML = `<span class="settings-group-label" style="margin:0">${escapeHtml(title)}</span>`;
    if (btnLabel) {
      const btn = document.createElement("button");
      btn.type = "button";
      btn.className = "mini-btn";
      btn.textContent = btnLabel;
      btn.onclick = onBtn;
      lbl.appendChild(btn);
    }
    body.appendChild(lbl);

    const box = document.createElement("div");
    box.className = "settings-box";
    box.innerHTML = itemsHtml;
    body.appendChild(box);
  };

  mkGroup(
    "Skills",
    (b.skills || []).length
      ? b.skills.map((s) => `<div class="settings-row"><div><div class="settings-row-title">${escapeHtml(s.name)}</div><div class="settings-row-desc">${escapeHtml(s.description)}</div></div><span style="color:var(--muted);font-size:11px">${escapeHtml(s.source)}</span></div>`).join("")
      : `<div class="settings-row"><div class="settings-row-desc">No workspace skills found in .oxicode/skills/</div></div>`,
    "+ New Skill",
    async () => {
      syncBootstrapUI(await invoke("create_starter_skill", { name: "custom-skill" }));
      showToast("Created starter skill");
    }
  );

  const plugBoxHead = document.createElement("div");
  plugBoxHead.className = "section-header-row";
  plugBoxHead.innerHTML = `<span class="settings-group-label" style="margin:0">Plugins</span>`;
  const newPlugBtn = document.createElement("button");
  newPlugBtn.type = "button";
  newPlugBtn.className = "mini-btn";
  newPlugBtn.textContent = "+ New Plugin";
  newPlugBtn.onclick = async () => {
    syncBootstrapUI(await invoke("create_starter_plugin", { name: "custom-plugin" }));
    showToast("Created starter plugin");
  };
  plugBoxHead.appendChild(newPlugBtn);
  body.appendChild(plugBoxHead);

  const plugBox = document.createElement("div");
  plugBox.className = "settings-box";
  if (!(b.plugins || []).length) {
    plugBox.innerHTML = `<div class="settings-row"><div class="settings-row-desc">No plugins found in .oxicode/plugins/</div></div>`;
  } else {
    for (const p of b.plugins) {
      plugBox.appendChild(
        makeSwitchRow(`plug-${p.id}`, `${p.name} (${p.version})`, p.description || "Plugin", p.enabled, async () => {
          syncBootstrapUI(await invoke("toggle_plugin", { pluginId: p.id }));
        })
      );
    }
  }
  body.appendChild(plugBox);

  mkGroup(
    "MCP Servers",
    (b.mcp_servers || []).length
      ? b.mcp_servers.map((m) => `<div class="settings-row"><div><div class="settings-row-title">${escapeHtml(m.name)}</div><div class="settings-row-desc">${escapeHtml(m.error || `${m.tool_count} tools available`)}</div></div><span style="color:${m.error ? "var(--red)" : "var(--green)"};font-size:11px">${m.error ? "error" : "connected"}</span></div>`).join("")
      : `<div class="settings-row"><div class="settings-row-desc">No MCP servers configured</div></div>`,
    "Reload MCP",
    async () => {
      syncBootstrapUI(await invoke("reload_mcp"));
      showToast("MCP servers reloaded");
    }
  );
}

function renderSettingsShortcuts(body) {
  const rows = [
    ["Ctrl + N", "New conversation"],
    ["Ctrl + ,", "Open Settings"],
    ["Tab", "Switch between Build and Plan mode"],
    ["Enter", "Send prompt"],
    ["Shift + Enter", "Insert newline in prompt"],
    ["/", "Open slash actions menu"],
    ["@", "Attach workspace file reference"],
    ["!", "Run inline shell command"],
    ["Esc", "Close Settings, menus, or modals"],
  ];
  const box = document.createElement("div");
  box.className = "settings-box";
  box.innerHTML = rows
    .map(([k, d]) => `<div class="settings-row"><div class="settings-row-title">${escapeHtml(d)}</div><kbd class="shortcut-kbd">${escapeHtml(k)}</kbd></div>`)
    .join("");
  body.appendChild(box);
}

function renderAllSettingsFiltered(body, q, b) {
  const hay = (txt) => txt.toLowerCase().includes(q);
  if (hay("fox mascot animation theme appearance лиса тема анимации")) {
    renderSettingsAppearance(body, b);
  }
  if (hay("mode build plan auto approve tool tokens режим")) {
    renderSettingsGeneral(body, b);
  }
  if (hay("provider model api key url openai openrouter модель провайдер")) {
    renderSettingsProviders(body, b);
  }
  if (hay("skill plugin command mcp extensions плагин скилл")) {
    renderSettingsCustomizations(body, b);
  }
  if (hay("shortcut keybind ctrl tab slash enter клавиши")) {
    renderSettingsShortcuts(body);
  }
  if (!body.children.length) {
    body.innerHTML = `<div class="settings-box"><div class="settings-row"><div class="settings-row-desc">No matching settings for "${escapeHtml(q)}"</div></div></div>`;
  }
}

/* Compact Modals (Model Picker & Directory Switcher) */
function openModal(title, builder) {
  $("modal-title").textContent = title;
  const body = $("modal-body");
  body.innerHTML = "";
  builder(body);
  $("modal-backdrop").classList.remove("hidden");
}

function closeModal() {
  $("modal-backdrop").classList.add("hidden");
  $("prompt-input").focus();
}

async function openModelModal() {
  openModal("Switch Model", (body) => {
    const inp = document.createElement("input");
    inp.className = "field-input";
    inp.value = state.boot?.model || "";
    inp.placeholder = "Type or pick model ID...";
    body.appendChild(inp);

    const list = document.createElement("div");
    list.className = "dir-browser-list";
    list.innerHTML = `<div style="padding:8px;color:var(--muted)">Loading models from provider...</div>`;
    body.appendChild(list);

    const actions = document.createElement("div");
    actions.className = "modal-actions";
    const provBtn = document.createElement("button");
    provBtn.className = "btn-secondary";
    provBtn.type = "button";
    provBtn.textContent = "Provider Settings...";
    provBtn.onclick = () => {
      closeModal();
      openSettingsView("providers");
    };

    const save = document.createElement("button");
    save.className = "btn-primary";
    save.type = "button";
    save.textContent = "Use Model";
    save.onclick = async () => {
      try {
        syncBootstrapUI(await invoke("set_model", { model: inp.value }));
        closeModal();
      } catch (e) {
        showToast(String(e));
      }
    };
    actions.append(provBtn, save);
    body.appendChild(actions);

    invoke("fetch_provider_models")
      .then((models) => {
        list.innerHTML = "";
        for (const m of models) {
          const row = document.createElement("div");
          row.className = "list-row";
          row.textContent = m;
          row.onclick = async () => {
            syncBootstrapUI(await invoke("set_model", { model: m }));
            closeModal();
          };
          list.appendChild(row);
        }
      })
      .catch((e) => {
        list.innerHTML = `<div style="padding:8px;color:var(--muted)">${escapeHtml(String(e))}</div>`;
      });
  });
}

async function openDirModal() {
  let currentPath = state.boot?.work_dir || "";
  openModal("Switch Workspace Directory", async (body) => {
    const pathInp = document.createElement("input");
    pathInp.className = "field-input";
    pathInp.value = currentPath;

    const list = document.createElement("div");
    list.className = "dir-browser-list";

    const loadDir = async (target) => {
      try {
        const res = await invoke("browse_dirs", { path: target || null });
        currentPath = res.current;
        pathInp.value = res.current;
        list.innerHTML = "";
        if (res.parent) {
          const up = document.createElement("div");
          up.className = "list-row";
          up.innerHTML = `<span>..</span><span style="color:var(--muted)">Parent folder</span>`;
          up.onclick = () => loadDir(res.parent);
          list.appendChild(up);
        }
        for (const fullDir of res.dirs || []) {
          const parts = String(fullDir).split(/[/\\]/).filter(Boolean);
          const name = parts[parts.length - 1] || fullDir;
          const row = document.createElement("div");
          row.className = "list-row";
          row.textContent = name;
          row.onclick = () => loadDir(fullDir);
          list.appendChild(row);
        }
      } catch (e) {
        list.innerHTML = `<div style="padding:8px;color:var(--red)">${escapeHtml(String(e))}</div>`;
      }
    };

    const navRow = document.createElement("div");
    navRow.style.display = "flex";
    navRow.style.gap = "8px";
    const goBtn = document.createElement("button");
    goBtn.className = "btn-secondary";
    goBtn.type = "button";
    goBtn.textContent = "Browse";
    goBtn.onclick = () => loadDir(pathInp.value);
    navRow.append(pathInp, goBtn);

    const actions = document.createElement("div");
    actions.className = "modal-actions";
    const selectBtn = document.createElement("button");
    selectBtn.className = "btn-primary";
    selectBtn.type = "button";
    selectBtn.textContent = "Open Workspace";
    selectBtn.onclick = async () => {
      try {
        const boot = await invoke("set_work_dir", { path: pathInp.value });
        syncBootstrapUI(boot);
        closeModal();
        showToast(`Workspace: ${boot.work_dir}`);
      } catch (e) {
        showToast(String(e));
      }
    };
    actions.appendChild(selectBtn);

    body.append(navRow, list, actions);
    loadDir(currentPath);
  });
}

/* Sending Prompts */
async function sendCurrentPrompt() {
  const inp = $("prompt-input");
  const val = inp.value;
  const trimmed = val.trim();
  if (!trimmed || state.busy) return;
  hideAutocomplete();
  closePlusMenu();

  const lower = trimmed.toLowerCase();
  if (lower === "/new") {
    inp.value = "";
    return $("btn-new-task").click();
  }
  if (lower === "/connect") {
    inp.value = "";
    return openSettingsView("providers");
  }
  if (lower === "/models" || lower === "/model") {
    inp.value = "";
    return openModelModal();
  }
  if (lower === "/settings" || lower === "/config") {
    inp.value = "";
    return openSettingsView("general");
  }
  if (lower === "/themes" || lower === "/theme") {
    inp.value = "";
    return openSettingsView("appearance");
  }
  if (lower === "/dir" || lower === "/cd") {
    inp.value = "";
    return openDirModal();
  }
  if (lower === "/undo") {
    inp.value = "";
    return $("btn-undo").click();
  }
  if (lower === "/redo") {
    inp.value = "";
    return $("btn-redo").click();
  }
  if (lower === "/compact") {
    inp.value = "";
    return $("btn-compact").click();
  }
  if (lower === "/export") {
    inp.value = "";
    return $("btn-export").click();
  }
  if (lower === "/clear") {
    inp.value = "";
    resetToEmptyView();
    return;
  }

  if (trimmed.startsWith("!")) {
    const command = trimmed.slice(1).trim();
    inp.value = "";
    appendUserMessage(trimmed);
    try {
      const out = await invoke("run_inline_shell", { command });
      appendSystemMessage("```sh\n" + out + "\n```");
    } catch (e) {
      appendSystemMessage(`**Shell error:** ${String(e)}`);
    }
    return;
  }

  inp.value = "";
  inp.style.height = "auto";
  if ($("task-title").textContent === "New Conversation") {
    $("task-title").textContent = trimmed.slice(0, 48);
  }
  appendUserMessage(trimmed);
  setBusy(true);
  try {
    await invoke("send_message", { prompt: trimmed, mode: state.mode });
  } catch (e) {
    setBusy(false);
    appendSystemMessage(`**Error:** ${String(e)}`);
  }
}

/* Exact Wrapped Caret Coordinates + Expressive Interactive Fox Mascot */
function createCaretMeasurer(textarea) {
  const mirror = document.createElement("div");
  mirror.style.cssText =
    "position:fixed;left:-9999px;top:0;visibility:hidden;pointer-events:none;white-space:pre-wrap;word-break:break-word;overflow-wrap:break-word;box-sizing:border-box;";
  document.body.appendChild(mirror);

  return () => {
    const r = textarea.getBoundingClientRect();
    if (!r.width || !r.height) return null;
    const cs = getComputedStyle(textarea);
    mirror.style.width = `${r.width}px`;
    mirror.style.padding = cs.padding;
    mirror.style.fontFamily = cs.fontFamily;
    mirror.style.fontSize = cs.fontSize;
    mirror.style.lineHeight = cs.lineHeight;

    const pos = textarea.selectionStart ?? textarea.value.length;
    mirror.textContent = textarea.value.slice(0, pos);
    const probe = document.createElement("span");
    probe.textContent = "\u200b";
    mirror.appendChild(probe);

    const mr = mirror.getBoundingClientRect();
    const pr = probe.getBoundingClientRect();
    const relX = pr.left - mr.left;
    const relY = Math.min(r.height - 8, pr.top - mr.top - textarea.scrollTop + 8);
    return { clientX: r.left + relX, clientY: r.top + relY };
  };
}

function startFoxAnimation() {
  const canvas = $("welcome-fox");
  if (!canvas) return;
  const ctx = canvas.getContext("2d");
  if (!ctx) return;

  const LOGICAL_W = 176;
  const LOGICAL_H = 156;
  const dpr = Math.min(window.devicePixelRatio || 1, 2.5);
  canvas.width = Math.round(LOGICAL_W * dpr);
  canvas.height = Math.round(LOGICAL_H * dpr);
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.lineJoin = "round";
  ctx.lineCap = "round";

  let curYaw = 0;
  let curPitch = 0;
  let earTwitch = 0;

  window.addEventListener("pointermove", (e) => {
    const dx = state.lastMouse.x === null ? 99 : Math.hypot(e.clientX - state.lastMouse.x, e.clientY - state.lastMouse.y);
    state.lastMouse = { x: e.clientX, y: e.clientY };
    if (dx > 5 || performance.now() > state.foxGaze.until) {
      state.foxGaze = {
        clientX: e.clientX,
        clientY: e.clientY,
        source: "mouse",
        until: performance.now() + 3600,
      };
    }
  });

  window.addEventListener("mouseleave", () => {
    state.foxGaze = { clientX: null, clientY: null, source: "idle", until: 0 };
  });

  const inp = $("prompt-input");
  const measureCaret = createCaretMeasurer(inp);
  const trackCaret = () => {
    const pt = measureCaret();
    if (pt) {
      state.foxGaze = {
        clientX: pt.clientX,
        clientY: pt.clientY,
        source: "caret",
        until: performance.now() + 4500,
      };
      earTwitch = Math.min(1, earTwitch + 0.35);
    }
  };
  ["input", "keyup", "click", "focus"].forEach((ev) => inp.addEventListener(ev, trackCaret));

  const pts = [
    [-0.78, 1.06, -0.06], // 0 Left ear tip
    [0.78, 1.06, -0.06],  // 1 Right ear tip
    [-0.92, 0.34, 0.08],  // 2 Left ear outer base
    [0.92, 0.34, 0.08],   // 3 Right ear outer base
    [-0.36, 0.62, 0.2],   // 4 Left ear inner base
    [0.36, 0.62, 0.2],    // 5 Right ear inner base
    [0, 0.68, 0.34],      // 6 Crown center
    [-0.98, -0.14, 0.14], // 7 Left cheek outer
    [0.98, -0.14, 0.14],  // 8 Right cheek outer
    [-0.58, -0.68, 0.3],  // 9 Left jaw
    [0.58, -0.68, 0.3],   // 10 Right jaw
    [0, -0.24, 1.08],     // 11 Snout bridge (deeper projection makes turns very clear)
    [0, -0.8, 0.78],      // 12 Chin
    [-0.7, 0.78, 0.05],   // 13 Left inner ear peak
    [0.7, 0.78, 0.05],    // 14 Right inner ear peak
  ];

  const faces = [
    [0, 4, 2, [212, 98, 46]],     // Left outer ear
    [1, 3, 5, [212, 98, 46]],     // Right outer ear
    [13, 4, 2, [250, 210, 180]],  // Left inner ear
    [14, 3, 5, [250, 210, 180]],  // Right inner ear
    [4, 6, 11, [240, 128, 70]],   // Left crown
    [5, 11, 6, [240, 128, 70]],   // Right crown
    [4, 11, 7, [230, 115, 58]],   // Left temple
    [5, 8, 11, [230, 115, 58]],   // Right temple
    [2, 4, 7, [214, 100, 46]],    // Left side cheek
    [3, 8, 5, [214, 100, 46]],    // Right side cheek
    [7, 11, 9, [249, 239, 226]],  // Left cream cheek
    [8, 10, 11, [249, 239, 226]], // Right cream cheek
    [9, 11, 12, [239, 227, 212]], // Left muzzle
    [10, 12, 11, [239, 227, 212]],// Right muzzle
  ];

  let blinkNextAt = performance.now() + 3400;

  const frame = (now) => {
    const cr = canvas.getBoundingClientRect();
    let goalYaw = 0;
    let goalPitch = 0;

    if (cr.width > 0 && state.foxGaze.clientX !== null && now < state.foxGaze.until) {
      const cx = cr.left + cr.width * 0.5;
      const cy = cr.top + cr.height * 0.52;
      goalYaw = Math.max(-1, Math.min(1, (state.foxGaze.clientX - cx) / 210));
      goalPitch = Math.max(-1, Math.min(1, (state.foxGaze.clientY - cy) / 180));
    } else if (document.activeElement === inp) {
      const pt = measureCaret();
      if (pt && cr.width > 0) {
        const cx = cr.left + cr.width * 0.5;
        const cy = cr.top + cr.height * 0.52;
        goalYaw = Math.max(-1, Math.min(1, (pt.clientX - cx) / 210));
        goalPitch = Math.max(-1, Math.min(1, (pt.clientY - cy) / 180));
      }
    }

    const speed = state.animationsEnabled ? 0.14 : 0.45;
    curYaw += (goalYaw - curYaw) * speed;
    curPitch += (goalPitch - curPitch) * speed;
    earTwitch *= 0.9;

    ctx.clearRect(0, 0, LOGICAL_W, LOGICAL_H);

    if (!$("welcome-fox-wrap").classList.contains("hidden")) {
      // Generous natural head rotation (39 deg yaw, 30 deg pitch) so direction is unmistakable
      const yaw = curYaw * 0.68;
      const pitch = curPitch * 0.5 + (state.animationsEnabled ? Math.sin(now * 0.002) * 0.025 : 0);
      const bob = state.animationsEnabled ? Math.sin(now * 0.0024) * 2.1 : 0;

      const cy = Math.cos(yaw);
      const sy = Math.sin(yaw);
      const cp = Math.cos(pitch);
      const sp = Math.sin(pitch);

      const proj = (x, y, z) => {
        const rx = x * cy + z * sy;
        const rz = -x * sy + z * cy;
        const ry = y * cp - rz * sp;
        const fz = y * sp + rz * cp;
        const s = 56 / (3.1 - fz * 0.48);
        return [LOGICAL_W * 0.5 + rx * s * 2.15, LOGICAL_H * 0.52 - ry * s * 2.0 + bob, fz];
      };

      // Soft ambient shadow
      ctx.fillStyle = "rgba(0, 0, 0, 0.28)";
      ctx.beginPath();
      ctx.ellipse(LOGICAL_W * 0.5 + yaw * 12, LOGICAL_H - 13, 38, 7, 0, 0, Math.PI * 2);
      ctx.fill();

      const dynamicPts = pts.map((p, idx) => {
        if (idx <= 1) return [p[0], p[1] + earTwitch * 0.06, p[2]];
        return p;
      });
      const tPts = dynamicPts.map(([x, y, z]) => proj(x, y, z));
      const drawn = [];

      for (const f of faces) {
        const [a, b, c, base] = f;
        const pa = dynamicPts[a];
        const pb = dynamicPts[b];
        const pc = dynamicPts[c];
        const ux = pb[0] - pa[0];
        const uy = pb[1] - pa[1];
        const uz = pb[2] - pa[2];
        const vx = pc[0] - pa[0];
        const vy = pc[1] - pa[1];
        const vz = pc[2] - pa[2];
        let nx = uy * vz - uz * vy;
        let ny = uz * vx - ux * vz;
        let nz = ux * vy - uy * vx;
        const nl = Math.hypot(nx, ny, nz) || 1;
        nx /= nl;
        ny /= nl;
        nz /= nl;

        const tp0 = tPts[a];
        const tp1 = tPts[b];
        const tp2 = tPts[c];
        const depth = (tp0[2] + tp1[2] + tp2[2]) / 3;

        const rnx = nx * cy + nz * sy;
        const rnz = -nx * sy + nz * cy;
        const rny = ny * cp - rnz * sp;
        const rnz2 = ny * sp + rnz * cp;
        const light = Math.max(0.7, Math.min(1.14, 0.8 + (rnx * -0.25 + rny * 0.42 + rnz2 * 0.58) * 0.36));

        const r = Math.min(255, Math.round(base[0] * light));
        const g = Math.min(255, Math.round(base[1] * light));
        const bl = Math.min(255, Math.round(base[2] * light));

        drawn.push({ depth, tp0, tp1, tp2, color: `rgb(${r},${g},${bl})` });
      }

      drawn.sort((a, b) => a.depth - b.depth);
      for (const d of drawn) {
        ctx.fillStyle = d.color;
        ctx.strokeStyle = d.color;
        ctx.lineWidth = 0.9;
        ctx.beginPath();
        ctx.moveTo(d.tp0[0], d.tp0[1]);
        ctx.lineTo(d.tp1[0], d.tp1[1]);
        ctx.lineTo(d.tp2[0], d.tp2[1]);
        ctx.closePath();
        ctx.fill();
        ctx.stroke();
      }

      // Expressive eyes with warm cream sclera & actively darting pupils
      if (now > blinkNextAt + 140) {
        blinkNextAt = now + 3500 + Math.random() * 2600;
      }
      const isBlinking = state.animationsEnabled && now >= blinkNextAt && now <= blinkNextAt + 140;

      for (const sx of [-0.36, 0.36]) {
        const [ex, ey] = proj(sx, 0.05, 0.65);
        const foreshorten = Math.max(0.55, 1 - Math.abs(yaw) * 0.42);
        const rx = 6.2 * foreshorten;
        const ry = 6.8;

        if (isBlinking) {
          ctx.strokeStyle = "#221e26";
          ctx.lineWidth = 2.2;
          ctx.beginPath();
          ctx.moveTo(ex - rx, ey);
          ctx.quadraticCurveTo(ex, ey + 2.6, ex + rx, ey);
          ctx.stroke();
        } else {
          // Cream sclera so eye direction pops unmistakably
          ctx.fillStyle = "#fbf4ea";
          ctx.beginPath();
          ctx.ellipse(ex, ey, rx, ry, sx < 0 ? 0.12 : -0.12, 0, Math.PI * 2);
          ctx.fill();

          // Dark iris/pupil shifting cleanly inside sclera toward curYaw & curPitch
          const px = ex + Math.max(-2.7, Math.min(2.7, curYaw * 3.1)) * foreshorten;
          const py = ey + Math.max(-2.3, Math.min(2.7, curPitch * 2.9));
          ctx.fillStyle = "#191922";
          ctx.beginPath();
          ctx.ellipse(px, py, 3.8 * foreshorten, 4.3, 0, 0, Math.PI * 2);
          ctx.fill();

          // Specular highlight
          ctx.fillStyle = "rgba(255, 255, 255, 0.95)";
          ctx.beginPath();
          ctx.ellipse(px - 1.1 * foreshorten, py - 1.4, 1.3 * foreshorten, 1.3, 0, 0, Math.PI * 2);
          ctx.fill();
        }
      }

      // Snout tip nose
      const [nx, ny] = proj(0, -0.35, 1.02);
      ctx.fillStyle = "#201d25";
      ctx.beginPath();
      ctx.ellipse(nx, ny, 5.8, 4.2, 0, 0, Math.PI * 2);
      ctx.fill();
      ctx.fillStyle = "rgba(255, 255, 255, 0.25)";
      ctx.beginPath();
      ctx.ellipse(nx - 1.4, ny - 1.2, 1.7, 0.9, -0.2, 0, Math.PI * 2);
      ctx.fill();
    }
    requestAnimationFrame(frame);
  };

  requestAnimationFrame(frame);
}

/* Synchronous Event Wiring */
function bindUiEvents() {
  $("btn-toggle-left").onclick = () => $("left-sidebar").classList.toggle("collapsed");
  $("btn-toggle-right").onclick = () => $("right-sidebar").classList.toggle("collapsed");
  $("btn-nav-customizations").onclick = () => openInspectorTab("extensions");

  document.querySelectorAll(".tab-btn").forEach((btn) => {
    btn.onclick = () => openInspectorTab(btn.dataset.tab);
  });

  document.querySelectorAll(".mode-btn").forEach((btn) => {
    btn.onclick = () => setModeUi(btn.dataset.mode);
  });

  $("chk-auto-approve").onchange = (e) => {
    invoke("set_auto_approve", { enabled: e.target.checked }).catch((err) => showToast(String(err)));
  };

  const startNewTask = async () => {
    state.activeSessionId = null;
    resetToEmptyView();
    try {
      syncBootstrapUI(await invoke("new_session"));
    } catch (e) {
      showToast(String(e));
    }
  };
  $("btn-new-task").onclick = startNewTask;
  $("btn-top-new").onclick = startNewTask;

  // Antigravity (+) Add Context Popover
  $("btn-plus-menu").onclick = (e) => {
    e.stopPropagation();
    togglePlusMenu();
  };
  document.querySelectorAll(".plus-menu-item").forEach((btn) => {
    btn.onclick = () => {
      const action = btn.dataset.plus;
      closePlusMenu();
      const inp = $("prompt-input");
      if (action === "files") {
        inp.value += (inp.value && !inp.value.endsWith(" ") ? " " : "") + "@";
        inp.focus();
        updateAutocomplete();
      } else if (action === "folder") {
        openDirModal();
      } else if (action === "actions") {
        inp.value = "/";
        inp.focus();
        updateAutocomplete();
      } else if (action === "shell") {
        inp.value = "! ";
        inp.focus();
      } else if (action === "customizations") {
        openSettingsView("customizations");
      }
    };
  });
  document.addEventListener("click", (e) => {
    if (!e.target.closest("#plus-menu") && !e.target.closest("#btn-plus-menu")) {
      closePlusMenu();
    }
  });

  $("btn-send").onclick = sendCurrentPrompt;
  $("btn-stop").onclick = () => invoke("stop_generation").catch(() => {});

  $("btn-approve-once").onclick = () => {
    $("approval-bar").classList.add("hidden");
    invoke("respond_approval", { id: state.pendingApprovalId, decision: "approve_once" }).catch((e) => showToast(String(e)));
  };
  $("btn-approve-all").onclick = () => {
    $("approval-bar").classList.add("hidden");
    $("chk-auto-approve").checked = true;
    invoke("respond_approval", { id: state.pendingApprovalId, decision: "approve_all" }).catch((e) => showToast(String(e)));
  };
  $("btn-reject").onclick = () => {
    $("approval-bar").classList.add("hidden");
    invoke("respond_approval", { id: state.pendingApprovalId, decision: "reject" }).catch((e) => showToast(String(e)));
  };

  $("btn-undo").onclick = async () => {
    try {
      syncBootstrapUI(await invoke("undo_change"));
      showToast("Reverted last file edit");
    } catch (e) {
      showToast(String(e));
    }
  };
  $("btn-redo").onclick = async () => {
    try {
      syncBootstrapUI(await invoke("redo_change"));
      showToast("Reapplied file edit");
    } catch (e) {
      showToast(String(e));
    }
  };
  $("btn-compact").onclick = async () => {
    try {
      setBusy(true);
      const summary = await invoke("compact_chat");
      $("messages-container").innerHTML = "";
      appendSystemMessage(`Context compacted:\n${summary}`);
    } catch (e) {
      showToast(String(e));
    } finally {
      setBusy(false);
    }
  };
  $("btn-export").onclick = async () => {
    try {
      const p = await invoke("export_chat");
      showToast(`Exported: ${p}`);
    } catch (e) {
      showToast(String(e));
    }
  };

  // Settings & modals
  $("btn-open-settings").onclick = () => openSettingsView("general");
  $("btn-open-themes").onclick = () => openSettingsView("appearance");
  $("btn-open-connect").onclick = () => openSettingsView("providers");
  $("btn-settings-back").onclick = closeSettingsView;
  $("btn-settings-close").onclick = closeSettingsView;
  $("settings-search").oninput = renderSettingsSection;
  document.querySelectorAll(".settings-cat-btn").forEach((btn) => {
    btn.onclick = () => {
      state.settingsCat = btn.dataset.cat;
      $("settings-search").value = "";
      document.querySelectorAll(".settings-cat-btn").forEach((b) => b.classList.remove("active"));
      btn.classList.add("active");
      renderSettingsSection();
    };
  });

  $("btn-model-pill").onclick = openModelModal;
  $("btn-open-dir").onclick = openDirModal;
  $("btn-project-row").onclick = openDirModal;
  $("btn-footer-dir").onclick = openDirModal;
  $("work-dir-path").onclick = openDirModal;
  $("modal-close").onclick = closeModal;
  $("modal-backdrop").onclick = (e) => {
    if (e.target === $("modal-backdrop")) closeModal();
  };

  $("file-filter").oninput = renderFilesList;
  $("btn-new-skill").onclick = async () => {
    try {
      syncBootstrapUI(await invoke("create_starter_skill", { name: "custom-skill" }));
      showToast("Created .oxicode/skills/custom-skill/SKILL.md");
    } catch (e) {
      showToast(String(e));
    }
  };
  $("btn-new-plugin").onclick = async () => {
    try {
      syncBootstrapUI(await invoke("create_starter_plugin", { name: "custom-plugin" }));
      showToast("Created .oxicode/plugins/custom-plugin/plugin.toml");
    } catch (e) {
      showToast(String(e));
    }
  };
  $("btn-reload-mcp").onclick = async () => {
    try {
      syncBootstrapUI(await invoke("reload_mcp"));
      showToast("MCP servers reloaded");
    } catch (e) {
      showToast(String(e));
    }
  };

  const inp = $("prompt-input");
  const resizeComposer = () => {
    inp.style.height = "auto";
    inp.style.height = `${Math.min(180, Math.max(44, inp.scrollHeight))}px`;
  };
  inp.oninput = () => {
    resizeComposer();
    updateAutocomplete();
  };
  inp.onkeydown = (e) => {
    if (state.acItems.length > 0) {
      if (e.key === "ArrowDown") {
        e.preventDefault();
        state.acIndex = (state.acIndex + 1) % state.acItems.length;
        updateAutocomplete();
        return;
      }
      if (e.key === "ArrowUp") {
        e.preventDefault();
        state.acIndex = (state.acIndex - 1 + state.acItems.length) % state.acItems.length;
        updateAutocomplete();
        return;
      }
      if (e.key === "Tab" || (e.key === "Enter" && !e.shiftKey)) {
        e.preventDefault();
        applyAutocomplete(state.acIndex);
        return;
      }
      if (e.key === "Escape") {
        hideAutocomplete();
        return;
      }
    } else if (e.key === "Tab" && !e.shiftKey) {
      e.preventDefault();
      setModeUi(state.mode === "build" ? "plan" : "build");
      return;
    }
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      sendCurrentPrompt();
    }
  };

  window.addEventListener("keydown", (e) => {
    if (e.key === "Escape") {
      if (!$("modal-backdrop").classList.contains("hidden")) closeModal();
      else if (!$("plus-menu").classList.contains("hidden")) closePlusMenu();
      else if (!$("settings-view").classList.contains("hidden")) closeSettingsView();
    } else if (e.ctrlKey && e.key === ",") {
      e.preventDefault();
      openSettingsView("general");
    } else if (e.ctrlKey && e.key.toLowerCase() === "n") {
      e.preventDefault();
      $("btn-new-task").click();
    }
  });

  $("messages-container").onclick = async (e) => {
    const btn = e.target.closest(".copy-code-btn");
    if (btn) {
      await navigator.clipboard.writeText(decodeURIComponent(btn.dataset.code));
      showToast("Code copied");
    }
  };
}

/* Init */
window.addEventListener("DOMContentLoaded", async () => {
  applyAnimationPref(state.animationsEnabled);
  bindUiEvents();
  try {
    startFoxAnimation();
  } catch (e) {
    console.error("Fox canvas error:", e);
  }

  $("prompt-input").focus();

  try {
    await listen("agent-event", (event) => handleAgentEvent(event.payload));
    const initBoot = await invoke("get_bootstrap");
    syncBootstrapUI(initBoot);
    setModeUi(initBoot.default_plan_mode ? "plan" : "build");
  } catch (e) {
    console.error("Bootstrap IPC error:", e);
    showToast(`IPC init error: ${String(e)}`);
  }
});
