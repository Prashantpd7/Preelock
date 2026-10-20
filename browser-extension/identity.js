export const browserNames = {
  'ai.perplexity.comet': 'Comet',
  'com.google.chrome': 'Chrome',
  'com.microsoft.edgemac': 'Edge',
  'com.brave.browser': 'Brave',
  'com.vivaldi.vivaldi': 'Vivaldi',
  'org.chromium.chromium': 'Chromium',
};
export async function browserIdentity() {
  const { browser_choice } = await chrome.storage.local.get('browser_choice');
  if (Object.hasOwn(browserNames, browser_choice ?? '')) return browser_choice;
  // No stored choice: do NOT guess. detectBrowser() cannot tell Brave from
  // Chrome inside a service worker (navigator.brave and userAgentData are
  // unavailable there), so a guess would misidentify the browser and the
  // app would attribute this client to the wrong browser. '' means
  // "unidentified" — the app will not match it to any browser, and the
  // popup will ask the user to pick.
  return '';
}
