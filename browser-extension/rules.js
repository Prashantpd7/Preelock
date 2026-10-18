// Pure rule compiler: no inspection of page content or URL history.
export function compileRules(policy) {
  if (!policy) return [];
  const types = [
    'main_frame',
    'sub_frame',
    'stylesheet',
    'script',
    'image',
    'font',
    'object',
    'xmlhttprequest',
    'ping',
    'csp_report',
    'media',
    'websocket',
    'webtransport',
    'webbundle',
    'other',
  ];
  const local = ['localhost', '127.0.0.1', '[::1]'];
  const safe = policy.domains.filter(
    (d) => /^[a-z0-9](?:[a-z0-9.-]*[a-z0-9])?$/.test(d) && d.includes('.'),
  );
  if (safe.length !== policy.domains.length)
    throw new Error('Invalid policy domains');
  if (policy.mode === 'blocklist')
    return safe.map((domain, index) => ({
      id: index + 1,
      priority: 1,
      action: { type: 'block' },
      condition: { requestDomains: [domain], resourceTypes: types },
    }));
  if (!['allowlist', 'offline'].includes(policy.mode))
    throw new Error('Unknown network policy');
  // Exempt local loopback for pairing/recovery. No global proxy, DNS or firewall changes.
  return [
    {
      id: 1,
      priority: 1,
      action: { type: 'block' },
      condition: {
        regexFilter: '^(https?|wss?)://',
        excludedRequestDomains: [
          ...local,
          ...(policy.mode === 'allowlist' ? safe : []),
        ],
        resourceTypes: types,
      },
    },
  ];
}
export function leaseValid(policy, now = Date.now()) {
  return Boolean(
    policy &&
    Number.isFinite(policy.expires_at) &&
    policy.expires_at > now &&
    policy.expires_at <= now + 70_000,
  );
}
