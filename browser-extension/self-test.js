import { compileRules } from './rules.js';
// Tests Chromium's installed matcher without navigating to or contacting a site.
export async function selfTest() {
  const existing = await chrome.declarativeNetRequest.getSessionRules();
  if (existing.length)
    throw new Error(
      'End the active session before running the browser self-test.',
    );
  const check = async (mode, domains, url, expected) => {
    const old = await chrome.declarativeNetRequest.getSessionRules();
    await chrome.declarativeNetRequest.updateSessionRules({
      removeRuleIds: old.map((r) => r.id),
      addRules: compileRules({ mode, domains }).map((rule) => ({
        ...rule,
        condition: { ...rule.condition, tabIds: [2147483646] },
      })),
    });
    const result = await chrome.declarativeNetRequest.testMatchOutcome({
      url,
      type: 'main_frame',
      tabId: 2147483646,
    });
    if (Boolean(result.matchedRules.length) !== expected)
      throw new Error(`${mode} matcher check failed`);
  };
  try {
    await check(
      'blocklist',
      ['vystriq-test.invalid'],
      'https://vystriq-test.invalid/',
      true,
    );
    await check(
      'blocklist',
      ['vystriq-test.invalid'],
      'https://allowed.invalid/',
      false,
    );
    await check(
      'allowlist',
      ['allowed.invalid'],
      'https://allowed.invalid/',
      false,
    );
    await check(
      'allowlist',
      ['allowed.invalid'],
      'https://vystriq-test.invalid/',
      true,
    );
    await check('offline', [], 'http://127.0.0.1:47832/sync', false);
    await check('offline', [], 'https://vystriq-test.invalid/', true);
  } finally {
    const rules = await chrome.declarativeNetRequest.getSessionRules();
    await chrome.declarativeNetRequest.updateSessionRules({
      removeRuleIds: rules.map((r) => r.id),
    });
  }
  return 'Passed six installed browser matcher checks; test rules removed. No network requests sent.';
}
