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

// no browser context menu ("Reload", "Inspect"...) outside text fields
document.addEventListener("contextmenu", (e) => {
  if (!e.target.closest("input, textarea")) e.preventDefault();
});

function toast(msg, ms = 2600) {
  const t = $("#toast");
  t.textContent = msg;
  t.hidden = false;
  clearTimeout(toast.h);
  toast.h = setTimeout(() => (t.hidden = true), ms);
}

// ------------------------------------------------------------------ modal
// One dialog for everything: resolves with the chosen button's value, or null when dismissed.
let modalDone = null;
function modal({ title, text = "", draw = null, buttons }) {
  closeModal(null);
  $("#modalTitle").textContent = title;
  $("#modalText").textContent = text;
  const img = $("#modalImg");
  img.hidden = !draw;
  if (draw) draw(img);
  const row = $("#modalBtns");
  row.innerHTML = "";
  for (const b of buttons) {
    const el = document.createElement("button");
    el.textContent = b.label;
    el.className = b.cls || "ghost";
    el.onclick = () => closeModal(b.value);
    row.appendChild(el);
  }
  $("#modal").hidden = false;
  row.querySelector(".primary, .danger")?.focus();
  return new Promise((resolve) => (modalDone = resolve));
}
function closeModal(value) {
  $("#modal").hidden = true;
  const done = modalDone;
  modalDone = null;
  done?.(value);
}
$("#modal").addEventListener("click", (e) => { if (e.target.id === "modal") closeModal(null); });
window.addEventListener("keydown", (e) => {
  if ($("#modal").hidden || e.key !== "Escape") return;
  e.stopImmediatePropagation();
  closeModal(null);
}, true);

/** Size a canvas so a w×h picture gets whole-pixel cells as big as the window allows. */
function pictureCanvas(cv, w, h) {
  const s = Math.max(1, Math.floor(Math.min(innerWidth * 0.8 / w, innerHeight * 0.65 / h)));
  cv.width = w * s; cv.height = h * s;
  const c = cv.getContext("2d");
  c.imageSmoothingEnabled = false;
  return { c, s };
}

// ------------------------------------------------------------------ status / account
let status = {};
async function refreshStatus() {
  try { status = await call("status"); } catch { return status; }
  const a = $("#account");
  let html = `${status.solved_count} solved`;
  if (status.score != null) html += ` · ${status.score.toLocaleString()} pts`;
  if (status.logged_in) {
    if (status.pending) html += ` · ${status.pending} to sync`;
    if (status.sync_error) html += ` · <span class="err" title="${esc(status.sync_error)}">sync error</span>`;
  } else html += " · not synced";
  html += "<br>";
  if (status.catalog_loading && !status.catalog_count) html += "downloading puzzle list…";
  else if (status.catalog_error) html += `<span class="err">${esc(status.catalog_error)}</span>`;
  else html += `${status.catalog_count.toLocaleString()} puzzles`;
  a.innerHTML = html;
  $("#userBtn").textContent = (status.logged_in ? status.nickname || "Account" : "Guest") + " ▾";
  $("#userEmail").textContent = status.logged_in ? status.email || "" : "Progress is kept on this device only";
  document.querySelectorAll("#userMenu [data-user]").forEach((b) => (b.hidden = !status.logged_in));
  document.querySelectorAll("#userMenu [data-guest]").forEach((b) => (b.hidden = status.logged_in));
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
  if (act === "login") {
    location.hash = "#/login";
  } else if (act === "sync") {
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
    toast("Logged out");
    if (location.hash === "#/" || !location.hash) route(); else location.hash = "#/";
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
  $("#loginSub").textContent = reg ? "Create a Nonograms Katana account to sync your progress"
    : "Log in with your Nonograms Katana account to sync your progress";
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
    toast(status.sync_error ? "Logged in, but sync failed: " + status.sync_error : `Logged in as ${status.nickname || status.email}`);
    location.hash = "#/";
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
  closeModal(null);
  const h = location.hash || "#/";
  if (h.startsWith("#/login")) {
    if (status.logged_in) { location.hash = "#/"; return; }
    game.close();
    showView("login");
    return;
  }
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

function setLoading(on) {
  loading = on;
  $("#more").classList.toggle("busy", on);
}
async function loadCards(reset) {
  if (reset) { page = 0; $("#cards").innerHTML = ""; $("#browse").scrollTop = 0; $("#moreBtn").hidden = true; }
  const token = ++loadToken;
  setLoading(true);
  let res;
  try { res = await call("catalog", queryArgs()); } catch (e) {
    if (token === loadToken) { toast(e.message); setLoading(false); }
    return;
  }
  if (token !== loadToken) return;
  setLoading(false);
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

const cardData = new Map(); // id -> the catalog item each rendered card was made from
function stars(v) { return v ? v.toFixed(1) : "–"; }
function cardHtml(p) {
  const scale = 110 / Math.max(p.w, p.h);
  const w = Math.round(p.w * scale), h = Math.round(p.h * scale);
  const started = p.progress != null;
  const box = (cls) => `<div class="box ${cls} ${p.color ? "color" : ""}" style="width:${w}px;height:${h}px">${p.w}×${p.h}</div>`;
  // solved: the picture; started: the player's own partial board (never the solution)
  const src = started ? `thumb/${p.id}.png?v=${Date.now()}` : p.solved ? `image/${p.id}.png` : null;
  // images are fetched in the background: a spinner shows until each one arrives (see imgDone)
  const thumb = src ? `<img src="${src}" width="${w}" height="${h}" loading="lazy" alt="">${box("fallback")}<span class="spinner"></span>` : box("");
  cardData.set(p.id, p);
  return `<a class="card" href="#/p/${p.id}" data-id="${p.id}">
    <div class="thumb${src ? " loading" : ""}">${thumb}
      ${p.solved ? '<span class="badge">✓ solved</span>' : ""}
      ${started ? `<span class="badge pct">${p.progress}%</span><div class="prog" style="width:${Math.min(100, p.progress)}%"></div>
        <button class="del" title="Delete progress">✕</button>` : ""}
    </div>
    <div class="meta">
      <div class="t" title="${esc(p.title)}">${esc(p.title || "Untitled")}</div>
      <div class="a">${p.w}×${p.h}${p.color ? " · color" : ""} · by <span class="author" data-author="${esc(p.author)}">${esc(p.author)}</span></div>
      <div class="stats"><span title="Rating">★ ${stars(p.rating)}</span><span title="Difficulty">⚔ ${stars(p.difficulty)}</span><span title="Fans">♥ ${stars(p.fans)}</span><span>#${p.id}</span></div>
    </div></a>`;
}
// load/error don't bubble, so listen in the capture phase
function imgDone(e) {
  const t = e.target.tagName === "IMG" && e.target.parentNode;
  if (!t || !t.classList.contains("thumb")) return;
  t.classList.remove("loading");
  t.classList.toggle("failed", e.type === "error");
}
$("#browse").addEventListener("load", imgDone, true);
$("#browse").addEventListener("error", imgDone, true);
$("#browse").addEventListener("click", (e) => {
  const card = e.target.closest(".card");
  const p = card && cardData.get(+card.dataset.id);
  if (p && e.target.closest(".del")) { e.preventDefault(); deleteProgress(p); return; }
  if (p && p.solved && p.progress == null && !e.target.closest(".author")) { e.preventDefault(); showResult(p); return; }
  const a = e.target.closest(".author");
  if (!a) return;
  e.preventDefault();
  filters.elements.author.value = a.dataset.author;
  filters.elements.status.value = "all";
  if (mode !== "browse") location.hash = "#/";
  filters.dispatchEvent(new Event("input"));
});

async function deleteProgress(p) {
  const ok = await modal({
    title: "Delete progress?",
    text: `Your progress on “${p.title || "Untitled"}” (${p.progress}%) will be deleted. This can't be undone.`,
    buttons: [{ label: "Cancel", value: false }, { label: "Delete", cls: "danger", value: true }],
  });
  if (!ok) return;
  try { await call("deleteProgress", { id: p.id }); } catch (e) { toast("Could not delete: " + e.message); return; }
  toast("Progress deleted");
  // update in place so the list keeps its scroll position and loaded pages
  const fresh = { ...p, progress: null };
  document.querySelectorAll(`.card[data-id="${p.id}"]`).forEach((el) => {
    if (el.closest("#stripCards") || mode === "continue") el.remove();
    else el.outerHTML = cardHtml(fresh);
  });
  $("#continueStrip").hidden = !$("#stripCards").children.length;
  if (mode === "continue" && !$("#cards").children.length) $("#cards").innerHTML = '<div class="empty">Nothing in progress yet.</div>';
}

/** The finished picture of a solved puzzle, with the option to play it again from scratch. */
async function showResult(p) {
  const pic = new Image();
  pic.src = `image/${p.id}.png`;
  const v = await modal({
    title: p.title || "Untitled",
    text: `#${p.id} · ${p.w}×${p.h} · by ${p.author} · ✓ solved`,
    draw: (cv) => {
      const { c } = pictureCanvas(cv, p.w, p.h);
      const paint = () => { c.fillStyle = "#fff"; c.fillRect(0, 0, cv.width, cv.height); c.drawImage(pic, 0, 0, cv.width, cv.height); };
      c.fillStyle = "#fff"; c.fillRect(0, 0, cv.width, cv.height);
      if (pic.complete && pic.naturalWidth) paint(); else pic.onload = paint;
    },
    buttons: [{ label: "Close", value: null }, { label: "Solve again", cls: "primary", value: "again" }],
  });
  if (v !== "again") return;
  try { await call("deleteProgress", { id: p.id }); } catch { /* nothing to clear */ }
  location.hash = `#/p/${p.id}`;
}

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
 * Returns null when no arrangement fits, else { exactly(k, s), empty(i) }: whether clue
 * number k can occupy exactly cells [s, s + n_k), and whether cell i can stay empty, in
 * some valid arrangement.
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
    empty(i) {
      if (!canEmpty(i)) return false;
      for (let k = 0; k <= m; k++) {
        if ((G[i * W + k] || E[i * W + k]) && (Gb[(i + 1) * W + k] || Eb[(i + 1) * W + k])) return true;
      }
      return false;
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
    this.helps = 0; // times Help was used on this board; undo and Reset don't take them back
    this.solved = false;
    // manually crossed-out clue numbers: rowMarks[y] / colMarks[x] = Set of clue indexes
    this.rowMarks = Array.from({ length: h }, () => new Set());
    this.colMarks = Array.from({ length: w }, () => new Set());
    if (progress && progress.cells && progress.cells.length === w * h && !progress.solved) {
      this.cells.set(progress.cells);
      this.time = progress.time || 0;
      this.helps = progress.helps || 0;
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
    this.hint = null; // what the last Help did: { color, cells, isRow?, line? }
    this.gaps = new Uint8Array(w * h); // auto crosses around solved numbers, see spreadGaps
    this.lineCache = [];
    this.dirty = false;
    this.buildPalette();
    this.showHelps();
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
  /** Raw player cells of a line; with auto: also the auto crosses of finished lines. */
  lineState(isRow, i, auto = true) {
    const n = isRow ? this.w : this.h;
    const out = new Array(n);
    for (let k = 0; k < n; k++) out[k] = isRow ? this.view(i * this.w + k, k, i, auto) : this.view(k * this.w + i, i, k, auto);
    return out;
  },
  /**
   * What cell i at (x, y) shows. Crosses filling the empty cells of finished lines, and the
   * ones around solved numbers, are never stored: they are derived from rowFull/colFull and
   * gaps, so they disappear as soon as the line stops matching its clue and undo/redo/saves
   * only ever see what the player did.
   */
  view(i, x, y, auto = true) {
    const v = this.cells[i];
    if (v !== EMPTY || !auto) return v;
    return this.gaps[i] || (this.autoCrossOn && (this.rowFull[y] || this.colFull[x])) ? CROSS : v;
  },
  lineFull(line, clue) {
    const runs = runsOf(line);
    return runs.length === clue.length && runs.every((r, k) => r.n === clue[k].n && r.c === clue[k].c);
  },
  /**
   * done[k]: clue number k is solved. gaps: cells that must be empty because of the solved
   * numbers (the caller skips the ones that aren't unknown).
   */
  doneFor(line, clue) {
    const m = clue.length, n = line.length;
    const done = new Array(m).fill(false), gaps = [];
    const span = (a, b) => { for (let j = a; j < b; j++) gaps.push(j); };
    if (this.lineFull(line, clue)) {
      done.fill(true);
      span(0, n);
      return { done, full: true, gaps };
    }
    if (!m) return { done, full: false, gaps };
    const sol = solveLine(line, clue);
    if (!sol) return { done, full: false, gaps }; // the line contradicts its clue: claim nothing
    // A block is a solved number when, across all valid arrangements, the only clue
    // placement covering it is one number sitting exactly on it. This catches blocks
    // closed by crosses/edges/other colours, and also full-length blocks next to
    // unknown cells that can't grow any further.
    // A block that several numbers could be, all of its own length, is complete all the
    // same: it needs a gap on each side where every candidate's neighbour shares its colour.
    const at = new Array(m); // where each solved number starts
    let i = 0;
    while (i < n) {
      const c = line[i];
      if (c <= 0) { i++; continue; }
      let e = i;
      while (e < n && line[e] === c) e++;
      let found = -1, count = 0, exact = true, gapL = true, gapR = true;
      for (let k = 0; k < m && exact; k++) {
        if (clue[k].c !== c || clue[k].n < e - i) continue;
        for (let s = Math.max(0, e - clue[k].n); s <= i; s++) {
          if (!sol.exactly(k, s)) continue;
          if (clue[k].n !== e - i) { exact = false; break; }
          found = k; count++;
          if (k > 0 && clue[k - 1].c !== c) gapL = false;
          if (k + 1 < m && clue[k + 1].c !== c) gapR = false;
        }
      }
      if (exact && count) {
        if (gapL && i > 0) gaps.push(i - 1);
        if (gapR && e < n) gaps.push(e);
        if (count === 1) { done[found] = true; at[found] = i; }
      }
      i = e;
    }
    // nothing but gaps between two solved neighbours, and between the border and the number next to it
    for (let k = 0; k < m; k++) {
      if (!done[k]) continue;
      if (k === 0) span(0, at[k]);
      if (k === m - 1) span(at[k] + clue[k].n, n);
      else if (done[k + 1]) span(at[k] + clue[k].n, at[k + 1]);
    }
    return { done, full: false, gaps };
  },
  updateDone(onlyRows, onlyCols) {
    this.autoCrossOn = $("#autoCross").checked && !this.solved;
    const gapsOn = $("#autoGaps").checked && !this.solved;
    const all = onlyRows == null || !this.rowFull;
    let rows = all ? this.rowClues.map((_, y) => y) : [...new Set(onlyRows)];
    let cols = all ? this.colClues.map((_, x) => x) : [...new Set(onlyCols)];
    if (all) { this.rowFull = []; this.colFull = []; this.rowDone = []; this.colDone = []; }
    // fullness only depends on the player's own cells
    let rowsFlipped = all, colsFlipped = all;
    for (const y of rows) {
      const f = this.lineFull(this.lineState(true, y, false), this.rowClues[y]);
      if (f !== this.rowFull[y]) { this.rowFull[y] = f; rowsFlipped = true; }
    }
    for (const x of cols) {
      const f = this.lineFull(this.lineState(false, x, false), this.colClues[x]);
      if (f !== this.colFull[x]) { this.colFull[x] = f; colsFlipped = true; }
    }
    if (gapsOn) { this.spreadGaps(); return; }
    if (all) this.gaps.fill(0);
    // a row finishing or unfinishing changes the auto crosses seen by every column, and vice versa
    if (rowsFlipped && this.autoCrossOn) cols = this.colClues.map((_, x) => x);
    if (colsFlipped && this.autoCrossOn) rows = this.rowClues.map((_, y) => y);
    for (const y of rows) this.rowDone[y] = this.doneFor(this.lineState(true, y), this.rowClues[y]);
    for (const x of cols) this.colDone[x] = this.doneFor(this.lineState(false, x), this.colClues[x]);
  },
  /**
   * Auto crosses around solved numbers. A new cross can finish a number in the crossing line
   * (or in its own), so lines are redone until nothing changes. Every move starts over from
   * no crosses, so a line the move didn't touch goes through the states it went through last
   * time: those come from the cache.
   */
  spreadGaps() {
    const w = this.w;
    this.gaps.fill(0);
    const rows = new Set(this.rowClues.keys()), cols = new Set(this.colClues.keys());
    while (rows.size || cols.size) {
      for (const [isRow, todo, other] of [[true, rows, cols], [false, cols, rows]]) {
        for (const i of todo) {
          todo.delete(i);
          const line = this.lineState(isRow, i), key = line.join();
          const seen = this.lineCache[isRow ? i : this.h + i] ??= new Map();
          let res = seen.get(key);
          if (!res) {
            if (seen.size >= 16) seen.clear();
            seen.set(key, res = this.doneFor(line, isRow ? this.rowClues[i] : this.colClues[i]));
          }
          (isRow ? this.rowDone : this.colDone)[i] = res;
          for (const k of res.gaps) {
            if (line[k] !== EMPTY) continue;
            this.gaps[isRow ? i * w + k : k * w + i] = 1;
            todo.add(i); other.add(k);
          }
        }
      }
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
  /** Length of the run of cells showing the same as (x, y), along its row or its column. */
  runLen(x, y, horiz) {
    const dx = +horiz, dy = +!horiz, v = this.view(y * this.w + x, x, y);
    const same = (x, y) => x >= 0 && y >= 0 && x < this.w && y < this.h && this.view(y * this.w + x, x, y) === v;
    let n = 1;
    for (let k = 1; same(x + k * dx, y + k * dy); k++) n++;
    for (let k = 1; same(x - k * dx, y - k * dy); k++) n++;
    return n;
  },
  applyStroke() {
    const d = this.drag;
    for (const [i, before] of d.changed) this.cells[i] = before; // undo the previous preview
    d.changed = new Map();
    const { sx, sy } = d;
    let { cx, cy } = d;
    if (Math.abs(cx - sx) >= Math.abs(cy - sy)) cy = sy; else cx = sx;
    d.horiz = cy === sy;
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
      if (this.spaceStroke) this.applySpace(false, !!sx);
    }
    this.kb = true;
    this.revealCursor();
    this.draw();
  },
  /** Space pressed (first=true) or cursor moved (horizontally or not) while space is held. */
  applySpace(first, horiz) {
    const c = this.cursor, kind = this.cursorKind();
    if (kind === "cell") {
      const i = c.y * this.w + c.x;
      if (first) this.spaceStroke = { kind, value: this.cycleValue(this.cells[i]), changed: new Map(), len: 1 };
      const st = this.spaceStroke;
      if (st.kind !== "cell") return;
      if (!first) {
        // the counted length starts over, from the cell the cursor turned at, when the movement changes direction
        const p = horiz ? c.x : c.y, line = horiz ? c.y : c.x;
        if (st.horiz !== horiz || st.line !== line) {
          st.horiz = horiz; st.line = line;
          st.lo = st.hi = (horiz ? st.at.y : st.at.x) === line ? (horiz ? st.at.x : st.at.y) : p;
        }
        st.lo = Math.min(st.lo, p); st.hi = Math.max(st.hi, p);
        st.len = st.hi - st.lo + 1;
      }
      st.at = { x: c.x, y: c.y };
      if (this.cells[i] === st.value) return;
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
  revealCursor(at = this.cursor) {
    if (this.autoFit) return;
    const b = this.bands(), cs = this.cs;
    const { x, y } = at;
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
  commit(diff, fromHistory, hint = null) {
    this.hint = hint; // a Help highlight lasts until the next move
    if (!fromHistory) {
      this.undo.push(diff);
      this.redo = [];
    }
    this.dirty = true;
    this.scheduleSave();
    this.checkWin();
    this.draw();
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
    this.hint = null;
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

  // ---- help
  /** Every run of unknown cells that one line alone settles: { isRow, line, cells: [[i, value]] }. */
  deductions() {
    const out = [];
    for (const isRow of [true, false]) {
      (isRow ? this.rowClues : this.colClues).forEach((clue, idx) => {
        const line = this.lineState(isRow, idx), n = line.length;
        const sol = solveLine(line, clue);
        if (!sol) return;
        // the colour each cell may take: 0 none, c just that one, -1 several
        const can = new Int16Array(n);
        clue.forEach(({ n: len, c }, k) => {
          let from = 0;
          for (let s = 0; s + len <= n; s++) {
            if (!sol.exactly(k, s)) continue;
            for (let j = Math.max(s, from); j < s + len; j++) can[j] = can[j] === 0 || can[j] === c ? c : -1;
            from = s + len;
          }
        });
        let run = null, last = 0;
        for (let j = 0; j < n; j++) {
          let v = 0;
          if (line[j] === EMPTY) {
            if (can[j] === 0) v = CROSS;
            else if (can[j] > 0 && !sol.empty(j)) v = can[j];
          }
          if (v && v !== last) out.push(run = { isRow, line: idx, cells: [] });
          if (v) run.cells.push([isRow ? idx * this.w + j : j * this.w + idx, v]);
          last = v;
        }
      });
    }
    return out;
  },
  /** Does the first that applies: fix a mistake, settle something a single line gives away, reveal a random cell. */
  help() {
    if (!this.p || this.solved || this.drag || this.spaceStroke) return;
    const bad = this.mistakes();
    let hint;
    if (bad.length) {
      hint = { color: "#e5484d", cells: [[bad[0], this.sol[bad[0]] || CROSS]] };
      toast(bad.length > 1 ? `Fixed a mistake, ${bad.length - 1} more left` : "Fixed a mistake");
    } else {
      const found = this.deductions();
      if (found.length) {
        hint = { color: "#30a46c", ...found[Math.floor(Math.random() * found.length)] };
        toast(`${hint.isRow ? "Row" : "Column"} ${hint.line + 1} gives this away`);
      } else {
        const open = [];
        for (let i = 0; i < this.cells.length; i++) if (this.view(i, i % this.w, Math.floor(i / this.w)) === EMPTY) open.push(i);
        if (!open.length) return;
        const i = open[Math.floor(Math.random() * open.length)];
        hint = { color: "#0090ff", cells: [[i, this.sol[i] || CROSS]] };
        toast("No line gives anything away: revealed a cell");
      }
    }
    this.helps++;
    this.showHelps();
    const diff = hint.cells.map(([i, v]) => [i, this.cells[i], v]);
    for (const [i, , v] of diff) this.cells[i] = v;
    this.updateDone();
    this.revealCursor({ x: diff[0][0] % this.w, y: Math.floor(diff[0][0] / this.w) });
    this.commit(diff, false, hint);
  },
  showHelps() {
    $("#helpCount").textContent = this.helps;
    $("#helpCount").hidden = !this.helps;
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
  // saves right away (moves are discrete): the window may close at any moment
  scheduleSave() {
    clearTimeout(this.saveTimer);
    this.saveTimer = setTimeout(() => this.save(), 0);
  },
  snapshot() {
    const marks = (arr) => Object.fromEntries(arr.map((s, i) => [i, [...s]]).filter(([, v]) => v.length));
    return {
      cells: Array.from(this.cells), time: this.time, helps: this.helps, solved: this.solved,
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
  // the full sidebar and the collapsed rail each get the same set of buttons
  buildPalette() {
    const colors = this.color ? this.pal.slice(1) : [this.pal[1]];
    for (const box of [$("#palette"), $("#railPalette")]) {
      box.innerHTML = "";
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
    }
    this.selectTool(1);
  },
  selectTool(t) {
    if (t === "x") this.crossTool = !this.crossTool;
    else if (t >= 1 && t <= this.pal.length - 1) { this.tool = t; this.crossTool = false; }
    else return;
    for (const box of [$("#palette"), $("#railPalette")]) {
      const btns = box.children;
      for (let k = 0; k < btns.length; k++) {
        const isX = k === btns.length - 1;
        btns[k].classList.toggle("sel", isX ? this.crossTool : !this.crossTool && k + 1 === this.tool);
      }
    }
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
        const v = this.view(y * w + x, x, y);
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

    const hint = this.solved ? null : this.hint;
    if (hint) {
      c.fillStyle = c.strokeStyle = hint.color;
      if (hint.line != null) {
        c.globalAlpha = 0.16;
        if (hint.isRow) c.fillRect(gx, gy + hint.line * cs, w * cs, cs); else c.fillRect(gx + hint.line * cs, gy, cs, h * cs);
        c.globalAlpha = 1;
      }
      c.lineWidth = Math.max(2, cs / 8);
      for (const [i] of hint.cells) c.strokeRect(gx + (i % w) * cs + 1, gy + Math.floor(i / w) * cs + 1, cs - 2, cs - 2);
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

    // the clue bands' background stops where the board does
    const boardR = Math.min(this.vw, gx + w * cs), boardB = Math.min(this.vh, gy + h * cs);

    // top clues
    const topY = Math.max(0, by), leftX = Math.max(0, bx);
    c.fillStyle = panel;
    c.fillRect(leftX, topY, boardR - leftX, bottom - topY);
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
    c.fillStyle = panel;
    c.fillRect(leftX, bottom, right - leftX, boardB - bottom);
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

    // outline the clues of the focused row and column; colour clue boxes hide the tint alone
    const lw = Math.max(2, Math.min(3, cs / 10));
    c.lineWidth = lw;
    const outline = (x, y, bw, bh, clipX, clipY) => {
      c.save();
      c.beginPath();
      c.rect(clipX, clipY, this.vw - clipX, this.vh - clipY);
      c.clip();
      c.strokeRect(x + lw / 2, y + lw / 2, bw - lw, bh - lw);
      c.restore();
    };
    const outlineCol = (x) => outline(gx + x * cs, topY, cs, bottom - topY, right, 0);
    const outlineRow = (y) => outline(leftX, gy + y * cs, right - leftX, cs, 0, bottom);
    if (focus && !this.solved) {
      c.strokeStyle = accent;
      if (fx >= 0 && fx < w) outlineCol(fx);
      if (fy >= 0 && fy < h) outlineRow(fy);
    }
    // and of the line the last Help worked from
    if (hint && hint.line != null) {
      c.strokeStyle = hint.color;
      if (hint.isRow) outlineRow(hint.line); else outlineCol(hint.line);
    }

    // keyboard cursor on the grid
    if (this.kb && this.cursorKind() === "cell") {
      const px = gx + this.cursor.x * cs, py = gy + this.cursor.y * cs;
      if (px >= right - 1 && py >= bottom - 1) cursorBox(px, py, cs, cs);
    }

    // corner: size / position / stroke length
    const kx = leftX, ky = topY, kw = right - kx, kh = bottom - ky;
    c.fillStyle = panel;
    c.fillRect(kx, ky, kw, kh);
    c.fillStyle = muted;
    c.font = `600 ${Math.max(10, Math.min(18, kw / 6))}px system-ui, sans-serif`;
    const st = this.drag || this.spaceStroke;
    if (st && st.len > 1) {
      // stroke length, and the length of the run it ended up part of when that is longer
      let text = String(st.len);
      const run = (this.drag ? st.to : st.value) === EMPTY ? st.len
        : this.drag ? this.runLen(st.sx, st.sy, st.horiz) : this.runLen(st.at.x, st.at.y, st.horiz);
      if (run !== st.len) text += "/" + run;
      c.fillStyle = ink;
      c.font = `700 ${Math.max(14, Math.min(40, kw / Math.max(3, text.length * 0.7), kh / 2))}px system-ui, sans-serif`;
      c.fillText(text, kx + kw / 2, ky + kh / 2, kw - 6);
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
  if (e.key === "Escape") location.hash = "#/";
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
$("#autoCross").onchange = (e) => { store.set("autoCross", e.target.checked); if (game.p) { game.updateDone(); game.draw(); } };
$("#autoGaps").checked = store.get("autoGaps", false);
$("#autoGaps").onchange = (e) => { store.set("autoGaps", e.target.checked); if (game.p) { game.updateDone(); game.draw(); } };
$("#helpBtn").onclick = () => game.help();
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
  if (game.time % 5 === 0) game.save();
}, 1000);
document.addEventListener("visibilitychange", () => { if (document.hidden && game.p) game.save(); });

// ---- open / win
let openToken = 0;
async function openPuzzle(id) {
  const token = ++openToken;
  $("#pTitle").textContent = "Loading…";
  $("#pSub").textContent = "";
  closeModal(null);
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
  const { w, h, sol, pal } = game;
  modal({
    title: "Solved!",
    text: `${m.title || "Untitled"} · ${fmtTime(game.time)} · ${game.helps ? `${game.helps} help${game.helps > 1 ? "s" : ""}` : "no help"}`,
    draw: (cv) => {
      const { c, s } = pictureCanvas(cv, w, h);
      for (let i = 0; i < sol.length; i++) {
        c.fillStyle = sol[i] ? pal[sol[i]] : pal[0];
        c.fillRect((i % w) * s, Math.floor(i / w) * s, s, s);
      }
    },
    buttons: [{ label: "Look at it", value: null }, { label: "Back to list", cls: "primary", value: "back" }],
  }).then((v) => { if (v === "back") location.hash = "#/"; });
  game.draw();
  try {
    const r = await call("solved", { id: m.id, seconds: game.time });
    $("#modalText").textContent += status.logged_in
      ? (r.new ? " · syncing to your account…" : " · already on your account")
      : " · saved on this device, log in to sync it";
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
