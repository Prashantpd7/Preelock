import { leaseValid } from './rules.js';
import { browserIdentity, browserNames } from './identity.js';
const connectedView = document.getElementById('connected-view');
const pairView = document.getElementById('pair-view');
const health = document.getElementById('health');
const pairStatus = document.getElementById('pair-status');
const browserLine = document.getElementById('browser-line');
const disconnectBtn = document.getElementById('disconnect');
const identifyView = document.getElementById('identify-view');
const saveBrowserBtn = document.getElementById('save-browser');
async function show() {
  const data = await chrome.storage.session.get([
    'policy',
    'error',
    'last_sync',
  ]);
  const { token, disabled, browser_choice } = await chrome.storage.local.get([
    'token',
    'disabled',
    'browser_choice',
  ]);
  const connected = Boolean(token) && !disabled && !data.error;
  connectedView.hidden = !connected;
  pairView.hidden = connected;
  if (!connected) {
    pairStatus.textContent =
      data.error ||
      'Paste the connection code from Preelock Settings, then connect.';
    return;
  }
  // Already paired but the browser was never identified (e.g. paired
  // before the picker existed): ask now, otherwise the app cannot tell
  // which browser this is.
  identifyView.hidden = Boolean(browser_choice);
  const detected = await browserIdentity();
  browserLine.textContent = detected
    ? `Browser: ${browserNames[detected] || detected}`
    : 'Browser: not detected';
  const blocking = leaseValid(data.policy);
  const fresh = data.last_sync && Date.now() - data.last_sync < 45000;
  health.textContent = '';
  const dot = document.createElement('span');
  dot.className = `dot ${blocking ? 'ok' : 'idle'}`;
  health.appendChild(dot);
  health.appendChild(
    document.createTextNode(
      blocking
        ? 'Working — blocking sites exactly as the desktop app instructs.'
        : fresh
          ? 'Connected — start a session in Preelock to begin blocking.'
          : 'Connected — waiting for the desktop app…',
    ),
  );
  // Warn if the extension cannot run in Incognito windows, where blocking
  // would silently not apply.
  try {
    const allowed = await chrome.extension.isAllowedIncognitoAccess();
    if (!allowed) {
      const warn = document.createElement('p');
      warn.className = 'incognito-warn';
      warn.textContent =
        'Blocking does not apply in Incognito windows. Enable "Allow in Incognito" for Preelock in chrome://extensions.';
      health.appendChild(warn);
    }
  } catch {}
}
async function sync() {
  pairStatus.textContent = 'Connecting…';
  await chrome.runtime.sendMessage({ type: 'sync' });
  await show();
}
document.getElementById('pair').addEventListener('submit', async (event) => {
  event.preventDefault();
  const field = document.getElementById('token');
  const token = field.value.trim();
  if (!/^[a-f0-9]{64}$/i.test(token)) {
    pairStatus.textContent =
      'Copy the full connection code from Preelock Settings.';
    return;
  }
  const { client = crypto.randomUUID() } =
    await chrome.storage.local.get('client');
  const browser_choice = document.querySelector(
    'input[name=browser]:checked',
  )?.value;
  if (!browser_choice) {
    pairStatus.textContent = 'Choose the browser you are using.';
    return;
  }
  await chrome.storage.local.set({
    token,
    client,
    disabled: false,
    browser_choice,
  });
  field.value = '';
  await sync();
});
disconnectBtn.addEventListener('click', async () => {
  // Tell the app first (still authenticated), then drop the token. Keep
  // this browser's client id, so pairing again with a fresh code
  // reconnects the same browser without a conflict.
  try {
    await chrome.runtime.sendMessage({ type: 'disconnect' });
  } catch {}
  await chrome.storage.local.remove(['token']);
  await chrome.storage.session.remove(['error', 'policy']);
  await show();
});
const choices = document.getElementById('browser-choices');
const choicesConnected = document.getElementById('browser-choices-connected');
for (const [id, name] of Object.entries(browserNames)) {
  const label = document.createElement('label');
  const input = document.createElement('input');
  input.type = 'radio';
  input.name = 'browser';
  input.value = id;
  input.required = true;
  label.append(input, document.createTextNode(name));
  choices.append(label);
  const label2 = document.createElement('label');
  const input2 = document.createElement('input');
  input2.type = 'radio';
  input2.name = 'browser-connected';
  input2.value = id;
  label2.append(input2, document.createTextNode(name));
  choicesConnected.append(label2);
}
saveBrowserBtn.addEventListener('click', async () => {
  const picked = document.querySelector(
    'input[name=browser-connected]:checked',
  )?.value;
  if (!picked) return;
  await chrome.storage.local.set({ browser_choice: picked });
  await chrome.runtime.sendMessage({ type: 'sync' });
  await show();
});
void show();
// Keep the indicator live while the popup is open.
setInterval(() => {
  void show();
}, 2000);
