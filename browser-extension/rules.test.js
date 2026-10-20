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
    const rules = compileRules({ mode: 'allowlist', domains: ['docs.rs'] });
    // Allow rules (priority 2) for the domain + localhost, then a
    // catch-all block rule (priority 1).
    const allows = rules.filter((r) => r.action.type === 'allow');
    const blocks = rules.filter((r) => r.action.type === 'block');
    expect(
      allows.some((r) => r.condition.requestDomains.includes('docs.rs')),
    ).toBe(true);
    expect(
      allows.some((r) => r.condition.requestDomains.includes('127.0.0.1')),
    ).toBe(true);
    expect(blocks).toHaveLength(1);
    expect(blocks[0].condition.regexFilter).toBe('^(https?|wss?)://');
    expect(blocks[0].priority).toBe(1);
    expect(allows[0].priority).toBe(2);
  });
  it('allowlist blocks navigation only so allowed pages load their assets', () => {
    const rules = compileRules({ mode: 'allowlist', domains: ['docs.rs'] });
    for (const rule of rules) {
      expect(rule.condition.resourceTypes).toEqual(['main_frame', 'sub_frame']);
    }
  });
  it('offline keeps filtering every request type', () => {
    const rules = compileRules({ mode: 'offline', domains: [] });
    const blocks = rules.filter((r) => r.action.type === 'block');
    expect(blocks).toHaveLength(1);
    expect(blocks[0].condition.resourceTypes).toContain('main_frame');
    expect(blocks[0].condition.resourceTypes).toContain('stylesheet');
    for (const rule of rules.filter((r) => r.action.type === 'allow')) {
      expect(rule.condition.resourceTypes).toContain('xmlhttprequest');
    }
    const [blockRule] = compileRules({
      mode: 'blocklist',
      domains: ['example.com'],
    });
    expect(blockRule.condition.resourceTypes).toContain('main_frame');
    expect(blockRule.condition.resourceTypes).toContain('stylesheet');
  });
  it('offline mode does not retain approved public destinations', () => {
    const rules = compileRules({ mode: 'offline', domains: ['docs.rs'] });
    const allows = rules.filter((r) => r.action.type === 'allow');
    expect(
      allows.some((r) => r.condition.requestDomains.includes('docs.rs')),
    ).toBe(false);
    expect(
      allows.some((r) => r.condition.requestDomains.includes('127.0.0.1')),
    ).toBe(true);
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
