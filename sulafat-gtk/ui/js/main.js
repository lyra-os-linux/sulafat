// Sulafat — cliente SSH em Tauri, no padrão do Vega. Paridade com a antiga
// interface GTK: hosts do ~/.ssh/config agrupados, sessões em abas com o
// `ssh` num terminal, conexão rápida, edição de host (opções avançadas e
// hosts somente leitura), arquivos via sftp://, backup e ações do sistema.

import { call, on, win } from "./api.js";
import { svg, registerIcons } from "./icons.js";
import { loadCatalog, tr, trf } from "./i18n.js";
import { h, icon, clear, normalize, button, iconButton, badge, list, banner, toast, confirm, sheet, popover, menuItem, fail } from "./ui.js";
import { Session, parseFont } from "./terminal.js";

registerIcons({
  sulafat: '<path d="m4 17 6-6-6-6"/><path d="M12 19h8"/><circle cx="19" cy="5" r="2" fill="currentColor"/>',
  bolt: '<path d="M4 14a1 1 0 0 1-.78-1.63l9.9-10.2a.5.5 0 0 1 .86.46l-1.92 6.02A1 1 0 0 0 13 10h7a1 1 0 0 1 .78 1.63l-9.9 10.2a.5.5 0 0 1-.86-.46l1.92-6.02A1 1 0 0 0 11 14z"/>',
  file: '<path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z"/><path d="M14 2v4a2 2 0 0 0 2 2h4"/>',
  "rotate-ccw": '<path d="M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8"/><path d="M3 3v5h5"/>',
  "log-out": '<path d="M9 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h4"/><path d="m16 17 5-5-5-5"/><path d="M21 12H9"/>',
});

const $ = (id) => document.getElementById(id);
const GROUPLESS = "\u0000";
const SWATCHES = ["#3584e4", "#2190a4", "#26a269", "#c88800", "#e66100", "#e01b24", "#d56199", "#9141ac", "#6f8396"];

let filter = "";
const collapsed = new Set();
let hosts = []; // { host, meta, connectable }
let groups = [];
let loadError = null;
let settings = { font: null, scrollback_lines: 10000 };
let systemFont = "";
let appInfo = { version: "" };
/** @type {Session[]} */
let tabs = [];
let current = null;
const earlyStates = new Map();

// ---------- Tema (segue o GNOME) ----------

const ACCENTS = {
  blue: "#3584e4", teal: "#2190a4", green: "#3a944a", yellow: "#c88800", orange: "#ed5b00",
  red: "#e62d42", pink: "#d56199", purple: "#9141ac", slate: "#6f8396",
};
function applyInterface({ color_scheme, accent, monospace_font } = {}) {
  const dark = color_scheme === "prefer-dark";
  document.documentElement.dataset.theme = dark ? "dark" : "light";
  let color = ACCENTS[accent] ?? ACCENTS.blue;
  if (dark) color = `color-mix(in srgb, ${color} 78%, #fff)`;
  document.documentElement.style.setProperty("--accent", color);
  if (monospace_font !== undefined && monospace_font !== systemFont) {
    systemFont = monospace_font;
    applySettingsToTabs();
  }
}

const terminalFont = () => settings.font || systemFont || "Monospace 11";
const addr = ({ host }) => [host.user ? `${host.user}@${host.host_name ?? ""}` : host.host_name ?? "", host.port ? `:${host.port}` : ""].join("").replace(/@$/, "");
const tabOf = (alias) => tabs.find((tb) => tb.alias === alias);
const connected = (alias) => !!tabOf(alias)?.running;
const currentTab = () => tabs.find((tb) => tb === current) ?? null;

// ---------- Hosts ----------

async function load() {
  try { applyHosts(await call("hosts_list")); } catch (error) { fail(error); }
}

function applyHosts(data) {
  hosts = data.hosts;
  groups = data.groups;
  loadError = data.load_error;
  renderHosts();
  renderTabs();
}

function hostMenu(anchor, entry) {
  const { host, connectable } = entry;
  const live = connected(host.alias);
  const pop = popover(anchor, [
    connectable && (live
      ? menuItem("power", tr("Disconnect"), () => { pop.close(); closeTab(tabOf(host.alias)); })
      : menuItem("terminal", tr("Connect"), () => { pop.close(); connect(entry); }, "Enter")),
    !host.read_only && menuItem("cog", tr("Edit"), () => { pop.close(); openHost(entry); }, "F2"),
    !host.read_only && menuItem("copy", tr("Duplicate"), () => { pop.close(); duplicate(entry); }),
    connectable && menuItem("folder", tr("Open files"), () => { pop.close(); openFiles(host.alias); }),
    !host.read_only && h("hr"),
    !host.read_only && menuItem("trash", tr("Delete"), () => { pop.close(); removeHost(entry); }, "Del"),
  ].filter(Boolean), "menu");
}

function hostRow(entry) {
  const { host, meta, connectable } = entry;
  const live = connected(host.alias);
  const active = currentTab()?.alias === host.alias;
  return h("div", {
    class: `host ${active ? "active" : ""}`.trim(), tabindex: "0", role: "listitem",
    style: { "--hc": meta.color ?? "transparent" }, title: meta.notes ?? "",
    ondblclick: () => connectable && connect(entry),
    onkeydown: (e) => {
      if (e.target !== e.currentTarget) return;
      if (e.key === "Enter" && connectable) { e.preventDefault(); connect(entry); }
      else if (e.key === "F2" && !host.read_only) { e.preventDefault(); openHost(entry); }
      else if (e.key === "Delete" && !host.read_only) { e.preventDefault(); removeHost(entry); }
      else if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        e.preventDefault();
        const all = [...document.querySelectorAll(".host")];
        all[all.indexOf(e.currentTarget) + (e.key === "ArrowDown" ? 1 : -1)]?.focus();
      }
    },
    oncontextmenu: (e) => { e.preventDefault(); hostMenu(e.currentTarget.querySelector(".more"), entry); },
  },
    h("span", { class: `farol ${live ? "on" : ""}`.trim(), title: tr("Connection status"), "aria-label": live ? tr("Connected") : tr("Not connected") }),
    h("div", { class: "body" },
      h("div", { class: "alias" }, h("span", {}, host.alias), host.read_only && badge(tr("read-only"), "ro outline")),
      h("div", { class: "addr" }, connectable ? addr(entry) || `ssh ${host.alias}` : tr("Template for several hosts"))),
    h("div", { class: "acts" },
      connectable && iconButton(live ? "external" : "terminal", live ? tr("Show session") : tr("Connect"), (e) => { e.stopPropagation(); connect(entry); }, "sm"),
      h("button", { class: "icon-btn sm more", "aria-label": tr("Host actions"), title: tr("Host actions"), onclick: (e) => { e.stopPropagation(); hostMenu(e.currentTarget, entry); } }, icon("more", 16))));
}

function renderHosts() {
  const q = normalize(filter);
  const shown = hosts.filter(({ host, meta }) => !q || [host.alias, host.host_name, host.user, meta.group, meta.notes].some((v) => v && normalize(v).includes(q)));
  const byGroup = new Map();
  for (const entry of shown) {
    const g = entry.meta.group ?? GROUPLESS;
    if (!byGroup.has(g)) byGroup.set(g, []);
    byGroup.get(g).push(entry);
  }
  const ordered = [...byGroup].sort(([a], [b]) => (a === GROUPLESS) - (b === GROUPLESS) || a.localeCompare(b));
  for (const [, items] of ordered) items.sort((a, b) => a.host.alias.toLowerCase().localeCompare(b.host.alias.toLowerCase()));
  clear($("hosts"), shown.length === 0
    ? h("div", { class: "empty", style: { padding: "36px 12px" } }, h("p", {}, q ? trf("No host matches “{query}”.", { query: filter }) : tr("No hosts in ~/.ssh/config yet.")))
    : ordered.map(([g, items]) => {
      const open = !collapsed.has(g);
      return h("div", { class: "host-group" },
        h("button", { "aria-expanded": String(open), onclick: () => { open ? collapsed.add(g) : collapsed.delete(g); renderHosts(); } },
          icon("chevron-down", 14), g === GROUPLESS ? tr("No group") : g, h("span", { class: "n" }, String(items.length))),
        open && h("div", { role: "list", "aria-label": g === GROUPLESS ? tr("No group") : g }, items.map(hostRow)));
    }));
  clear($("config-chip"), h("span", { class: "lead" }, icon(loadError ? "alert" : "file", 17)),
    h("div", {}, h("strong", {}, "~/.ssh/config"),
      h("span", {}, loadError ? tr("Read-only: the file could not be loaded") : trf("{count} hosts", { count: hosts.length }))));
  $("config-chip").classList.toggle("bad", !!loadError);
  clear($("load-error"), loadError && h("div", { style: { padding: "0 10px 8px" } },
    banner("danger", "alert-circle", tr("Could not load SSH configuration"), `${loadError} ${tr("Sulafat is read-only until the configuration can be loaded.")}`,
      button(tr("Restore latest backup"), { size: "sm", onClick: restoreBackup }))));
  $("add-btn").disabled = !!loadError;
}

async function duplicate(entry) {
  try {
    const alias = await call("host_duplicate", { alias: entry.host.alias });
    await load();
    const copy = hosts.find((x) => x.host.alias === alias);
    if (copy) openHost(copy);
  } catch (error) { fail(error); }
}

async function removeHost(entry) {
  const alias = entry.host.alias;
  const ok = await confirm({
    title: tr("Delete host?"), text: trf("Delete “{alias}” from ~/.ssh/config? Three backup generations are kept.", { alias }),
    ic: "trash", tone: "danger", confirmLabel: tr("Delete"), confirmKind: "danger",
    facts: [{ icon: "key", text: tr("Keys and known_hosts are not touched.") }],
  });
  if (!ok) return;
  try { await call("host_delete", { alias }); toast(tr("Host deleted.")); load(); } catch (error) { fail(error); }
}

async function openFiles(alias) {
  try { await call("open_files", { alias }); } catch (error) { fail(error); }
}

// ---------- Diálogo de host ----------

function openHost(entry) {
  const existing = entry?.host ?? null;
  const hst = existing ?? { alias: "", host_name: null, user: null, port: null, identity_file: null, proxy_jump: null, extra: "" };
  const m = entry?.meta ?? {};
  const input = (value, attrs = {}) => { const el = h("input", { class: "input", spellcheck: "false", ...attrs }); el.value = value ?? ""; return el; };
  const alias = input(hst.alias, { "aria-label": tr("Alias"), placeholder: "web01" });
  const hostName = input(hst.host_name, { "aria-label": "HostName", placeholder: "web01.example.com" });
  const user = input(hst.user, { "aria-label": tr("User") });
  const port = input(hst.port ?? "", { type: "number", min: "1", max: "65535", placeholder: "22", "aria-label": tr("Port") });
  const proxyJump = input(hst.proxy_jump, { "aria-label": "ProxyJump" });
  const key = input(hst.identity_file, { "aria-label": tr("Identity file (private key)") });
  const group = h("select", { class: "input", "aria-label": tr("Group") },
    h("option", { value: "" }, tr("None")), groups.map((g) => h("option", { value: g, selected: g === m.group }, g)), h("option", { value: "+new" }, tr("New group…")));
  const newGroup = input("", { "aria-label": tr("New group name"), placeholder: tr("New group name"), hidden: true });
  group.addEventListener("change", () => { newGroup.hidden = group.value !== "+new"; if (!newGroup.hidden) newGroup.focus(); });
  let color = m.color ?? null;
  const swatches = h("div", { class: "swatches", role: "radiogroup", "aria-label": tr("Color") });
  const paintSw = () => clear(swatches,
    h("button", { class: "swatch-btn none", role: "radio", "aria-checked": String(!color), title: tr("No color"), onclick: () => { color = null; paintSw(); } }, icon("x", 14)),
    SWATCHES.map((c) => h("button", { class: "swatch-btn", role: "radio", "aria-checked": String(color === c), style: { "--sw": c }, title: c, onclick: () => { color = c; paintSw(); } })),
    h("label", { class: "swatch-btn", style: { "--sw": color && !SWATCHES.includes(color) ? color : "conic-gradient(red, yellow, lime, cyan, blue, magenta, red)" }, title: tr("Other color…") },
      h("input", { type: "color", value: color ?? "#3584e4", "aria-label": tr("Other color…"), onchange: (e) => { color = e.target.value; paintSw(); } })));
  paintSw();
  const notes = h("textarea", { class: "input", rows: "2", "aria-label": tr("Notes"), style: { "font-family": "var(--font)" } });
  notes.value = m.notes ?? "";
  const extra = h("textarea", { class: "input", rows: "4", "aria-label": tr("Advanced options"), spellcheck: "false", placeholder: "ServerAliveInterval 30\nForwardAgent no" });
  extra.value = hst.extra ?? "";
  const preview = h("span", {});
  const updatePreview = () => (preview.textContent = `ssh ${alias.value.trim() || "…"}`);
  alias.addEventListener("input", () => { updatePreview(); save.disabled = !alias.value.trim(); });
  const err = h("p", { class: "hint err", role: "alert" });
  const save = button(tr("Save"), { kind: "primary" });
  save.disabled = !hst.alias;
  updatePreview();
  const browse = button(tr("Choose…"), { icon: "key", onClick: async () => {
    const path = await call("pick_identity_file").catch(() => null);
    if (path) key.value = path;
  } });
  const field = (label, el, extraClass = "") => h("div", { class: `field ${extraClass}`.trim() }, h("label", {}, label), el);
  const close = sheet(existing ? tr("Edit connection") : tr("New connection"), [
    h("div", { class: "form-section" }, h("h4", {}, icon("globe", 15), tr("Connection")),
      h("div", { class: "form-grid" },
        h("div", { class: "field" }, h("label", {}, tr("Alias")), alias, h("p", { class: "hint" }, tr("The name used by ssh and in the sidebar."))),
        field("HostName", hostName),
        field(tr("User"), user),
        field(tr("Port"), port),
        field("ProxyJump", proxyJump, "full"),
        field(tr("Identity file (private key)"), h("div", { class: "key-pick" }, key, browse), "full"))),
    h("div", { class: "form-section" }, h("h4", {}, icon("layers", 15), tr("Organization")),
      h("div", { class: "form-grid" },
        h("div", { class: "field" }, h("label", {}, tr("Group")), group, newGroup),
        h("div", { class: "field" }, h("span", { class: "label" }, tr("Color")), swatches),
        field(tr("Notes"), notes, "full"),
        h("p", { class: "hint full" }, tr("Group, color and notes are kept in Sulafat's own file; ~/.ssh/config stays plain for ssh.")))),
    h("details", { class: "form-section", open: !!hst.extra }, h("summary", { class: "more-toggle" }, tr("Advanced options")),
      h("div", { class: "field", style: { "margin-top": "10px" } }, extra, h("p", { class: "hint" }, tr("Directives not mapped above, free-form text")))),
    h("div", { class: "cmd-preview" }, icon("terminal", 15), preview),
    err,
    h("div", { class: "sheet-actions" }, button(tr("Cancel"), { onClick: () => close() }), save),
  ]);
  save.addEventListener("click", async () => {
    err.textContent = "";
    const portValue = port.value.trim();
    const portNumber = Number(portValue);
    if (portValue && !(Number.isInteger(portNumber) && portNumber >= 1 && portNumber <= 65535)) { err.textContent = tr("The port must be between 1 and 65535."); return; }
    const groupName = group.value === "+new" ? newGroup.value.trim() || null : group.value || null;
    const next = {
      alias: alias.value.trim(), host_name: hostName.value.trim() || null, user: user.value.trim() || null,
      port: portValue ? portNumber : null, identity_file: key.value.trim() || null, proxy_jump: proxyJump.value.trim() || null,
      extra: extra.value, read_only: false,
    };
    save.disabled = true;
    try {
      applyHosts(await call("host_save", { original: existing?.alias ?? null, host: next, meta: { group: groupName, color, notes: notes.value.trim() || null } }));
      close();
      toast(existing ? tr("Host saved in ~/.ssh/config.") : tr("Host created in ~/.ssh/config."));
    } catch (error) {
      err.textContent = `${tr("Could not save")}: ${error.message}`;
      save.disabled = false;
    }
  });
  requestAnimationFrame(() => (existing ? hostName : alias).focus());
}

// ---------- Sessões (abas) ----------

function connect(entry) {
  const { host, meta } = entry;
  const tab = tabOf(host.alias);
  // Já há uma aba (conectando, conectada ou encerrada) para o host: só a traz para frente.
  if (tab) { select(tab); return; }
  open({ target: { alias: host.alias }, alias: host.alias, title: host.alias, color: meta.color ?? null });
}

async function quickConnect(text) {
  const title = await call("quick_target", { text }).catch(() => null);
  if (!title) { toast(tr("Use user@host or user@host:port."), { ic: "alert-circle" }); return false; }
  open({ target: { quick: text.trim() }, alias: null, title, color: null });
  return true;
}

async function open(spec) {
  const session = new Session({
    ...spec, font: terminalFont(), scrollback: settings.scrollback_lines,
    onShortcut: (e, k) => terminalShortcut(e, k),
  });
  tabs.push(session);
  for (const tb of tabs) if (tb !== session) tb.show(false);
  current = session;
  $("stage").classList.remove("empty");
  $("stage").querySelector(".term-empty")?.remove();
  try {
    await session.start($("stage"));
    session.setState(earlyStates.get(session.id) ?? { running: true });
    earlyStates.delete(session.id);
    renderEnded(session);
  } catch (error) {
    fail(error);
    dropTab(session);
    return;
  }
  select(session);
}

function select(tab) {
  current = tab;
  for (const tb of tabs) tb.show(tb === tab);
  renderTabs();
  renderStage();
  renderHosts();
}

function dropTab(tab) {
  const i = tabs.indexOf(tab);
  if (i < 0) return;
  tabs.splice(i, 1);
  tab.dispose();
  if (current === tab) current = tabs[Math.min(i, tabs.length - 1)] ?? null;
  if (current) select(current); else { renderTabs(); renderStage(); renderHosts(); }
}

async function closeTab(tab) {
  if (!tab) return;
  if (tab.running) {
    const ok = await confirm({
      title: tr("End session?"), text: tr("This tab has an active SSH session. Do you really want to close it?"),
      ic: "terminal", confirmLabel: tr("Close"), confirmKind: "danger",
    });
    if (!ok) return;
  }
  dropTab(tab);
}

function onSessionState(state) {
  const tab = tabs.find((tb) => tb.id === state.id);
  if (!tab) { earlyStates.set(state.id, state); return; }
  tab.setState(state);
  renderEnded(tab);
  renderTabs();
  renderHosts();
}

function renderEnded(tab) {
  const exit = tab.exit;
  const message = !exit || (exit.code === 0 && !exit.signal) ? tr("Session ended")
    : exit.signal ? trf("Session ended (signal {signal})", { signal: exit.signal })
      : trf("Session ended (code {status})", { status: exit.code ?? "?" });
  clear(tab.ended, icon("alert", 20),
    h("div", { class: "body" }, h("strong", {}, message), h("small", {}, tab.alias ? `ssh ${tab.alias}` : `ssh ${tab.title}`)),
    button(tr("Close"), { size: "sm", onClick: () => dropTab(tab) }),
    button(tr("Reconnect"), { size: "sm", kind: "primary", icon: "rotate-ccw", onClick: () => tab.reconnect().catch(fail) }));
}

function renderTabs() {
  clear($("tabs"), tabs.map((tb) => h("button", {
    class: "tab", role: "tab", "aria-selected": String(tb === current), style: { "--tc": tb.color ?? "var(--fg-3)" },
    title: tb.alias ? `ssh ${tb.alias}` : `ssh ${tb.title}`,
    onclick: () => select(tb),
    onauxclick: (e) => { if (e.button === 1) closeTab(tb); },
  }, h("span", { class: `dot ${tb.running ? "" : "ended"}`.trim() }), h("span", { class: "label" }, tb.title),
    h("span", { class: "x", role: "button", "aria-label": tr("Close"), title: tr("Close"), onclick: (e) => { e.stopPropagation(); closeTab(tb); } }, icon("x", 13)))),
  iconButton("bolt", tr("Quick connection"), openQuick, "sm tab-add"));
  const tab = currentTab();
  const entry = tab?.alias && hosts.find((x) => x.host.alias === tab.alias);
  clear($("header-actions"),
    entry && iconButton("folder", tr("Open files"), () => openFiles(entry.host.alias)),
    tab && h("button", { class: "icon-btn", "aria-label": tr("Session menu"), title: tr("Session menu"), onclick: (e) => tabMenu(e.currentTarget, tab) }, icon("more", 17)),
    h("span", { class: "header-sep" }));
  const title = tab ? `${tab.title} — Sulafat` : "Sulafat";
  if (document.title !== title) { document.title = title; win.setTitle(title).catch(() => {}); }
}

function tabMenu(anchor, tab) {
  const entry = tab.alias && hosts.find((x) => x.host.alias === tab.alias);
  const pop = popover(anchor, [
    menuItem("copy", tr("Copy ssh command"), () => {
      pop.close();
      call("clipboard_set", { text: `ssh ${tab.alias ?? tab.title}`, primary: false }).then(() => toast(tr("Command copied."))).catch(fail);
    }),
    entry && !entry.host.read_only && menuItem("cog", tr("Edit host"), () => { pop.close(); openHost(entry); }),
    !tab.running && menuItem("rotate-ccw", tr("Reconnect"), () => { pop.close(); tab.reconnect().catch(fail); }),
    h("hr"),
    menuItem("x", tr("Close tab"), () => { pop.close(); closeTab(tab); }, "Ctrl Shift W"),
  ].filter(Boolean), "menu");
}

function renderStage() {
  const stage = $("stage");
  const empty = tabs.length === 0;
  stage.classList.toggle("empty", empty);
  stage.querySelector(".term-empty")?.remove();
  if (!empty) return;
  const q = h("input", { class: "input", placeholder: tr("user@host[:port]"), "aria-label": tr("Quick connection target"), spellcheck: "false" });
  const go = async () => { if (await quickConnect(q.value)) q.value = ""; };
  q.addEventListener("keydown", (e) => { if (e.key === "Enter") go(); });
  stage.append(h("div", { class: "term-empty" }, h("div", { class: "empty" },
    h("div", { class: "art" }, icon("terminal", 38)),
    h("h3", {}, tr("No open sessions")),
    h("p", {}, tr("Select a host in the sidebar or use quick connection")),
    h("div", { class: "quick" }, q, button(tr("Connect"), { kind: "primary", onClick: go })),
    h("div", { class: "quick-more" }, button(tr("New connection"), { icon: "plus", disabled: !!loadError, onClick: () => openHost() })))));
}

// ---------- Conexão rápida, configurações, menu ----------

function openQuick() {
  const q = h("input", { class: "input mono", placeholder: tr("user@host[:port]"), "aria-label": tr("Quick connection target"), spellcheck: "false", style: { width: "100%" } });
  const go = button(tr("Connect"), { kind: "primary", onClick: async () => { if (await quickConnect(q.value)) close(); } });
  go.disabled = true;
  q.addEventListener("input", async () => { go.disabled = !(await call("quick_target", { text: q.value }).catch(() => null)); });
  q.addEventListener("keydown", (e) => { if (e.key === "Enter" && !go.disabled) go.click(); });
  const close = sheet(tr("Quick connection"), [
    h("p", { class: "muted", style: { "margin-bottom": "12px" } }, tr("Opens a session without saving the host. Keys, ssh-agent and ~/.ssh/config apply as in plain ssh.")),
    q, h("div", { class: "sheet-actions" }, button(tr("Cancel"), { onClick: () => close() }), go)], { wide: false });
  requestAnimationFrame(() => q.focus());
}

function applySettingsToTabs() {
  for (const tb of tabs) tb.applySettings(terminalFont(), settings.scrollback_lines);
}

async function openSettings() {
  const families = await call("monospace_fonts").catch(() => []);
  const current = settings.font ? parseFont(settings.font) : null;
  const system = parseFont(systemFont || "Monospace 11");
  const family = h("select", { class: "input", "aria-label": tr("Font") },
    h("option", { value: "" }, trf("System font ({font})", { font: systemFont || "Monospace 11" })),
    [...new Set([...(current ? [current.family] : []), ...families])].map((f) => h("option", { value: f, selected: f === current?.family }, f)));
  const size = h("input", { class: "input", type: "number", min: "6", max: "72", step: "1", value: String(current?.points ?? system.points), "aria-label": tr("Font size"), style: { width: "84px" } });
  const lines = h("input", { class: "input", type: "number", min: "0", max: "1000000", step: "1000", value: String(settings.scrollback_lines), "aria-label": tr("Scrollback (lines)"), style: { width: "120px" } });
  const sync = () => { size.disabled = !family.value; };
  const persist = async () => {
    sync();
    const points = Math.min(72, Math.max(6, Number(size.value) || system.points));
    const scrollback = Math.min(1000000, Math.max(0, Math.round(Number(lines.value) || 0)));
    settings = await call("settings_set", { settings: { font: family.value ? `${family.value} ${points}` : null, scrollback_lines: scrollback } });
    applySettingsToTabs();
  };
  for (const el of [family, size, lines]) el.addEventListener("change", () => persist().catch(fail));
  sync();
  const rowOf = (title, subtitle, ...suffix) => h("div", { class: "list-row" }, h("div", { class: "body" }, h("div", { class: "title" }, title), subtitle && h("div", { class: "subtitle" }, subtitle)), h("div", { class: "suffix" }, suffix));
  sheet(tr("Settings"), [
    h("div", { class: "pref-group" }, h("h4", {}, tr("Terminal")), list(
      rowOf(tr("Font"), null, family, size),
      rowOf(tr("Scrollback (lines)"), tr("How many lines each tab keeps to scroll back."), lines),
    )),
  ], { wide: false });
}

async function restoreBackup() {
  const ok = await confirm({
    title: tr("Restore latest backup?"), text: "~/.ssh/config", ic: "rotate-ccw", confirmLabel: tr("Restore"),
    facts: [{ icon: "info", text: tr("Sulafat keeps a backup before every save. The current file becomes a backup too.") }],
  });
  if (!ok) return;
  try { applyHosts(await call("backup_restore")); toast(tr("Backup restored.")); } catch (error) { fail(error); }
}

async function systemAction(kind) {
  const spec = {
    "log-out": [tr("Log out?"), tr("Your session and all active applications will be closed."), "log-out", tr("Log out")],
    restart: [tr("Restart?"), tr("The computer will restart and all active sessions will end."), "rotate-ccw", tr("Restart")],
    "power-off": [tr("Power off?"), tr("The computer and all active sessions will be powered off."), "power", tr("Power off")],
  }[kind];
  const ok = await confirm({ title: spec[0], text: spec[1], ic: spec[2], tone: "danger", confirmLabel: spec[3], confirmKind: "danger" });
  if (!ok) return;
  try { await call("system_action", { action: kind }); } catch (error) { fail(error); }
}

function openMainMenu(anchor) {
  const pop = popover(anchor, [
    !loadError && menuItem("plus", tr("New connection"), () => { pop.close(); openHost(); }, "Ctrl N"),
    menuItem("bolt", tr("Quick connection"), () => { pop.close(); openQuick(); }, "Ctrl Shift T"),
    h("hr"),
    menuItem("cog", tr("Settings"), () => { pop.close(); openSettings(); }, "Ctrl ,"),
    menuItem("rotate-ccw", tr("Restore latest backup"), () => { pop.close(); restoreBackup(); }),
    menuItem("keyboard", tr("Keyboard shortcuts"), () => { pop.close(); openShortcuts(); }, "Ctrl ?"),
    menuItem("star", tr("About Sulafat"), () => { pop.close(); openAbout(); }),
    h("hr"),
    menuItem("log-out", tr("Log out…"), () => { pop.close(); systemAction("log-out"); }),
    menuItem("rotate-ccw", tr("Restart…"), () => { pop.close(); systemAction("restart"); }),
    menuItem("power", tr("Power off…"), () => { pop.close(); systemAction("power-off"); }),
  ].filter(Boolean), "menu");
}

function openShortcuts() {
  const rows = [
    [tr("New connection"), "Ctrl N"], [tr("Quick connection"), "Ctrl Shift T"], [tr("Search hosts"), "Ctrl F"],
    [tr("Connect to the selected host"), "Enter"], [tr("Edit the selected host"), "F2"], [tr("Delete the selected host"), "Del"],
    [tr("Copy in the terminal"), "Ctrl Shift C"], [tr("Paste in the terminal"), "Ctrl Shift V"],
    [tr("Close tab"), "Ctrl Shift W"], [tr("Previous / next tab"), "Ctrl PgUp PgDn"], [tr("Settings"), "Ctrl ,"],
  ];
  sheet(tr("Keyboard shortcuts"), list(rows.map(([label, keys]) => h("div", { class: "list-row" }, h("div", { class: "body" }, label), h("div", { class: "suffix" }, keys.split(" ").map((k) => h("kbd", {}, k)))))), { wide: false });
}

function openAbout() {
  sheet(tr("About Sulafat"), [h("div", { style: { "text-align": "center", padding: "6px 0 4px" } },
    h("div", { class: "brand-mark", style: { width: "72px", height: "72px", "border-radius": "20px", margin: "0 auto 12px" } }, icon("sulafat", 38)),
    h("h2", { style: { "font-size": "22px", "font-weight": "800" } }, "Sulafat"),
    h("p", { class: "muted" }, appInfo.version),
    h("p", { class: "muted" }, tr("SSH client for the Lyra OS ecosystem.")),
    h("p", { class: "small faint" }, "OpenSSH · ~/.ssh/config · Lyra OS · GPL-3.0-or-later"))], { wide: false });
}

// ---------- Janela e atalhos ----------

function cycleTab(step) {
  if (!tabs.length) return;
  const i = tabs.indexOf(current);
  select(tabs[(i + step + tabs.length) % tabs.length]);
}

/** Atalhos que valem também com o foco no terminal (o resto vai para o `ssh`). */
function terminalShortcut(e, k) {
  if (document.querySelector(".overlay")) return false;
  if (e.ctrlKey && e.shiftKey && !e.altKey && k === "t") { e.preventDefault(); openQuick(); return true; }
  if (e.ctrlKey && e.shiftKey && !e.altKey && k === "w") { e.preventDefault(); closeTab(current); return true; }
  if (e.ctrlKey && !e.shiftKey && !e.altKey && (e.key === "PageDown" || e.key === "PageUp")) { e.preventDefault(); cycleTab(e.key === "PageDown" ? 1 : -1); return true; }
  if (e.ctrlKey && !e.shiftKey && !e.altKey && e.key === ",") { e.preventDefault(); openSettings(); return true; }
  return false;
}

function setupWindow() {
  const make = (name, label, fn) => h("button", { class: "wc", "aria-label": label, title: label, onclick: fn }, icon(name, 13));
  const maxBtn = make("maximize", tr("Maximize"), () => win.toggleMaximize());
  $("window-controls").append(make("minus", tr("Minimize"), () => win.minimize()), maxBtn, make("x", tr("Close"), () => win.close()));
  document.addEventListener("mousedown", (e) => {
    if (e.button !== 0 || !e.target.closest("[data-drag]") || e.target.closest("button, input, select")) return;
    if (e.detail === 2) win.toggleMaximize(); else win.startDragging();
  });
  for (const dir of ["North", "South", "East", "West", "NorthEast", "NorthWest", "SouthEast", "SouthWest"]) {
    document.body.append(h("div", { class: "resize", dataset: { dir }, onmousedown: (e) => { e.preventDefault(); win.startResizeDragging(dir); } }));
  }
  const sync = async () => {
    const max = await win.isMaximized();
    document.body.classList.toggle("maximized", max);
    clear(maxBtn, icon(max ? "restore" : "maximize", 13));
  };
  win.onResized(sync);
  sync();
  new ResizeObserver(() => current?.refit()).observe($("stage"));
  win.onCloseRequested(async (event) => {
    const running = tabs.filter((tb) => tb.running).length;
    if (!running) return;
    event.preventDefault();
    const ok = await confirm({
      title: tr("Close window?"), text: tr("There are active SSH sessions in this window. Do you really want to close it?"),
      ic: "terminal", confirmLabel: tr("Close"), confirmKind: "danger",
    });
    if (ok) win.destroy();
  });
  document.addEventListener("keydown", (e) => {
    if (document.querySelector(".overlay") || e.target.closest?.(".term-host")) return;
    const mod = e.ctrlKey || e.metaKey;
    const k = e.key.toLowerCase();
    if (terminalShortcut(e, k)) return;
    if (mod && !e.shiftKey && k === "n") { e.preventDefault(); if (!loadError) openHost(); }
    else if (mod && k === "f") { e.preventDefault(); $("search").focus(); $("search").select(); }
    else if (mod && !e.shiftKey && k === "w" && current) { e.preventDefault(); closeTab(current); }
    else if (mod && e.key === "?") { e.preventDefault(); openShortcuts(); }
  });
}

async function start() {
  appInfo = await call("app_info");
  await loadCatalog(call, appInfo.locale);
  applyInterface(await call("interface_style").catch(() => ({})));
  on("interface-changed", applyInterface);
  settings = await call("settings_get").catch(() => settings);
  $("brand-mark").innerHTML = svg("sulafat", 15);
  $("search-ic").innerHTML = svg("search", 15);
  $("quick-btn").innerHTML = svg("bolt", 17);
  $("add-btn").innerHTML = svg("plus", 17);
  $("menu-btn").innerHTML = svg("menu", 17);
  $("sidebar-toggle").innerHTML = svg("sidebar", 17);
  $("quick-btn").title = $("quick-btn").ariaLabel = tr("Quick connection");
  $("add-btn").title = $("add-btn").ariaLabel = tr("New connection");
  $("menu-btn").title = $("menu-btn").ariaLabel = tr("Main menu");
  $("sidebar-toggle").title = $("sidebar-toggle").ariaLabel = tr("Hosts");
  $("hosts").setAttribute("aria-label", tr("Hosts"));
  $("search").placeholder = tr("Search hosts…");
  $("search").setAttribute("aria-label", tr("Search hosts"));
  $("search").addEventListener("input", (e) => { filter = e.target.value; renderHosts(); });
  $("search").addEventListener("keydown", (e) => {
    if (e.key === "ArrowDown") { e.preventDefault(); document.querySelector(".host")?.focus(); }
    else if (e.key === "Escape") { e.target.value = ""; filter = ""; renderHosts(); }
  });
  $("quick-btn").addEventListener("click", openQuick);
  $("add-btn").addEventListener("click", () => openHost());
  $("menu-btn").addEventListener("click", (e) => openMainMenu(e.currentTarget));
  $("sidebar-toggle").addEventListener("click", () => $("sidebar").classList.toggle("open"));
  setupWindow();
  on("session-state", onSessionState);
  on("hosts-changed", applyHosts);
  await load();
  renderStage();
  devHooks();
}

/** Só em builds de debug (SULAFAT_DEV_*): conecta, digita e registra o estado para testes. */
async function devHooks() {
  const dev = await call("dev_config").catch(() => null);
  if (!dev) return;
  if (dev.connect) {
    const entry = hosts.find((x) => x.host.alias === dev.connect);
    if (entry) connect(entry); else await quickConnect(dev.connect);
  }
  const steps = [...(dev.script ?? [])];
  const screen = () => {
    const buffer = current?.term.buffer.active;
    if (!buffer) return "";
    const lines = [];
    for (let i = 0; i < buffer.length; i++) lines.push(buffer.getLine(i)?.translateToString(true) ?? "");
    return lines.join("\n").trimEnd();
  };
  setInterval(() => {
    const text = screen();
    if (steps.length && current?.running && new RegExp(steps[0][0]).test(text)) current.term.input(steps.shift()[1].replace(/\\r/g, "\r"));
    if (dev.dump) {
      const state = tabs.map((tb) => ({ id: tb.id, title: tb.title, alias: tb.alias, running: tb.running, exit: tb.exit, current: tb === current, cols: tb.term.cols, rows: tb.term.rows }));
      call("dev_dump", { text: `${JSON.stringify({ tabs: state, hosts: hosts.length, loadError, title: document.title })}\n----\n${text}\n` }).catch(() => {});
    }
  }, 400);
}

start().catch((error) => call("app_log", { message: `start: ${error?.stack ?? error}` }).catch(() => {}));
