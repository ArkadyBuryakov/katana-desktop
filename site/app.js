'use strict';

const REPO = 'ArkadyBuryakov/katana-desktop';
// The version written into index.html: its links keep working after later releases,
// and are moved to the latest one when GitHub answers.
const BAKED = '1.1.0';
const SYSTEMS = {
  windows: 'Windows', macos: 'macOS', arch: 'Arch Linux',
  debian: 'Debian / Ubuntu', fedora: 'Fedora', linux: 'Linux',
};

const root = document.documentElement;
const $$ = (sel) => [...document.querySelectorAll(sel)];
// storage can be blocked: the page works without it
const stored = (key, value) => {
  try {
    if (value === undefined) return localStorage.getItem(key);
    localStorage.setItem(key, value);
  } catch {}
  return null;
};

// ---------- Desktop / TUI
function setMode(mode) {
  root.dataset.mode = mode;
  for (const b of $$('.modes [data-set-mode]')) b.setAttribute('aria-pressed', b.dataset.setMode === mode);
  stored('mode', mode);
}
for (const el of $$('[data-set-mode]')) {
  el.addEventListener('click', (e) => {
    e.preventDefault();
    setMode(el.dataset.setMode);
    history.replaceState(null, '', '#' + el.dataset.setMode);
  });
}
addEventListener('hashchange', () => {
  const mode = location.hash.slice(1);
  if (mode === 'tui' || mode === 'desktop') setMode(mode);
});
setMode(root.dataset.mode);

// ---------- systems
// A browser says which OS it runs on but rarely which Linux; Firefox on Ubuntu and Fedora does.
function detectSystem() {
  const ua = navigator.userAgent;
  if (/Android|iPhone|iPad|iPod/.test(ua)) return null;
  const platform = (navigator.userAgentData && navigator.userAgentData.platform) || navigator.platform || '';
  if (/Win/i.test(platform)) return 'windows';
  if (/Mac/i.test(platform)) return 'macos';
  if (/Linux|X11/i.test(platform + ua)) {
    if (/Fedora|Red Hat/i.test(ua)) return 'fedora';
    if (/Ubuntu|Debian|Mint/i.test(ua)) return 'debian';
    return 'linux-any';
  }
  return null;
}

function setSystem(os) {
  for (const b of $$('.tabs [data-os]')) b.setAttribute('aria-selected', b.dataset.os === os);
  for (const p of $$('.panel')) p.classList.toggle('on', p.dataset.os === os);
  document.getElementById('ctaOs').textContent = ' on ' + SYSTEMS[os];
}
for (const b of $$('.tabs [data-os]')) {
  b.addEventListener('click', () => {
    setSystem(b.dataset.os);
    stored('os', b.dataset.os);
  });
}
{
  const detected = detectSystem();
  // any Linux: the most common packages first, the others are one click away
  const os = detected === 'linux-any' ? 'debian' : detected;
  if (detected) {
    document.getElementById('detected').textContent =
      'Detected ' + (detected === 'linux-any' ? 'Linux' : SYSTEMS[os]);
  }
  const saved = stored('os');
  setSystem(saved in SYSTEMS ? saved : os || 'windows');
}

// ---------- copy buttons
for (const pre of $$('.cmd')) {
  const b = document.createElement('button');
  b.type = 'button';
  b.textContent = 'Copy';
  b.addEventListener('click', async () => {
    try {
      await navigator.clipboard.writeText(pre.querySelector('code').textContent);
      b.textContent = 'Copied';
    } catch {
      b.textContent = 'Failed';
    }
    setTimeout(() => (b.textContent = 'Copy'), 1500);
  });
  pre.append(b);
}

// ---------- keys, as hinted at the bottom of the TUI page
addEventListener('keydown', (e) => {
  if (e.ctrlKey || e.altKey || e.metaKey || e.target.closest('input, textarea, select')) return;
  const tabs = $$('.tabs [data-os]');
  if (e.key === 'd' || e.key === 't') {
    const mode = e.key === 'd' ? 'desktop' : 'tui';
    setMode(mode);
    history.replaceState(null, '', '#' + mode);
  } else if (e.key === 'i') {
    document.getElementById('install').scrollIntoView();
  } else if (e.key >= '1' && e.key <= String(tabs.length)) {
    tabs[e.key - 1].click();
  }
});

// ---------- the latest release
function setVersion(v) {
  if (v === BAKED) return;
  for (const a of $$('a[href*="/releases/download/"]')) a.href = a.href.replaceAll(BAKED, v);
  for (const el of $$('.ver')) el.textContent = v;
}
(async () => {
  try {
    let v = sessionStorage.getItem('version');
    if (!v) {
      const r = await fetch(`https://api.github.com/repos/${REPO}/releases/latest`);
      if (!r.ok) return;
      v = (await r.json()).tag_name.replace(/^v/, '');
      sessionStorage.setItem('version', v);
    }
    setVersion(v);
  } catch {}
})();
