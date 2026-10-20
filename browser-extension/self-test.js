import { compileRules } from './rules.js';
// Tests Chromium's installed matcher without navigating to or contacting a site.
export async function selfTest() {
  const existing = await chrome.declarativeNetRequest.getSessionRules();
  if (existing.length)
    throw new Error(
      'End the active session before running the browser self-test.',
    );
  const check = async (mode, domains, url, expected, type = 'main_frame') => {
    const old = await chrome.declarativeNetRequest.getSessionRules();
    const testing = compileRules({ mode, domains }).map((rule) => ({
      ...rule,
      condition: { ...rule.condition, tabIds: [2147483646] },
    }));
    await chrome.declarativeNetRequest.updateSessionRules({
      removeRuleIds: old.map((r) => r.id),
      addRules: testing,
    });
    const result = await chrome.declarativeNetRequest.testMatchOutcome({
      url,
      type,
      tabId: 2147483646,
    });
    const ids = new Set(result.matchedRules.map((rule) => rule.ruleId));
    const winner = testing
      .filter((rule) => ids.has(rule.id))
      .sort(
        (a, b) =>
          b.priority - a.priority || (a.action.type === 'allow' ? -1 : 1),
      )[0];
    if ((winner?.action.type === 'block') !== expected)
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
    await check(
      'allowlist',
      ['github.com', 'whatsapp.com'],
      'https://www.youtube.com/watch?v=test',
      true,
    );
    await check(
      'offline',
      [],
      'http://127.0.0.1:47832/sync',
      false,
      'xmlhttprequest',
    );
  } finally {
    const rules = await chrome.declarativeNetRequest.getSessionRules();
    await chrome.declarativeNetRequest.updateSessionRules({
      removeRuleIds: rules.map((r) => r.id),
    });
  }
  return 'Passed eight installed browser matcher checks; test rules removed. No network requests sent.';
}
