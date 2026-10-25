// Uma aba de sessão: xterm.js ligado ao `ssh` que roda num PTY no Rust.
// Como no VTE, o `ssh` cuida de senha, passphrase e fingerprint dentro do
// terminal. Ctrl+Shift+C/V copiam e colam; selecionar com o mouse preenche a
// seleção primária e o clique do meio cola, como no GNOME.

import { Terminal } from "../vendor/xterm/xterm.mjs";
import { FitAddon } from "../vendor/xterm/addon-fit.mjs";
import { WebLinksAddon } from "../vendor/xterm/addon-web-links.mjs";
import { call, newChannel } from "./api.js";
import { h } from "./ui.js";

// Paleta do GNOME Console (libadwaita), sempre escura como o palco do terminal.
const THEME = {
  background: "#1e1e22", foreground: "#e6e6ea", cursor: "#e6e6ea", cursorAccent: "#1e1e22",
  selectionBackground: "rgba(153, 193, 241, 0.35)",
  black: "#241f31", red: "#c01c28", green: "#2ec27e", yellow: "#f5c211",
  blue: "#1e78e4", magenta: "#9841bb", cyan: "#0ab9dc", white: "#c0bfbc",
  brightBlack: "#5e5c64", brightRed: "#ed333b", brightGreen: "#57e389", brightYellow: "#f8e45c",
  brightBlue: "#51a1ff", brightMagenta: "#c061cb", brightCyan: "#4fd2fd", brightWhite: "#f6f5f4",
};
const URL = /(https?|ftp):\/\/[^\s<>"']+/;
const STYLE_WORDS = /^(thin|ultra-?light|extra-?light|light|semi-?light|book|regular|normal|medium|semi-?bold|demi-?bold|bold|ultra-?bold|extra-?bold|heavy|black|italic|oblique|condensed|semi-?condensed|expanded)$/i;

/** Descrição de fonte do Pango ("Adwaita Mono Bold 11") → família e tamanho em px. */
export function parseFont(desc) {
  const words = (desc || "").trim().split(/\s+/).filter(Boolean);
  let points = 11;
  if (words.length && /^\d+(\.\d+)?$/.test(words.at(-1))) points = Number(words.pop());
  while (words.length > 1 && STYLE_WORDS.test(words.at(-1))) words.pop();
  return { family: words.join(" ") || "monospace", points };
}

const fontOptions = (desc) => {
  const { family, points } = parseFont(desc);
  return { fontFamily: `"${family}", monospace`, fontSize: Math.round(points * 4 / 3) };
};

export class Session {
  /**
   * @param {{ target: object, alias: string|null, title: string, color: string|null,
   *   font: string, scrollback: number, onState: Function, onShortcut: Function, onClose: Function }} spec
   */
  constructor(spec) {
    Object.assign(this, { alias: spec.alias, title: spec.title, color: spec.color, target: spec.target });
    this.id = null;
    this.running = false;
    this.exit = null;
    this.spec = spec;
    this.host = h("div", { class: "term-host" });
    this.ended = h("div", { class: "session-ended", role: "status", hidden: true });
    this.el = h("div", { class: "term-pane", hidden: true }, this.host, this.ended);
    this.term = new Terminal({
      ...fontOptions(spec.font), scrollback: spec.scrollback, theme: THEME,
      cursorBlink: true, allowProposedApi: false, rightClickSelectsWord: false,
    });
    this.fit = new FitAddon();
    this.term.loadAddon(this.fit);
    this.term.loadAddon(new WebLinksAddon((event, uri) => {
      event.preventDefault();
      call("open_uri", { uri }).catch(() => {});
    }, { urlRegex: URL }));
    this.term.attachCustomKeyEventHandler((e) => this.key(e));
    this.term.onData((data) => this.running && call("session_write", { id: this.id, data }).catch(() => {}));
    this.term.onBinary((raw) => this.running && call("session_write", { id: this.id, bytes: [...raw].map((c) => c.charCodeAt(0) & 255) }).catch(() => {}));
    this.term.onResize(({ cols, rows }) => this.running && call("session_resize", { id: this.id, cols, rows }).catch(() => {}));
    let selTimer = 0;
    this.term.onSelectionChange(() => {
      clearTimeout(selTimer);
      selTimer = setTimeout(() => {
        const text = this.term.getSelection();
        if (text) call("clipboard_set", { text, primary: true }).catch(() => {});
      }, 120);
    });
    // Clique do meio: cola a seleção primária, a menos que o programa remoto use o mouse.
    this.host.addEventListener("mousedown", (e) => {
      if (e.button !== 1 || this.term.modes.mouseTrackingMode !== "none") return;
      e.preventDefault();
      e.stopPropagation();
      this.paste(true);
    }, true);
    this.host.addEventListener("mouseup", (e) => { if (e.button === 1 && this.term.modes.mouseTrackingMode === "none") { e.preventDefault(); e.stopPropagation(); } }, true);
    this.pendingAck = 0;
    this.ackTimer = 0;
  }

  key(e) {
    if (e.type !== "keydown") return true;
    const k = e.key.toLowerCase();
    if (e.ctrlKey && e.shiftKey && !e.altKey && e.code === "KeyC") {
      const text = this.term.getSelection();
      if (text) call("clipboard_set", { text, primary: false }).catch(() => {});
      return false;
    }
    if (e.ctrlKey && e.shiftKey && !e.altKey && e.code === "KeyV") { this.paste(false); return false; }
    // Atalhos do aplicativo que também valem com o foco no terminal.
    if (this.spec.onShortcut(e, k)) return false;
    return true;
  }

  async paste(primary) {
    const text = await call("clipboard_get", { primary }).catch(() => null);
    if (text && this.running) this.term.paste(text);
  }

  /** Coloca o terminal no DOM (precisa estar visível para medir) e inicia o `ssh`. */
  async start(stage) {
    stage.append(this.el);
    this.el.hidden = false;
    this.term.open(this.host);
    this.fit.fit();
    const output = newChannel();
    output.onmessage = (message) => this.output(message);
    const opened = await call("session_open", { target: this.spec.target, cols: this.term.cols, rows: this.term.rows, output });
    this.id = opened.id;
    this.title = opened.title;
  }

  output(message) {
    const bytes = message instanceof ArrayBuffer ? new Uint8Array(message) : new Uint8Array(message?.buffer ?? message);
    this.term.write(bytes, () => {
      this.pendingAck += bytes.length;
      if (!this.ackTimer) this.ackTimer = setTimeout(() => {
        const n = this.pendingAck;
        this.pendingAck = 0;
        this.ackTimer = 0;
        if (this.id !== null) call("session_ack", { id: this.id, bytes: n }).catch(() => {});
      }, 16);
    });
  }

  setState({ running, code, signal }) {
    this.running = running;
    this.exit = running ? null : { code, signal };
    this.ended.hidden = running;
  }

  async reconnect() {
    if (this.running) return;
    this.term.write("\r\n");
    this.fit.fit();
    await call("session_spawn", { id: this.id, cols: this.term.cols, rows: this.term.rows });
    this.focus();
  }

  show(visible) {
    this.el.hidden = !visible;
    if (visible) requestAnimationFrame(() => { this.refit(); this.focus(); });
  }

  refit() {
    if (!this.el.hidden && this.el.isConnected) {
      try { this.fit.fit(); } catch { /* sem tamanho ainda */ }
    }
  }

  focus() { this.term.focus(); }

  applySettings(font, scrollback) {
    Object.assign(this.term.options, fontOptions(font), { scrollback });
    this.refit();
  }

  dispose() {
    if (this.id !== null) call("session_close", { id: this.id }).catch(() => {});
    clearTimeout(this.ackTimer);
    this.term.dispose();
    this.el.remove();
  }
}
