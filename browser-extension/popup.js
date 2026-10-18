const status = document.getElementById('status');
async function show() {
  const data = await chrome.storage.session.get([
    'policy',
    'error',
    'last_sync',
  ]);
  const { token, disabled } = await chrome.storage.local.get([
    'token',
    'disabled',
  ]);
  status.textContent = !token
    ? 'Paste the connection code from VYSTRIQ Settings.'
    : disabled
      ? 'Disconnected. Paste your code and click Connect.'
      : data.error ||
        (data.policy
          ? 'Website blocking is on.'
          : data.last_sync && Date.now() - data.last_sync < 45000
            ? 'Connected. Start a session in VYSTRIQ.'
            : 'Open VYSTRIQ, then click Reconnect.');
}
async function sync() {
  status.textContent = 'Connecting…';
  await chrome.runtime.sendMessage({ type: 'sync' });
  await show();
}
document.getElementById('pair').addEventListener('submit', async (event) => {
  event.preventDefault();
  const token = document.getElementById('token').value.trim();
  if (!/^[a-f0-9]{64}$/i.test(token)) {
    status.textContent = 'Copy the full connection code from VYSTRIQ Settings.';
    return;
  }
  const { client = crypto.randomUUID() } =
    await chrome.storage.local.get('client');
  await chrome.storage.local.set({ token, client, disabled: false });
  document.getElementById('token').value = '';
  await sync();
});
document.getElementById('retry').addEventListener('click', sync);
document.getElementById('release').addEventListener('click', async () => {
  if (document.getElementById('confirm').value !== 'END FOCUS') {
    status.textContent = 'Type END FOCUS exactly.';
    return;
  }
  await chrome.runtime.sendMessage({
    type: 'release',
    confirmation: 'END FOCUS',
  });
  await show();
});
void show();

document.getElementById('self-test').addEventListener('click', async () => {
  const result = await chrome.runtime.sendMessage({ type: 'self_test' });
  status.textContent = result.result || result.error;
});
