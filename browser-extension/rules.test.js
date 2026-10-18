import { describe, it, expect } from 'vitest';
import { compileRules, leaseValid } from './rules.js';
describe('real browser policy rule generation', () => {
  it('makes blocklist domain rules with subdomain matching', () => {
    const rules = compileRules({
      mode: 'blocklist',
      domains: ['example.com', 'reddit.com'],
    });
    expect(rules).toHaveLength(2);
    expect(rules[0].condition.requestDomains).toEqual(['example.com']);
    expect(rules[0].action.type).toBe('block');
  });
  it('allows only selected domains and recovery loopback in strict mode', () => {
    const [rule] = compileRules({ mode: 'allowlist', domains: ['docs.rs'] });
    expect(rule.condition.excludedRequestDomains).toContain('docs.rs');
    expect(rule.condition.excludedRequestDomains).toContain('127.0.0.1');
    expect(rule.condition.regexFilter).toBe('^(https?|wss?)://');
  });
  it('offline mode does not retain approved public destinations', () => {
    const [rule] = compileRules({ mode: 'offline', domains: ['docs.rs'] });
    expect(rule.condition.excludedRequestDomains).not.toContain('docs.rs');
  });
  it('removes every rule for cleanup and rejects malformed domains', () => {
    expect(compileRules(null)).toEqual([]);
    expect(() =>
      compileRules({ mode: 'blocklist', domains: ['example.com/path'] }),
    ).toThrow();
  });
  it('rejects expired and excessively long policy leases', () => {
    expect(leaseValid({ expires_at: 1000 }, 2000)).toBe(false);
    expect(leaseValid({ expires_at: 100000 }, 2000)).toBe(false);
    expect(leaseValid({ expires_at: 5000 }, 2000)).toBe(true);
  });
});
