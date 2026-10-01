"use strict";

const $ = (s) => document.querySelector(s);
const esc = (s) => String(s ?? "").replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c]));
const store = {
  get(k, d) { try { const v = localStorage.getItem(k); return v == null ? d : JSON.parse(v); } catch { return d; } },
  set(k, v) { try { localStorage.setItem(k, JSON.stringify(v)); } catch { /* ignore */ } },
};

// ------------------------------------------------------------------ native bridge
// Every command goes to the Rust side over IPC; replies come back via __ipcReply.
const pending = new Map();
let seq = 0;
window.__ipcReply = (id, ok, payload) => {
  const p = pending.get(id);
  if (!p) return;
  pending.delete(id);
  ok ? p.resolve(payload) : p.reject(new Error(payload));
};
function call(cmd, args = {}) {
  return new Promise((resolve, reject) => {
    const id = ++seq;
    pending.set(id, { resolve, reject });
    window.ipc.postMessage(JSON.stringify({ id, cmd, args }));
  });
}

function toast(msg, ms = 2600) {
  const t = $("#toast");
  t.textContent = msg;
  t.hidden = false;
  clearTimeout(toast.h);
  toast.h = setTimeout(() => (t.hidden = true), ms);
}

// ------------------------------------------------------------------ status / account
let status = {};
async function refreshStatus() {
  try { status = await call("status"); } catch { return status; }
  const a = $("#account");
  let html = `${status.solved_count} solved`;
  if (status.score != null) html += ` · ${status.score.toLocaleString()} pts`;
  if (status.pending) html += ` · ${status.pending} to sync`;
  if (status.sync_error) html += ` · <span class="err" title="${esc(status.sync_error)}">sync error</span>`;
  html += "<br>";
  if (status.catalog_loading && !status.catalog_count) html += "downloading puzzle list…";
  else if (status.catalog_error) html += `<span class="err">${esc(status.catalog_error)}</span>`;
  else html += `${status.catalog_count.toLocaleString()} puzzles`;
  a.innerHTML = html;
  $("#userBtn").textContent = (status.nickname || "Account") + " ▾";
  $("#userEmail").textContent = status.email || "";
  if (!status.catalog_count && !status.catalog_error) setTimeout(refreshStatus, 1000);
  else if (view === "browse" && cardsWaitingForCatalog) { cardsWaitingForCatalog = false; loadCards(true); }
  return status;
}
setInterval(() => { if (status.logged_in) refreshStatus(); }, 20000);

$("#userBtn").onclick = (e) => { e.stopPropagation(); $("#userMenu").hidden = !$("#userMenu").hidden; };
document.addEventListener("click", () => ($("#userMenu").hidden = true));
$("#userMenu").onclick = async (e) => {
  const act = e.target.dataset.act;
  if (!act) return;
  if (act === "sync") {
    try {
      const r = await call("sync");
      toast(r.pushed ? `Synced, uploaded ${r.pushed} solved` : "Synced");
    } catch (err) { toast("Sync failed: " + err.message); }
    await refreshStatus();
    if (view === "browse") loadCards(true);
  } else if (act === "refresh") {
    await call("refreshCatalog");
    toast("Refreshing the puzzle list…");
    setTimeout(refreshStatus, 1500);
  } else if (act === "logout") {
    try {
      await call("logout");
    } catch (err) {
      if (!confirm(`${err.message}. They will be lost if you log out now. Log out anyway?`)) return;
      await call("logout", { force: true });
    }
    status = await refreshStatus();
    location.hash = "#/";
    route();
  }
};

// ------------------------------------------------------------------ login
let loginMode = "login";
function setLoginMode(m) {
  loginMode = m;
  const reg = m === "register";
  $("#nickRow").hidden = !reg;
  $("#loginForm").elements.nickname.required = reg;
  $("#toRegister").hidden = reg;
  $("#toLogin").hidden = !reg;
  $("#loginSubmit").textContent = reg ? "Create account" : "Log in";
  $("#loginSub").textContent = reg ? "Create a Nonograms Katana account" : "Log in with your Nonograms Katana account";
  $("#loginForm").elements.password.autocomplete = reg ? "new-password" : "current-password";
  $("#loginError").hidden = true;
}
document.querySelectorAll("[data-mode]").forEach((a) => (a.onclick = (e) => { e.preventDefault(); setLoginMode(a.dataset.mode); }));
$("#loginForm").onsubmit = async (e) => {
  e.preventDefault();
  const f = e.target.elements;
  const btn = $("#loginSubmit");
  btn.disabled = true;
  $("#loginError").hidden = true;
  try {
    const args = { email: f.email.value, password: f.password.value };
    if (loginMode === "register") args.nickname = f.nickname.value;
    await call(loginMode, args);
    f.password.value = "";
    status = await refreshStatus();
    route();
  } catch (err) {
    $("#loginError").textContent = err.message;
    $("#loginError").hidden = false;
  } finally {
    btn.disabled = false;
  }
};

// ------------------------------------------------------------------ routing
let view = null;
let mode = "browse";
function route() {
  if (!status.logged_in) {
    game.close();
    showView("login");
    return;
  }
  const h = location.hash || "#/";
  const m = h.match(/^#\/p\/(\d+)/);
  document.querySelectorAll("[data-nav]").forEach((a) => a.classList.remove("on"));
  if (m) {
    showView("play");
    openPuzzle(+m[1]);
  } else {
    game.close();
    showView("browse");
    mode = h.startsWith("#/continue") ? "continue" : "browse";
    document.querySelector(`[data-nav=${mode}]`).classList.add("on");
    $("#filters").hidden = mode === "continue";
    $("#contHead").hidden = mode !== "continue";
    loadCards(true);
    loadStrip();
  }
}
function showView(v) {
  view = v;
  $("#login").hidden = v !== "login";
  $("#topbar").hidden = v !== "browse";
  $("#browse").hidden = v !== "browse";
  $("#play").hidden = v !== "play";
}
window.addEventListener("hashchange", route);

// ------------------------------------------------------------------ browse
let page = 0;
let loading = false;
let cardsWaitingForCatalog = false;
let loadToken = 0;
const filters = $("#filters");

(function restoreFilters() {
  const saved = store.get("filters", {});
  for (const [k, v] of Object.entries(saved)) if (filters.elements[k]) filters.elements[k].value = v;
})();
let filterTimer;
filters.addEventListener("input", () => {
  clearTimeout(filterTimer);
  filterTimer = setTimeout(() => {
    store.set("filters", Object.fromEntries(new FormData(filters)));
    loadCards(true);
  }, 250);
});
filters.addEventListener("submit", (e) => e.preventDefault());

function queryArgs() {
  if (mode === "continue") return { status: "started", page, per: 60 };
  const f = Object.fromEntries(new FormData(filters));
  const [min, max] = f.size.split("-");
  return { q: f.q, author: f.author, color: f.color, status: f.status, sort: f.sort, min, max, rating: f.rating, page, per: 60 };
}

async function loadCards(reset) {
  if (reset) { page = 0; $("#cards").innerHTML = ""; $("#browse").scrollTop = 0; }
  const token = ++loadToken;
  loading = true;
  let res;
  try { res = await call("catalog", queryArgs()); } catch (e) { toast(e.message); loading = false; return; }
  if (token !== loadToken) return;
  loading = false;
  const box = $("#cards");
  if (reset && !res.items.length) {
    const empty = !status.catalog_count;
    cardsWaitingForCatalog = empty;
    box.innerHTML = `<div class="empty">${empty ? "Downloading the puzzle list (≈5 MB)…"
      : mode === "continue" ? "Nothing in progress yet." : "No puzzles match."}</div>`;
  }
  $("#resultCount").textContent = `${res.total.toLocaleString()} puzzles`;
  box.insertAdjacentHTML("beforeend", res.items.map(cardHtml).join(""));
  $("#moreBtn").hidden = (res.page + 1) * res.per >= res.total;
}
async function loadStrip() {
  if (mode !== "browse") { $("#continueStrip").hidden = true; return; }
  let res;
  try { res = await call("catalog", { status: "started", per: 12 }); } catch { return; }
  $("#continueStrip").hidden = !res.items.length;
  $("#stripCards").innerHTML = res.items.map(cardHtml).join("");
}
$("#moreBtn").onclick = () => { page++; loadCards(false); };
new IntersectionObserver((e) => {
  if (e[0].isIntersecting && !loading && !$("#moreBtn").hidden) { page++; loadCards(false); }
}, { root: $("#browse"), rootMargin: "400px" }).observe($("#more"));

function stars(v) { return v ? v.toFixed(1) : "–"; }
function cardHtml(p) {
  const scale = 110 / Math.max(p.w, p.h);
  const w = Math.round(p.w * scale), h = Math.round(p.h * scale);
  const started = p.progress != null;
  let thumb;
  if (p.solved) thumb = `<img src="image/${p.id}.png" width="${w}" height="${h}" loading="lazy" alt="">`;
  // the player's own partial board (never the solution)
  else if (started) thumb = `<img src="thumb/${p.id}.png?v=${Date.now()}" width="${w}" height="${h}" loading="lazy" alt="">`;
  else thumb = `<div class="box ${p.color ? "color" : ""}" style="width:${w}px;height:${h}px">${p.w}×${p.h}</div>`;
  return `<a class="card" href="#/p/${p.id}">
    <div class="thumb">${thumb}
      ${p.solved ? '<span class="badge">✓ solved</span>' : ""}
      ${started ? `<span class="badge pct">${p.progress}%</span><div class="prog" style="width:${Math.min(100, p.progress)}%"></div>` : ""}
    </div>
    <div class="meta">
      <div class="t" title="${esc(p.title)}">${esc(p.title || "Untitled")}</div>
      <div class="a">${p.w}×${p.h}${p.color ? " · color" : ""} · by <span class="author" data-author="${esc(p.author)}">${esc(p.author)}</span></div>
      <div class="stats"><span title="Rating">★ ${stars(p.rating)}</span><span title="Difficulty">⚔ ${stars(p.difficulty)}</span><span title="Fans">♥ ${stars(p.fans)}</span><span>#${p.id}</span></div>
    </div></a>`;
}
$("#browse").addEventListener("click", (e) => {
  const a = e.target.closest(".author");
  if (!a) return;
  e.preventDefault();
  filters.elements.author.value = a.dataset.author;
  filters.elements.status.value = "all";
  if (mode !== "browse") location.hash = "#/";
  filters.dispatchEvent(new Event("input"));
});

// ------------------------------------------------------------------ game
const EMPTY = 0, CROSS = -1;

function runsOf(arr) {
  const out = [];
  let i = 0;
  while (i < arr.length) {
    const c = arr[i];
    if (c > 0) {
      let j = i;
      while (j < arr.length && arr[j] === c) j++;
      out.push({ n: j - i, c });
      i = j;
    } else i++;
  }
  return out;
}

function colorDistance(a, b) {
  const x = parseInt(a.slice(1), 16), y = parseInt(b.slice(1), 16);
  return Math.hypot((x >> 16) - (y >> 16), ((x >> 8) & 255) - ((y >> 8) & 255), (x & 255) - (y & 255));
}

function luminance(hex) {
  const n = parseInt(hex.slice(1), 16);
  return (0.299 * (n >> 16) + 0.587 * ((n >> 8) & 255) + 0.114 * (n & 255)) / 255;
}

/**
 * Line feasibility for a (possibly colour) nonogram line.
 * line[i]: 0 unknown, -1 crossed, c > 0 filled with colour c. Same-colour neighbouring
 * clue numbers need a gap, different colours may touch.
 * Returns null when no arrangement fits, else { exactly(k, s) } telling whether clue
 * number k can occupy exactly cells [s, s + n_k) in some valid arrangement.
 */
function solveLine(line, clue) {
  const n = line.length, m = clue.length, W = m + 1;
  const blocked = new Map(); // colour -> prefix count of cells that can't take that colour
  for (const { c } of clue) {
    if (blocked.has(c)) continue;
    const a = new Int32Array(n + 1);
    for (let i = 0; i < n; i++) a[i + 1] = a[i] + (line[i] === EMPTY || line[i] === c ? 0 : 1);
    blocked.set(c, a);
  }
  const fits = (k, s) => {
    const e = s + clue[k].n;
    if (e > n) return false;
    const a = blocked.get(clue[k].c);
    return a[e] - a[s] === 0;
  };
  const canEmpty = (i) => line[i] <= 0;
  const sameAsPrev = (k) => k > 0 && clue[k - 1].c === clue[k].c;
  const sameAsNext = (k) => k + 1 < m && clue[k + 1].c === clue[k].c;
  // forward: first k numbers fill [0, i); G: cell i-1 empty (or i = 0), E: number k-1 ends at i
  const G = new Uint8Array((n + 1) * W), E = new Uint8Array((n + 1) * W);
  G[0] = 1;
  for (let i = 0; i <= n; i++) {
    for (let k = 0; k <= m; k++) {
      const g = G[i * W + k], e = E[i * W + k];
      if (!g && !e) continue;
      if (i < n && canEmpty(i)) G[(i + 1) * W + k] = 1;
      if (k < m && (g || !sameAsPrev(k)) && fits(k, i)) E[(i + clue[k].n) * W + k + 1] = 1;
    }
  }
  if (!G[n * W + m] && !E[n * W + m]) return null;
  // backward: numbers k.. fill [i, n); Gb: cell i empty (or i = n), Eb: number k starts at i
  const Gb = new Uint8Array((n + 1) * W), Eb = new Uint8Array((n + 1) * W);
  Gb[n * W + m] = 1;
  for (let i = n - 1; i >= 0; i--) {
    for (let k = m; k >= 0; k--) {
      if (canEmpty(i) && (Gb[(i + 1) * W + k] || Eb[(i + 1) * W + k])) Gb[i * W + k] = 1;
      if (k < m && fits(k, i)) {
        const e = i + clue[k].n;
        if (Gb[e * W + k + 1] || (Eb[e * W + k + 1] && !sameAsNext(k))) Eb[i * W + k] = 1;
      }
    }
  }
  return {
    exactly(k, s) {
      if (!fits(k, s)) return false;
      const e = s + clue[k].n;
      const before = G[s * W + k] || (E[s * W + k] && !sameAsPrev(k));
      const after = Gb[e * W + k + 1] || (Eb[e * W + k + 1] && !sameAsNext(k));
      return !!(before && after);
    },
  };
}

const game = {
  canvas: $("#board"),
  ctx: $("#board").getContext("2d"),
  p: null,
  autoFit: store.get("autoFit", true),

  load(data) {
    const { puzzle, meta, progress } = data;
    const { w, h, grid } = puzzle;
    this.p = puzzle;
    this.meta = meta;
    this.w = w; this.h = h;
    this.sol = grid;
    this.color = meta.color;
    this.pal = puzzle.palette.slice();
    if (!this.color) this.pal[1] = "#2b2620";
    this.nearBoard = this.pal.map((p) => colorDistance(p, this.pal[0]) < 64);
    this.cells = new Int8Array(w * h);
    this.time = 0;
    this.solved = false;
    // manually crossed-out clue numbers: rowMarks[y] / colMarks[x] = Set of clue indexes
    this.rowMarks = Array.from({ length: h }, () => new Set());
    this.colMarks = Array.from({ length: w }, () => new Set());
    if (progress && progress.cells && progress.cells.length === w * h && !progress.solved) {
      this.cells.set(progress.cells);
      this.time = progress.time || 0;
      const m = progress.marks || {};
      for (const [y, ks] of Object.entries(m.r || {})) if (this.rowMarks[y]) ks.forEach((k) => this.rowMarks[y].add(k));
      for (const [x, ks] of Object.entries(m.c || {})) if (this.colMarks[x]) ks.forEach((k) => this.colMarks[x].add(k));
    }
    this.rowClues = []; this.colClues = [];
    for (let y = 0; y < h; y++) this.rowClues.push(runsOf(grid.slice(y * w, y * w + w)));
    for (let x = 0; x < w; x++) {
      const col = [];
      for (let y = 0; y < h; y++) col.push(grid[y * w + x]);
      this.colClues.push(runsOf(col));
    }
    this.maxRow = Math.max(1, ...this.rowClues.map((c) => c.length));
    this.maxCol = Math.max(1, ...this.colClues.map((c) => c.length));
    this.totalFilled = grid.reduce((a, v) => a + (v > 0 ? 1 : 0), 0);
    this.undo = []; this.redo = [];
    this.tool = 1;
    this.crossTool = false;
    this.hover = null;
    this.cursor = { x: 0, y: 0 };
    this.kb = false; // keyboard cursor visible
    this.drag = null;
    this.spaceStroke = null;
    this.flash = null;
    this.dirty = false;
    this.buildPalette();
    this.resize();
    if (this.autoFit) this.fit(); else this.fit(14);
    this.updateFitBtn();
    this.updateDone();
    this.draw();
  },

  close() {
    if (this.p && this.dirty) this.save();
    this.p = null;
  },

  // ---- view geometry
  resize() {
    const r = this.canvas.getBoundingClientRect();
    const dpr = window.devicePixelRatio || 1;
    this.vw = r.width; this.vh = r.height;
    this.canvas.width = Math.round(r.width * dpr);
    this.canvas.height = Math.round(r.height * dpr);
    this.ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  },
  step() { return this.cs * 0.9; },
  clueW() { return this.maxRow * this.step() + 6; },
  clueH() { return this.maxCol * this.step() + 6; },
  fit(minCs = 3) {
    const pad = 16;
    const cs = Math.min((this.vw - pad) / (this.w + this.maxRow * 0.9 + 0.4), (this.vh - pad) / (this.h + this.maxCol * 0.9 + 0.4));
    this.cs = Math.max(minCs, Math.min(96, cs));
    this.gx = null;
    this.clamp();
  },
  setAutoFit(on) {
    this.autoFit = on;
    store.set("autoFit", on);
    this.updateFitBtn();
    if (on && this.p) { this.fit(); this.draw(); }
  },
  updateFitBtn() { $("#fitBtn").classList.toggle("on", this.autoFit); },
  zoom(f, cx, cy) {
    if (!this.p) return;
    this.setAutoFit(false);
    const ncs = Math.max(3, Math.min(80, this.cs * f));
    if (cx == null) { cx = this.vw / 2; cy = this.vh / 2; }
    const gx = cx - (cx - this.gx) * ncs / this.cs;
    const gy = cy - (cy - this.gy) * ncs / this.cs;
    this.cs = ncs; this.gx = gx; this.gy = gy;
    this.clamp();
    this.draw();
  },
  clamp() {
    const cw = this.clueW(), ch = this.clueH(), gw = this.w * this.cs, gh = this.h * this.cs;
    const fitX = cw + gw + 16 <= this.vw, fitY = ch + gh + 16 <= this.vh;
    if (fitX) this.gx = (this.vw - cw - gw) / 2 + cw;
    else this.gx = Math.min(cw + 8, Math.max(this.vw - gw - 8, this.gx ?? cw + 8));
    if (fitY) this.gy = (this.vh - ch - gh) / 2 + ch;
    else this.gy = Math.min(ch + 8, Math.max(this.vh - gh - 8, this.gy ?? ch + 8));
  },
  // Clue bands follow the grid but stay pinned to the viewport edge when scrolled.
  // A pinned band is capped at 40% of the view (showing the clues nearest the grid).
  bands() {
    const cw = this.clueW(), ch = this.clueH();
    const right = Math.max(this.gx, Math.min(cw, this.vw * 0.4));
    const bottom = Math.max(this.gy, Math.min(ch, this.vh * 0.4));
    return { cw, ch, bx: right - cw, by: bottom - ch, right, bottom };
  },
  /** What is under the pointer: {kind: "cell"|"row"|"col", x, y, k?} or null. */
  hit(mx, my) {
    const b = this.bands(), cs = this.cs, step = this.step();
    const x = Math.floor((mx - this.gx) / cs), y = Math.floor((my - this.gy) / cs);
    if (mx >= b.right && my >= b.bottom) {
      if (x < 0 || y < 0 || x >= this.w || y >= this.h) return null;
      return { kind: "cell", x, y };
    }
    if (my < b.bottom && mx >= b.right && x >= 0 && x < this.w) {
      const len = this.colClues[x].length;
      const k = len - Math.ceil((b.by + b.ch - 3 - my) / step);
      return k >= 0 && k < len ? { kind: "col", x, y: k - len, k } : null;
    }
    if (mx < b.right && my >= b.bottom && y >= 0 && y < this.h) {
      const len = this.rowClues[y].length;
      const k = len - Math.ceil((b.bx + b.cw - 3 - mx) / step);
      return k >= 0 && k < len ? { kind: "row", x: k - len, y, k } : null;
    }
    return null;
  },

  // ---- clue completion
  lineState(isRow, i) {
    const n = isRow ? this.w : this.h;
    const out = new Array(n);
    for (let k = 0; k < n; k++) out[k] = isRow ? this.cells[i * this.w + k] : this.cells[k * this.w + i];
    return out;
  },
  doneFor(line, clue) {
    const done = new Array(clue.length).fill(false);
    const runs = runsOf(line);
    if (runs.length === clue.length && runs.every((r, k) => r.n === clue[k].n && r.c === clue[k].c)) {
      done.fill(true);
      return { done, full: true };
    }
    const m = clue.length, n = line.length;
    if (!m) return { done, full: false };
    const sol = solveLine(line, clue);
    if (!sol) return { done, full: false }; // the line contradicts its clue: claim nothing
    // A block is a solved number when, across all valid arrangements, the only clue
    // placement covering it is one number sitting exactly on it. This catches blocks
    // closed by crosses/edges/other colours, and also full-length blocks next to
    // unknown cells that can't grow any further.
    let i = 0;
    while (i < n) {
      const c = line[i];
      if (c <= 0) { i++; continue; }
      let e = i;
      while (e < n && line[e] === c) e++;
      let found = -1, ambiguous = false;
      for (let k = 0; k < m && !ambiguous; k++) {
        if (clue[k].c !== c || clue[k].n < e - i) continue;
        for (let s = Math.max(0, e - clue[k].n); s <= i; s++) {
          if (!sol.exactly(k, s)) continue;
          if (found !== -1 || s !== i || clue[k].n !== e - i) { ambiguous = true; break; }
          found = k;
        }
      }
      if (found !== -1 && !ambiguous) done[found] = true;
      i = e;
    }
    return { done, full: false };
  },
  updateDone(onlyRows, onlyCols) {
    if (onlyRows == null) {
      this.rowDone = this.rowClues.map((c, y) => this.doneFor(this.lineState(true, y), c));
      this.colDone = this.colClues.map((c, x) => this.doneFor(this.lineState(false, x), c));
    } else {
      for (const y of onlyRows) this.rowDone[y] = this.doneFor(this.lineState(true, y), this.rowClues[y]);
      for (const x of onlyCols) this.colDone[x] = this.doneFor(this.lineState(false, x), this.colClues[x]);
    }
  },
  toggleMark(kind, idx, k, value) {
    const set = kind === "row" ? this.rowMarks[idx] : this.colMarks[idx];
    const on = value ?? !set.has(k);
    if (on) set.add(k); else set.delete(k);
    this.dirty = true;
    this.scheduleSave();
    return on;
  },

  // ---- mouse strokes
  applyStroke() {
    const d = this.drag;
    for (const [i, before] of d.changed) this.cells[i] = before; // undo the previous preview
    d.changed = new Map();
    const { sx, sy } = d;
    let { cx, cy } = d;
    if (Math.abs(cx - sx) >= Math.abs(cy - sy)) cy = sy; else cx = sx;
    const x0 = Math.min(sx, cx), x1 = Math.max(sx, cx), y0 = Math.min(sy, cy), y1 = Math.max(sy, cy);
    d.len = x1 - x0 + y1 - y0 + 1;
    for (let y = y0; y <= y1; y++) {
      for (let x = x0; x <= x1; x++) {
        const i = y * this.w + x;
        if (this.cells[i] !== d.from) continue;
        d.changed.set(i, this.cells[i]);
        this.cells[i] = d.to;
      }
    }
    const rows = [], cols = [];
    for (let y = y0; y <= y1; y++) rows.push(y);
    for (let x = x0; x <= x1; x++) cols.push(x);
    this.updateDone([...rows, ...(d.prevRows || [])], [...cols, ...(d.prevCols || [])]);
    d.prevRows = rows; d.prevCols = cols;
  },
  startStroke(cell, cross) {
    const i = cell.y * this.w + cell.x;
    const cur = this.cells[i];
    const val = cross ? CROSS : this.tool;
    const erase = cur === val;
    this.drag = { sx: cell.x, sy: cell.y, cx: cell.x, cy: cell.y, from: erase ? val : cur, to: erase ? EMPTY : val,
      changed: new Map(), len: 1 };
    this.applyStroke();
  },
  endStroke() {
    const d = this.drag;
    this.drag = null;
    if (!d || !d.changed.size) { this.draw(); return; }
    this.commit([...d.changed].map(([i, before]) => [i, before, this.cells[i]]));
  },

  // ---- keyboard cursor
  /** Space cycles: empty → fill (current colour) → cross → empty. */
  cycleValue(cur) {
    if (cur === EMPTY) return this.crossTool ? CROSS : this.tool;
    if (cur === CROSS) return EMPTY;
    return cur === this.tool || this.crossTool ? CROSS : this.tool;
  },
  cursorKind(c = this.cursor) {
    if (c.x >= 0 && c.y >= 0) return "cell";
    if (c.x < 0 && c.y >= 0) return "row";
    if (c.y < 0 && c.x >= 0) return "col";
    return null;
  },
  moveCursor(dx, dy) {
    let { x, y } = this.cursor;
    const n = Math.abs(dx || dy);
    const sx = Math.sign(dx), sy = Math.sign(dy);
    for (let s = 0; s < n; s++) {
      let nx = x + sx, ny = y + sy;
      if (nx < 0 && ny < 0) break;
      if (nx >= this.w || ny >= this.h) break;
      if (nx < 0) { // row clue area: only where a number exists
        const len = this.rowClues[ny].length;
        if (sx < 0 && nx < -len) break;
        if (sy) nx = len === 0 ? 0 : Math.max(nx, -len);
      }
      if (ny < 0) { // column clue area
        const len = this.colClues[nx].length;
        if (sy < 0 && ny < -len) break;
        if (sx) ny = len === 0 ? 0 : Math.max(ny, -len);
      }
      x = nx; y = ny;
      this.cursor = { x, y };
      if (this.spaceStroke) this.applySpace(false);
    }
    this.kb = true;
    this.revealCursor();
    this.draw();
  },
  /** Space pressed (first=true) or cursor moved while space is held. */
  applySpace(first) {
    const c = this.cursor, kind = this.cursorKind();
    if (kind === "cell") {
      const i = c.y * this.w + c.x;
      if (first) this.spaceStroke = { kind, value: this.cycleValue(this.cells[i]), changed: new Map() };
      const st = this.spaceStroke;
      if (st.kind !== "cell" || this.cells[i] === st.value) return;
      if (!st.changed.has(i)) st.changed.set(i, this.cells[i]);
      this.cells[i] = st.value;
      this.updateDone([c.y], [c.x]);
    } else if (kind) {
      const idx = kind === "row" ? c.y : c.x;
      const len = (kind === "row" ? this.rowClues[c.y] : this.colClues[c.x]).length;
      const k = len + (kind === "row" ? c.x : c.y);
      if (first) this.spaceStroke = { kind, value: this.toggleMark(kind, idx, k), changed: new Map() };
      else if (this.spaceStroke.kind === kind) this.toggleMark(kind, idx, k, this.spaceStroke.value);
    }
  },
  endSpace() {
    const st = this.spaceStroke;
    this.spaceStroke = null;
    if (st && st.changed.size) this.commit([...st.changed].map(([i, before]) => [i, before, this.cells[i]]));
    else this.draw();
  },
  revealCursor() {
    if (this.autoFit) return;
    const b = this.bands(), cs = this.cs;
    const { x, y } = this.cursor;
    const px = this.gx + Math.max(x, 0) * cs, py = this.gy + Math.max(y, 0) * cs;
    if (x >= 0) {
      if (px < b.right) this.gx += b.right - px + cs;
      else if (px + cs > this.vw) this.gx -= px + cs - this.vw + cs;
    }
    if (y >= 0) {
      if (py < b.bottom) this.gy += b.bottom - py + cs;
      else if (py + cs > this.vh) this.gy -= py + cs - this.vh + cs;
    }
    this.clamp();
  },

  // ---- history / persistence
  commit(diff, fromHistory) {
    if (!fromHistory) {
      if ($("#autoCross").checked) this.autoCross(diff);
      this.undo.push(diff);
      this.redo = [];
    }
    this.dirty = true;
    this.scheduleSave();
    this.checkWin();
    this.draw();
  },
  autoCross(diff) {
    const rows = new Set(), cols = new Set();
    for (const [i] of diff) { rows.add(Math.floor(i / this.w)); cols.add(i % this.w); }
    const extra = [];
    const mark = (i) => { if (this.cells[i] === EMPTY) { extra.push([i, EMPTY, CROSS]); this.cells[i] = CROSS; } };
    for (const y of rows) if (this.rowDone[y].full) for (let x = 0; x < this.w; x++) mark(y * this.w + x);
    for (const x of cols) if (this.colDone[x].full) for (let y = 0; y < this.h; y++) mark(y * this.w + x);
    if (extra.length) { diff.push(...extra); this.updateDone(); }
  },
  history(back) {
    const from = back ? this.undo : this.redo, to = back ? this.redo : this.undo;
    const diff = from.pop();
    if (!diff) return;
    for (const [i, before, after] of diff) this.cells[i] = back ? before : after;
    to.push(diff);
    this.updateDone();
    this.commit(diff, true);
  },
  reset() {
    const diff = [];
    this.cells.forEach((v, i) => { if (v) diff.push([i, v, EMPTY]); });
    this.rowMarks.forEach((s) => s.clear());
    this.colMarks.forEach((s) => s.clear());
    this.cells.fill(0);
    this.updateDone();
    if (diff.length) { this.undo.push(diff); this.redo = []; }
    this.dirty = true; this.scheduleSave(); this.draw();
  },
  mistakes() {
    const bad = [];
    this.cells.forEach((v, i) => {
      if ((v > 0 && v !== this.sol[i]) || (v === CROSS && this.sol[i] > 0)) bad.push(i);
    });
    return bad;
  },
  check() {
    const bad = this.mistakes();
    if (!bad.length) { toast("No mistakes so far"); return; }
    toast(`${bad.length} mistake${bad.length > 1 ? "s" : ""}`);
    this.flash = new Set(bad);
    this.draw();
    setTimeout(() => { this.flash = null; this.draw(); }, 1500);
  },
  filledCount() { let n = 0; for (const v of this.cells) if (v > 0) n++; return n; },
  checkWin() {
    if (this.solved) return;
    for (let i = 0; i < this.cells.length; i++) {
      if ((this.cells[i] > 0 ? this.cells[i] : 0) !== this.sol[i]) return;
    }
    this.solved = true;
    for (let i = 0; i < this.cells.length; i++) if (this.cells[i] === CROSS) this.cells[i] = EMPTY;
    this.updateDone();
    this.save();
    onSolved();
  },
  scheduleSave() {
    clearTimeout(this.saveTimer);
    this.saveTimer = setTimeout(() => this.save(), 700);
  },
  snapshot() {
    const marks = (arr) => Object.fromEntries(arr.map((s, i) => [i, [...s]]).filter(([, v]) => v.length));
    return {
      cells: Array.from(this.cells), time: this.time, solved: this.solved,
      marks: { r: marks(this.rowMarks), c: marks(this.colMarks) },
      pct: Math.min(99, Math.floor(this.filledCount() * 100 / Math.max(1, this.totalFilled))),
    };
  },
  save() {
    clearTimeout(this.saveTimer);
    if (!this.p) return;
    this.dirty = false;
    call("saveProgress", { id: this.p.id, data: this.snapshot() }).catch((e) => toast("Save failed: " + e.message));
  },

  // ---- palette
  buildPalette() {
    const box = $("#palette");
    box.innerHTML = "";
    const colors = this.color ? this.pal.slice(1) : [this.pal[1]];
    colors.forEach((c, k) => {
      const b = document.createElement("button");
      b.style.background = c;
      const key = k < 10 ? String((k + 1) % 10) : k < 20 ? "⇧" + ((k - 9) % 10) : "";
      b.title = `Colour ${k + 1}${key ? ` (${key})` : ""}`;
      b.innerHTML = `<span class="k" style="color:${luminance(c) > 0.55 ? "#000" : "#fff"}">${key}</span>`;
      b.onclick = () => this.selectTool(k + 1);
      box.appendChild(b);
    });
    const x = document.createElement("button");
    x.textContent = "✕";
    x.title = "Cross tool (X) — right click always crosses";
    x.onclick = () => this.selectTool("x");
    box.appendChild(x);
    this.selectTool(1);
  },
  selectTool(t) {
    if (t === "x") this.crossTool = !this.crossTool;
    else if (t >= 1 && t <= this.pal.length - 1) { this.tool = t; this.crossTool = false; }
    else return;
    const btns = $("#palette").children;
    for (let k = 0; k < btns.length; k++) {
      const isX = k === btns.length - 1;
      btns[k].classList.toggle("sel", isX ? this.crossTool : !this.crossTool && k + 1 === this.tool);
    }
    const rt = $("#railTool");
    rt.style.background = this.crossTool ? "transparent" : this.pal[this.tool];
    rt.textContent = this.crossTool ? "✕" : "";
  },

  // ---- rendering
  draw() {
    if (!this.p) return;
    const c = this.ctx, cs = this.cs, w = this.w, h = this.h, step = this.step();
    const css = getComputedStyle(document.documentElement);
    const bg = css.getPropertyValue("--bg").trim();
    const panel = css.getPropertyValue("--panel").trim();
    const ink = css.getPropertyValue("--ink").trim();
    const muted = css.getPropertyValue("--muted").trim();
    const accent = css.getPropertyValue("--accent").trim();
    const dark = matchMedia("(prefers-color-scheme: dark)").matches;
    const tint = dark ? "rgba(255,220,160,.1)" : "rgba(181,69,43,.09)";
    c.fillStyle = bg;
    c.fillRect(0, 0, this.vw, this.vh);

    const gx = this.gx, gy = this.gy;
    const x0 = Math.max(0, Math.floor(-gx / cs)), x1 = Math.min(w, Math.ceil((this.vw - gx) / cs));
    const y0 = Math.max(0, Math.floor(-gy / cs)), y1 = Math.min(h, Math.ceil((this.vh - gy) / cs));
    // the board uses the puzzle's own background colour (white unless the author picked another);
    // lines, crosses and outlines adapt to it rather than to the app theme
    const board = this.pal[0];
    const boardLight = luminance(board) > 0.5;
    const ink2 = (a) => (boardLight ? `rgba(0,0,0,${a})` : `rgba(255,255,255,${a})`);
    const cellTint = boardLight ? "rgba(181,69,43,.09)" : "rgba(255,255,255,.12)";
    const focus = this.kb ? this.cursor : this.hover;
    const fx = focus ? focus.x : -99, fy = focus ? focus.y : -99;

    // cells
    c.fillStyle = board;
    c.fillRect(gx + x0 * cs, gy + y0 * cs, (x1 - x0) * cs, (y1 - y0) * cs);
    if (focus && !this.solved) {
      c.fillStyle = cellTint;
      if (fy >= 0) c.fillRect(gx, gy + fy * cs, w * cs, cs);
      if (fx >= 0) c.fillRect(gx + fx * cs, gy, cs, h * cs);
    }
    for (let y = y0; y < y1; y++) {
      for (let x = x0; x < x1; x++) {
        const v = this.cells[y * w + x];
        const px = gx + x * cs, py = gy + y * cs;
        if (v > 0) {
          c.fillStyle = this.pal[v];
          c.fillRect(px, py, cs, cs);
          // fills that look like the board get an inner outline so they stay visible
          if (!this.solved && this.nearBoard[v] && cs > 6) {
            c.strokeStyle = ink2(0.4);
            c.lineWidth = 1;
            c.strokeRect(px + 2.5, py + 2.5, cs - 5, cs - 5);
          }
        } else if (v === CROSS && !this.solved) {
          c.strokeStyle = ink2(0.45);
          c.lineWidth = Math.max(1, cs / 14);
          const m = cs * 0.3;
          c.beginPath();
          c.moveTo(px + m, py + m); c.lineTo(px + cs - m, py + cs - m);
          c.moveTo(px + cs - m, py + m); c.lineTo(px + m, py + cs - m);
          c.stroke();
        }
      }
    }
    if (this.flash) {
      c.strokeStyle = "#e5484d";
      c.lineWidth = Math.max(2, cs / 8);
      for (const i of this.flash) c.strokeRect(gx + (i % w) * cs + 1, gy + Math.floor(i / w) * cs + 1, cs - 2, cs - 2);
    }

    // grid lines
    if (!this.solved || cs >= 10) {
      const thin = ink2(0.14);
      const thick = ink2(0.5);
      for (let pass = 0; pass < 2; pass++) {
        c.strokeStyle = pass ? thick : thin;
        c.lineWidth = pass ? 1.4 : 1;
        c.beginPath();
        for (let x = x0; x <= x1; x++) {
          if ((x % 5 === 0 || x === w) !== !!pass) continue;
          const px = Math.round(gx + x * cs) + 0.5;
          c.moveTo(px, gy + y0 * cs); c.lineTo(px, gy + y1 * cs);
        }
        for (let y = y0; y <= y1; y++) {
          if ((y % 5 === 0 || y === h) !== !!pass) continue;
          const py = Math.round(gy + y * cs) + 0.5;
          c.moveTo(gx + x0 * cs, py); c.lineTo(gx + x1 * cs, py);
        }
        c.stroke();
      }
    }

    // clue bands
    const { cw, ch, bx, by, right, bottom } = this.bands();
    const fontPx = (n) => Math.max(6, Math.min(cs * 0.62, n >= 10 ? step * 0.52 : cs * 0.62));
    const drawClue = (item, auto, marked, x, y, bw, bh, hl) => {
      if (auto && $("#autoClues").checked) marked = true;
      const dim = auto || marked;
      if (this.color) {
        c.fillStyle = this.pal[item.c];
        c.globalAlpha = dim ? 0.28 : 1;
        c.fillRect(x + 1, y + 1, bw - 2, bh - 2);
        c.fillStyle = luminance(this.pal[item.c]) > 0.55 ? "#000" : "#fff";
      } else {
        c.fillStyle = dim ? muted : ink;
        c.globalAlpha = dim ? 0.5 : 1;
      }
      c.font = `${hl && !dim ? 700 : this.color ? 600 : 500} ${fontPx(item.n)}px system-ui, sans-serif`;
      c.fillText(String(item.n), x + bw / 2, y + bh / 2 + 0.5);
      c.globalAlpha = 1;
      if (marked) {
        c.strokeStyle = this.color ? (luminance(this.pal[item.c]) > 0.55 ? "#000" : muted) : muted;
        c.lineWidth = Math.max(1, cs / 16);
        c.beginPath();
        c.moveTo(x + bw * 0.2, y + bh * 0.8); c.lineTo(x + bw * 0.8, y + bh * 0.2);
        c.stroke();
      }
    };
    const cursorBox = (x, y, bw, bh) => {
      c.strokeStyle = accent;
      c.lineWidth = Math.max(2, cs / 10);
      c.strokeRect(x + 1, y + 1, bw - 2, bh - 2);
    };
    c.textAlign = "center";
    c.textBaseline = "middle";

    // top clues
    const topY = Math.max(0, by);
    c.fillStyle = panel;
    c.fillRect(0, topY, this.vw, bottom - topY);
    for (let x = x0; x < x1; x++) {
      const clue = this.colClues[x], done = this.colDone[x], marks = this.colMarks[x];
      const hl = fx === x;
      if (hl) { c.fillStyle = tint; c.fillRect(gx + x * cs, topY, cs, bottom - topY); }
      for (let k = 0; k < clue.length; k++) {
        const yy = by + ch - 3 - (clue.length - k) * step;
        if (yy + step < 0) continue;
        drawClue(clue[k], done.done[k], marks.has(k), gx + x * cs, yy, cs, step, hl);
        if (this.kb && this.cursor.x === x && this.cursor.y === k - clue.length) cursorBox(gx + x * cs, yy, cs, step);
      }
    }
    // left clues
    const leftX = Math.max(0, bx);
    c.fillStyle = panel;
    c.fillRect(leftX, bottom, right - leftX, this.vh);
    for (let y = y0; y < y1; y++) {
      const clue = this.rowClues[y], done = this.rowDone[y], marks = this.rowMarks[y];
      const py = gy + y * cs;
      if (py + cs < bottom) continue;
      const hl = fy === y;
      if (hl) { c.fillStyle = tint; c.fillRect(leftX, py, right - leftX, cs); }
      for (let k = 0; k < clue.length; k++) {
        const xx = bx + cw - 3 - (clue.length - k) * step;
        if (xx + step < 0) continue;
        drawClue(clue[k], done.done[k], marks.has(k), xx, py, step, cs, hl);
        if (this.kb && this.cursor.y === y && this.cursor.x === k - clue.length) cursorBox(xx, py, step, cs);
      }
    }
    // separators every 5 in clue bands
    c.strokeStyle = dark ? "rgba(255,255,255,.18)" : "rgba(0,0,0,.18)";
    c.lineWidth = 1;
    c.beginPath();
    for (let x = x0; x <= x1; x++) if (x % 5 === 0) { const px = Math.round(gx + x * cs) + 0.5; if (px >= right) { c.moveTo(px, topY); c.lineTo(px, bottom); } }
    for (let y = y0; y <= y1; y++) if (y % 5 === 0) { const py = Math.round(gy + y * cs) + 0.5; if (py >= bottom) { c.moveTo(leftX, py); c.lineTo(right, py); } }
    c.stroke();

    // keyboard cursor on the grid
    if (this.kb && this.cursorKind() === "cell") {
      const px = gx + this.cursor.x * cs, py = gy + this.cursor.y * cs;
      if (px >= right - 1 && py >= bottom - 1) cursorBox(px, py, cs, cs);
    }

    // corner: size / position / drag length
    const kx = leftX, ky = topY, kw = right - kx, kh = bottom - ky;
    c.fillStyle = panel;
    c.fillRect(kx, ky, kw, kh);
    c.fillStyle = muted;
    c.font = `600 ${Math.max(10, Math.min(18, kw / 6))}px system-ui, sans-serif`;
    if (this.drag && this.drag.len > 1) {
      c.fillStyle = ink;
      c.font = `700 ${Math.max(14, Math.min(40, kw / 3, kh / 2))}px system-ui, sans-serif`;
      c.fillText(String(this.drag.len), kx + kw / 2, ky + kh / 2);
    } else if (focus && fx >= 0 && fy >= 0) {
      c.fillText(`${fx + 1}, ${fy + 1}`, kx + kw / 2, ky + kh / 2);
    } else {
      c.fillText(`${w}×${h}`, kx + kw / 2, ky + kh / 2);
    }
  },
};

// ---- mouse
const cv = game.canvas;
let panning = null;
cv.addEventListener("contextmenu", (e) => e.preventDefault());
cv.addEventListener("pointerdown", (e) => {
  if (!game.p) return;
  const r = cv.getBoundingClientRect();
  const mx = e.clientX - r.left, my = e.clientY - r.top;
  cv.setPointerCapture(e.pointerId);
  if (e.button === 1) {
    panning = { x: mx, y: my, gx: game.gx, gy: game.gy };
    e.preventDefault();
    return;
  }
  if (game.solved || (e.button !== 0 && e.button !== 2)) return;
  const hit = game.hit(mx, my);
  if (!hit) return;
  game.kb = false;
  game.cursor = { x: hit.x, y: hit.y };
  if (hit.kind === "cell") {
    game.startStroke(hit, e.button === 2 || game.crossTool);
  } else {
    game.toggleMark(hit.kind, hit.kind === "row" ? hit.y : hit.x, hit.k);
  }
  game.draw();
});
cv.addEventListener("pointermove", (e) => {
  if (!game.p) return;
  const r = cv.getBoundingClientRect();
  const mx = e.clientX - r.left, my = e.clientY - r.top;
  if (panning) {
    game.setAutoFit(false);
    game.gx = panning.gx + mx - panning.x;
    game.gy = panning.gy + my - panning.y;
    game.clamp();
    game.draw();
    return;
  }
  const hit = game.hit(mx, my);
  const cell = hit && hit.kind === "cell" ? hit : null;
  if (game.drag) {
    game.drag.cx = Math.max(0, Math.min(game.w - 1, Math.floor((mx - game.gx) / game.cs)));
    game.drag.cy = Math.max(0, Math.min(game.h - 1, Math.floor((my - game.gy) / game.cs)));
    game.applyStroke();
  }
  const old = game.hover;
  game.hover = cell;
  if (game.kb && e.movementX + e.movementY !== 0) game.kb = false;
  if (game.drag || !old !== !cell || (old && (old.x !== cell.x || old.y !== cell.y))) game.draw();
});
const endPointer = () => {
  if (panning) { panning = null; return; }
  if (game.drag) game.endStroke();
};
cv.addEventListener("pointerup", endPointer);
cv.addEventListener("pointercancel", endPointer);
cv.addEventListener("pointerleave", () => { if (!game.drag && game.hover) { game.hover = null; game.draw(); } });
cv.addEventListener("wheel", (e) => {
  if (!game.p) return;
  e.preventDefault();
  const r = cv.getBoundingClientRect();
  if (e.ctrlKey || e.metaKey) {
    game.zoom(Math.exp(-e.deltaY * (e.deltaMode ? 0.05 : 0.0025)), e.clientX - r.left, e.clientY - r.top);
    return;
  }
  const k = e.deltaMode ? 30 : 1;
  let dx = e.deltaX * k, dy = e.deltaY * k;
  if (e.shiftKey && !dx) { dx = dy; dy = 0; }
  game.gx -= dx; game.gy -= dy;
  game.clamp();
  game.draw();
}, { passive: false });
new ResizeObserver(() => {
  if (!game.p || view !== "play") return;
  game.resize();
  if (game.autoFit) game.fit(); else game.clamp();
  game.draw();
}).observe($("#stage"));

// ---- keyboard
const MOVES = { ArrowLeft: [-1, 0], ArrowRight: [1, 0], ArrowUp: [0, -1], ArrowDown: [0, 1], h: [-1, 0], l: [1, 0], k: [0, -1], j: [0, 1] };
window.addEventListener("keydown", (e) => {
  if (view !== "play" || !game.p) return;
  if (e.target.tagName === "INPUT" && e.target.type !== "checkbox") return;
  const ctrl = e.ctrlKey || e.metaKey;
  const key = e.key.length === 1 ? e.key.toLowerCase() : e.key;
  if (ctrl && key === "z") { e.preventDefault(); game.history(!e.shiftKey); return; }
  if (ctrl && key === "y") { e.preventDefault(); game.history(false); return; }
  if (ctrl || e.altKey) return;
  if (e.key === " " || e.code === "Space") {
    e.preventDefault();
    if (!e.repeat && !game.solved) { game.kb = true; game.applySpace(true); game.draw(); }
    return;
  }
  if (MOVES[key]) {
    e.preventDefault();
    const [dx, dy] = MOVES[key];
    const n = e.shiftKey ? 5 : 1;
    game.moveCursor(dx * n, dy * n);
    return;
  }
  const digit = /^Digit(\d)$/.exec(e.code);
  if (digit) {
    const d = +digit[1];
    game.selectTool((d === 0 ? 10 : d) + (e.shiftKey ? 10 : 0));
    return;
  }
  if (e.key === "Escape") { if (!$("#win").hidden) $("#win").hidden = true; else location.hash = "#/"; }
  else if (e.key === "+" || e.key === "=") game.zoom(1.2);
  else if (e.key === "-") game.zoom(1 / 1.2);
  else if (key === "x") game.selectTool("x");
  else if (e.key === "[") toggleSidebar();
  else if (e.key === "Tab") e.preventDefault();
});
window.addEventListener("keyup", (e) => {
  if ((e.key === " " || e.code === "Space") && game.spaceStroke) game.endSpace();
});
window.addEventListener("blur", () => { if (game.spaceStroke) game.endSpace(); });

// ---- sidebar
function toggleSidebar(force) {
  const collapsed = force ?? !$("#side").classList.contains("collapsed");
  $("#side").classList.toggle("collapsed", collapsed);
  $("#collapseBtn").textContent = collapsed ? "»" : "«";
  $("#collapseBtn").title = collapsed ? "Expand sidebar ( [ )" : "Collapse sidebar ( [ )";
  store.set("sideCollapsed", collapsed);
}
toggleSidebar(store.get("sideCollapsed", false));
$("#collapseBtn").onclick = () => toggleSidebar();
$("#backBtn").onclick = () => (location.hash = "#/");
$("#undoBtn").onclick = $("#railUndo").onclick = () => game.history(true);
$("#redoBtn").onclick = $("#railRedo").onclick = () => game.history(false);
$("#zoomIn").onclick = () => game.zoom(1.25);
$("#zoomOut").onclick = () => game.zoom(0.8);
$("#fitBtn").onclick = $("#railFit").onclick = () => game.setAutoFit(true);
$("#checkBtn").onclick = () => game.check();
$("#resetBtn").onclick = () => { if (confirm("Clear the whole board?")) game.reset(); };
$("#autoClues").checked = store.get("autoClues", true);
$("#autoClues").onchange = (e) => { store.set("autoClues", e.target.checked); game.draw(); };
$("#autoCross").checked = store.get("autoCross", false);
$("#autoCross").onchange = (e) => store.set("autoCross", e.target.checked);
$("#winBack").onclick = () => { $("#win").hidden = true; location.hash = "#/"; };
$("#winStay").onclick = () => { $("#win").hidden = true; };
// keep keyboard focus on the board after clicking sidebar buttons
$("#side").addEventListener("click", (e) => { const b = e.target.closest("button"); if (b) b.blur(); });

// ---- timer
function fmtTime(s) {
  const h = Math.floor(s / 3600), m = Math.floor(s / 60) % 60, sec = s % 60;
  return (h ? h + ":" + String(m).padStart(2, "0") : m) + ":" + String(sec).padStart(2, "0");
}
function showTime() {
  $("#timer").textContent = fmtTime(game.time);
  $("#railTimer").textContent = fmtTime(game.time);
}
setInterval(() => {
  if (view !== "play" || !game.p || game.solved || document.hidden) return;
  game.time++;
  showTime();
  if (game.time % 15 === 0) game.save();
}, 1000);
// called by the native side when the window closes: hand over the board to save
window.__beforeQuit = () => (game.p && !game.solved ? JSON.stringify({ id: game.p.id, data: game.snapshot() }) : null);
document.addEventListener("visibilitychange", () => { if (document.hidden && game.p) game.save(); });

// ---- open / win
let openToken = 0;
async function openPuzzle(id) {
  const token = ++openToken;
  $("#pTitle").textContent = "Loading…";
  $("#pSub").textContent = "";
  $("#win").hidden = true;
  let data;
  try { data = await call("puzzle", { id }); } catch (e) {
    toast("Could not load puzzle: " + e.message);
    if (token === openToken) location.hash = "#/";
    return;
  }
  if (token !== openToken) return;
  const m = data.meta;
  $("#pTitle").textContent = m.title || "Untitled";
  $("#pTitle").title = m.title2 || m.title;
  $("#pSub").textContent = `#${m.id} · ${m.w}×${m.h}${m.color ? " · color" : ""} · by ${m.author}${m.solved ? " · ✓ solved before" : ""}`;
  game.close();
  game.load(data);
  showTime();
}

async function onSolved() {
  const m = game.meta;
  const img = $("#winImg");
  const s = Math.max(2, Math.floor(320 / Math.max(game.w, game.h)));
  img.width = game.w * s; img.height = game.h * s;
  const c = img.getContext("2d");
  for (let i = 0; i < game.sol.length; i++) {
    c.fillStyle = game.sol[i] ? game.pal[game.sol[i]] : game.pal[0];
    c.fillRect((i % game.w) * s, Math.floor(i / game.w) * s, s, s);
  }
  $("#winText").textContent = `${m.title || "Untitled"} · ${fmtTime(game.time)}`;
  $("#win").hidden = false;
  game.draw();
  try {
    const r = await call("solved", { id: m.id, seconds: game.time });
    $("#winText").textContent += r.new ? " · syncing to your account…" : " · already on your account";
    setTimeout(refreshStatus, 3000);
  } catch (e) {
    toast("Could not record solve: " + e.message);
  }
}

// ------------------------------------------------------------------ boot
(async () => {
  await refreshStatus();
  route();
})();
