import { detectBrowser, isSupportedBrowser } from './identity.js';
import {
  WEBSITE_CONTROLS_KEY,
  WEBSITE_CONTROL_DEFINITIONS,
  countActiveWebsiteControls,
  normalizeWebsiteControls,
} from './website-controls.js';

const connectionStatusBadge = document.getElementById('connection-status');
const connectionStatusText = document.getElementById('connection-status-text');
const incognitoWarning = document.getElementById('incognito-warning');
const pairView = document.getElementById('pair-view');
const pairForm = document.getElementById('pair');
const connectBtn = document.getElementById('connect-btn');
const pairStatus = document.getElementById('pair-status');

const wsListRoot = document.getElementById('ws-list');
const wsActiveCount = document.getElementById('ws-active-count');

const ytGroupsRoot = document.getElementById('yt-groups');
const ytActiveCount = document.getElementById('yt-active-count');
const ytResetBtn = document.getElementById('yt-reset');
const ytControlsApi = globalThis.__preelockYoutubeControls;

let pairingInFlight = false;

// The popup is the only extension context where the Brave probe is
// available, so detection happens here and the worker reuses the
// stored result. Nothing is ever chosen by the user.
async function rememberIdentity() {
  const detected = await detectBrowser();
  if (detected) await chrome.storage.local.set({ browser_identity: detected });
  return detected;
}

function setPairError(message) {
  if (!pairStatus) return;
  const text = String(message || '').trim();
  pairStatus.textContent = text;
  pairStatus.hidden = !text;
}

async function show() {
  const [data, localData] = await Promise.all([
    chrome.storage.session.get(['error']),
    chrome.storage.local.get(['token', 'disabled']),
  ]);
  const { token, disabled } = localData || {};
  const connected = Boolean(token) && !disabled && !data?.error;

  if (connectionStatusBadge && connectionStatusText) {
    connectionStatusBadge.classList.toggle('is-connected', connected);
    connectionStatusBadge.classList.toggle('is-unpaired', !connected);
    connectionStatusText.textContent = connected
      ? 'Connected'
      : 'Not connected';
  }
  if (pairView) {
    pairView.hidden = connected;
  }

  const detected = await rememberIdentity();

  if (!connected) {
    if (incognitoWarning) {
      incognitoWarning.textContent = '';
      incognitoWarning.hidden = true;
    }
    if (!pairingInFlight) {
      if (!detected) {
        setPairError(
          "Couldn't identify this browser. Use a supported browser.",
        );
      } else if (data?.error) {
        setPairError(data.error);
      } else {
        setPairError('');
      }
    }
    return;
  }

  setPairError('');
  // Warn if the extension cannot run in Incognito windows, where blocking
  // would silently not apply.
  if (incognitoWarning) {
    let allowed = true;
    try {
      allowed = await chrome.extension.isAllowedIncognitoAccess();
    } catch {
      allowed = true;
    }
    if (!allowed) {
      incognitoWarning.textContent =
        'Blocking does not apply in Incognito windows. Enable "Allow in Incognito" for Preelock in chrome://extensions.';
      incognitoWarning.hidden = false;
    } else {
      incognitoWarning.textContent = '';
      incognitoWarning.hidden = true;
    }
  }
}

if (pairForm) {
  pairForm.addEventListener('submit', async (event) => {
    event.preventDefault();
    if (pairingInFlight) return;
    const field = document.getElementById('token');
    const token = field ? field.value.trim() : '';
    if (!/^[a-f0-9]{64}$/i.test(token)) {
      setPairError('Copy the full 64-character code from Preelock Settings.');
      return;
    }
    pairingInFlight = true;
    if (connectBtn) {
      connectBtn.disabled = true;
      connectBtn.textContent = 'Connecting…';
    }
    setPairError('');
    try {
      // Identity comes from the browser itself. Without a positive
      // identification no connection is attempted: the desktop app would
      // otherwise have to guess which browser this is.
      const detected = await detectBrowser();
      if (!isSupportedBrowser(detected)) {
        setPairError(
          "Couldn't identify this browser. Use a supported browser.",
        );
        return;
      }
      const { client = crypto.randomUUID() } =
        await chrome.storage.local.get('client');
      await chrome.storage.local.set({
        token,
        client,
        disabled: false,
        browser_identity: detected,
      });
      // The legacy manual pick is no longer used; drop it so an old value
      // can never override what the browser reported.
      await chrome.storage.local.remove('browser_choice');
      if (field) field.value = '';
      await chrome.runtime.sendMessage({ type: 'sync' });
      await show();
    } catch {
      setPairError('Cannot reach Preelock. Open the desktop app, then retry.');
    } finally {
      pairingInFlight = false;
      if (connectBtn) {
        connectBtn.disabled = false;
        connectBtn.textContent = 'Connect';
      }
    }
  });
}

// ---- Website Controls UI ----
let currentWebsiteControls = normalizeWebsiteControls(null);
const websiteControlInputs = new Map();
let wsWriteQueue = Promise.resolve();

function syncWebsiteControlsUi(rawControls) {
  currentWebsiteControls = normalizeWebsiteControls(rawControls);
  for (const [key, input] of websiteControlInputs.entries()) {
    const val = Boolean(currentWebsiteControls[key]);
    if (input.checked !== val) input.checked = val;
  }
  const active = countActiveWebsiteControls(currentWebsiteControls);
  if (wsActiveCount) {
    wsActiveCount.textContent = `${active} active`;
    wsActiveCount.hidden = active === 0;
  }
}

function updateWebsiteControl(key, checked) {
  currentWebsiteControls = normalizeWebsiteControls({
    ...currentWebsiteControls,
    [key]: Boolean(checked),
  });
  syncWebsiteControlsUi(currentWebsiteControls);
  const snapshot = { ...currentWebsiteControls };
  wsWriteQueue = wsWriteQueue.then(async () => {
    await chrome.storage.local.set({ [WEBSITE_CONTROLS_KEY]: snapshot });
    try {
      await chrome.runtime.sendMessage({ type: 'reconcile_website_controls' });
    } catch {
      // storage.onChanged in worker also reconciles if SW wakes up later.
    }
  });
  return wsWriteQueue;
}

function buildWebsiteControlsUi(initialControls) {
  if (!wsListRoot) return;
  wsListRoot.textContent = '';
  websiteControlInputs.clear();
  const normalized = normalizeWebsiteControls(initialControls);

  for (const item of WEBSITE_CONTROL_DEFINITIONS) {
    const row = document.createElement('div');
    row.className = 'ws-item';

    const copy = document.createElement('div');
    copy.className = 'ws-item-copy';

    const inputId = `ws-ctrl-${item.key}`;
    const label = document.createElement('label');
    label.className = 'ws-item-label';
    label.htmlFor = inputId;
    label.textContent = item.label;

    const desc = document.createElement('p');
    desc.className = 'ws-item-desc';
    desc.textContent = item.description;

    copy.appendChild(label);
    copy.appendChild(desc);

    const sw = document.createElement('label');
    sw.className = 'yt-switch';
    const input = document.createElement('input');
    input.type = 'checkbox';
    input.id = inputId;
    input.dataset.websiteControl = item.key;
    input.checked = Boolean(normalized[item.key]);
    input.addEventListener('change', () => {
      void updateWebsiteControl(item.key, input.checked);
    });
    websiteControlInputs.set(item.key, input);

    const track = document.createElement('span');
    track.className = 'yt-switch-track';
    track.setAttribute('aria-hidden', 'true');
    sw.appendChild(input);
    sw.appendChild(track);

    row.appendChild(copy);
    row.appendChild(sw);
    wsListRoot.appendChild(row);
  }

  syncWebsiteControlsUi(normalized);
}

async function initWebsiteControls() {
  if (!wsListRoot) return;
  const data = await chrome.storage.local.get(WEBSITE_CONTROLS_KEY);
  buildWebsiteControlsUi(data?.[WEBSITE_CONTROLS_KEY]);
}

// ---- YouTube Controls UI ----
let currentControls = ytControlsApi
  ? ytControlsApi.normalizeYoutubeControls(null)
  : null;
let conflictNoteEl = null;
const groupRefs = new Map();
const controlInputs = new Map();

function createChevron() {
  const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
  svg.setAttribute('viewBox', '0 0 12 12');
  svg.setAttribute('aria-hidden', 'true');
  svg.classList.add('yt-chevron');
  const path = document.createElementNS('http://www.w3.org/2000/svg', 'path');
  path.setAttribute('d', 'M4.2 2.4L8 6L4.2 9.6');
  path.setAttribute('fill', 'none');
  path.setAttribute('stroke', 'currentColor');
  path.setAttribute('stroke-width', '1.6');
  path.setAttribute('stroke-linecap', 'round');
  path.setAttribute('stroke-linejoin', 'round');
  svg.appendChild(path);
  return svg;
}

function registerControlInput(path, input) {
  if (!controlInputs.has(path)) controlInputs.set(path, []);
  controlInputs.get(path).push(input);
}

function createControlRow(item, controls, subBox) {
  const row = document.createElement('div');
  row.className = 'yt-row';

  const labelWrap = document.createElement('div');
  labelWrap.className = 'yt-row-label-wrap';

  const inputId = `yt-ctrl-${item.path.replace(/\./g, '-')}-${Math.random()
    .toString(36)
    .slice(2, 7)}`;

  if (item.children && item.children.length && subBox) {
    const subBtn = document.createElement('button');
    subBtn.type = 'button';
    subBtn.className = 'yt-sub-toggle';
    subBtn.setAttribute('aria-label', `Toggle ${item.label} options`);
    // All sub-control dropdowns start collapsed whenever the popup opens.
    subBtn.setAttribute('aria-expanded', 'false');
    subBox.hidden = true;
    subBtn.appendChild(createChevron());
    subBtn.addEventListener('click', () => {
      const nextExpanded = subBtn.getAttribute('aria-expanded') !== 'true';
      subBtn.setAttribute('aria-expanded', nextExpanded ? 'true' : 'false');
      subBox.hidden = !nextExpanded;
      if (nextExpanded && typeof subBox.scrollIntoView === 'function') {
        subBox.scrollIntoView({ block: 'nearest' });
      }
    });
    labelWrap.appendChild(subBtn);
  }

  const label = document.createElement('label');
  label.className = 'yt-row-label';
  label.htmlFor = inputId;
  label.textContent = item.label;
  labelWrap.appendChild(label);

  const sw = document.createElement('label');
  sw.className = 'yt-switch';
  const input = document.createElement('input');
  input.type = 'checkbox';
  input.id = inputId;
  input.dataset.controlPath = item.path;
  input.checked = ytControlsApi.getControlValue(controls, item.path);
  input.addEventListener('change', () => {
    void updateYoutubeControl(item.path, input.checked);
  });
  registerControlInput(item.path, input);

  const track = document.createElement('span');
  track.className = 'yt-switch-track';
  track.setAttribute('aria-hidden', 'true');
  sw.appendChild(input);
  sw.appendChild(track);

  row.appendChild(labelWrap);
  row.appendChild(sw);
  return row;
}

function countGroupActive(group, controls) {
  const seen = new Set();
  for (const item of group.controls) {
    if (ytControlsApi.getControlValue(controls, item.path)) seen.add(item.path);
    for (const child of item.children || []) {
      if (ytControlsApi.getControlValue(controls, child.path))
        seen.add(child.path);
    }
  }
  return seen.size;
}

function syncControlsUi(controls) {
  if (!ytControlsApi || !ytGroupsRoot) return;
  currentControls = ytControlsApi.normalizeYoutubeControls(controls);

  for (const [path, inputs] of controlInputs.entries()) {
    const val = ytControlsApi.getControlValue(currentControls, path);
    for (const input of inputs) {
      if (input.checked !== val) input.checked = val;
    }
  }

  for (const group of ytControlsApi.YOUTUBE_CONTROL_GROUPS) {
    const ref = groupRefs.get(group.id);
    if (!ref) continue;
    const count = countGroupActive(group, currentControls);
    ref.badge.textContent = count ? String(count) : '';
    ref.badge.hidden = count === 0;
  }

  if (conflictNoteEl) {
    const note = ytControlsApi.subscriptionsConflictNote(currentControls);
    conflictNoteEl.textContent = note;
    conflictNoteEl.hidden = !note;
  }

  const total = ytControlsApi.countActiveControls(currentControls);
  if (ytActiveCount) {
    ytActiveCount.textContent = `${total} active`;
    ytActiveCount.hidden = total === 0;
  }
  if (ytResetBtn) {
    ytResetBtn.hidden = total === 0;
  }
}

let ytWriteQueue = Promise.resolve();

function updateYoutubeControl(path, checked) {
  if (!ytControlsApi) return Promise.resolve();
  currentControls = ytControlsApi.setControlValue(
    currentControls,
    path,
    checked,
  );
  syncControlsUi(currentControls);
  const snapshot = ytControlsApi.normalizeYoutubeControls(currentControls);
  ytWriteQueue = ytWriteQueue.then(async () => {
    await chrome.storage.local.set({ [ytControlsApi.STORAGE_KEY]: snapshot });
  });
  return ytWriteQueue;
}

function resetYoutubeControls() {
  if (!ytControlsApi) return Promise.resolve();
  const clean = ytControlsApi.normalizeYoutubeControls(null);
  syncControlsUi(clean);
  ytWriteQueue = ytWriteQueue.then(async () => {
    await chrome.storage.local.set({ [ytControlsApi.STORAGE_KEY]: clean });
  });
  return ytWriteQueue;
}

function buildControlsUi(initialControls) {
  if (!ytControlsApi || !ytGroupsRoot) return;
  ytGroupsRoot.textContent = '';
  groupRefs.clear();
  controlInputs.clear();
  if (conflictNoteEl && conflictNoteEl.parentNode) {
    conflictNoteEl.remove();
  }
  conflictNoteEl = null;

  const controls = ytControlsApi.normalizeYoutubeControls(initialControls);

  if (ytGroupsRoot.parentNode) {
    conflictNoteEl = document.createElement('p');
    conflictNoteEl.className = 'yt-conflict-note';
    conflictNoteEl.setAttribute('role', 'status');
    const note = ytControlsApi.subscriptionsConflictNote(controls);
    conflictNoteEl.textContent = note;
    conflictNoteEl.hidden = !note;
    ytGroupsRoot.parentNode.insertBefore(conflictNoteEl, ytGroupsRoot);
  }

  for (const group of ytControlsApi.YOUTUBE_CONTROL_GROUPS) {
    const card = document.createElement('div');
    card.className = 'yt-group';
    card.dataset.groupId = group.id;

    const headerBtn = document.createElement('button');
    headerBtn.type = 'button';
    headerBtn.className = 'yt-group-toggle';

    const groupActive = countGroupActive(group, controls);
    // All group accordions start closed whenever the popup opens; active
    // settings remain summarized via each group's count badge until expanded.
    headerBtn.setAttribute('aria-expanded', 'false');

    const titleWrap = document.createElement('span');
    titleWrap.className = 'yt-group-title';
    const titleText = document.createElement('span');
    titleText.textContent = group.title;
    const badge = document.createElement('span');
    badge.className = 'yt-group-badge';
    badge.hidden = groupActive === 0;
    badge.textContent = groupActive ? String(groupActive) : '';
    titleWrap.appendChild(titleText);
    titleWrap.appendChild(badge);

    headerBtn.appendChild(titleWrap);
    headerBtn.appendChild(createChevron());

    const body = document.createElement('div');
    body.className = 'yt-group-body';
    body.hidden = true;

    headerBtn.addEventListener('click', () => {
      const nextExpanded = headerBtn.getAttribute('aria-expanded') !== 'true';
      headerBtn.setAttribute('aria-expanded', nextExpanded ? 'true' : 'false');
      body.hidden = !nextExpanded;
      if (nextExpanded && typeof card.scrollIntoView === 'function') {
        card.scrollIntoView({ block: 'nearest' });
      }
    });

    for (const item of group.controls) {
      let subBox = null;
      if (item.children && item.children.length) {
        subBox = document.createElement('div');
        subBox.className = 'yt-subcontrols';
        for (const child of item.children) {
          subBox.appendChild(createControlRow(child, controls, null));
        }
      }
      body.appendChild(createControlRow(item, controls, subBox));
      if (subBox) body.appendChild(subBox);
    }

    card.appendChild(headerBtn);
    card.appendChild(body);
    ytGroupsRoot.appendChild(card);
    groupRefs.set(group.id, { badge, body, headerBtn });
  }

  syncControlsUi(controls);
}

async function initYoutubeControls() {
  if (!ytControlsApi || !ytGroupsRoot) return;
  const data = await chrome.storage.local.get(ytControlsApi.STORAGE_KEY);
  const normalized = ytControlsApi.normalizeYoutubeControls(
    data[ytControlsApi.STORAGE_KEY],
  );
  buildControlsUi(normalized);
  if (ytResetBtn) {
    ytResetBtn.addEventListener('click', () => {
      void resetYoutubeControls();
    });
  }
}

if (chrome.storage?.onChanged) {
  chrome.storage.onChanged.addListener((changes, areaName) => {
    if (areaName !== 'local') return;
    if (ytControlsApi && changes[ytControlsApi.STORAGE_KEY]) {
      syncControlsUi(changes[ytControlsApi.STORAGE_KEY].newValue);
    }
    if (changes[WEBSITE_CONTROLS_KEY]) {
      syncWebsiteControlsUi(changes[WEBSITE_CONTROLS_KEY].newValue);
    }
  });
}

void initWebsiteControls();
void initYoutubeControls();
void show();
// Keep the indicator live while the popup is open.
setInterval(() => {
  void show();
}, 2000);
