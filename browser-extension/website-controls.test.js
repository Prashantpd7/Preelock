import { describe, expect, it } from 'vitest';
import {
  ADULT_DOMAINS_DATA,
  ADULT_DOMAIN_LIST_METADATA,
} from './adult-domains.js';
import {
  ADULT_RULE_ID_BASE,
  DISTRACTING_DOMAINS,
  DISTRACTING_RULE_ID,
  WEBSITE_CONTROL_RULE_PRIORITY,
  compileWebsiteControlRules,
  countActiveWebsiteControls,
  isExcludedWebsiteControlsHost,
  isWebsiteControlRuleId,
  normalizeWebsiteControls,
  websiteControlBlockCategory,
} from './website-controls.js';

describe('Website Controls configuration, domain lists, and DNR compilation', () => {
  const adultDomains = ADULT_DOMAINS_DATA.split(' ').filter(Boolean);
  const adultSet = new Set(adultDomains);

  it('ships an offline MIT-licensed adult domain blocklist covering known adult sites without platform false positives', () => {
    expect(ADULT_DOMAIN_LIST_METADATA.license).toBe('MIT');
    expect(adultDomains.length).toBe(ADULT_DOMAIN_LIST_METADATA.count);
    expect(adultDomains.length).toBeGreaterThan(40000);

    // Core desktop adult domains must all be covered.
    for (const host of [
      'pornhub.com',
      'xvideos.com',
      'xnxx.com',
      'xhamster.com',
      'redtube.com',
      'youporn.com',
      'brazzers.com',
      'chaturbate.com',
      'stripchat.com',
      'livejasmin.com',
      'cam4.com',
      'onlyfans.com',
    ]) {
      expect(adultSet.has(host)).toBe(true);
    }

    // YouTube, WhatsApp, distracting apps, and mainstream dev/study tools stay excluded.
    for (const safe of [
      'youtube.com',
      'youtu.be',
      'whatsapp.com',
      'instagram.com',
      'facebook.com',
      'netflix.com',
      'google.com',
      'github.com',
      'wikipedia.org',
      'reddit.com',
      'x.com',
      'twitter.com',
    ]) {
      expect(adultSet.has(safe)).toBe(false);
    }
  });

  it('defines Distracting Websites strictly around initial list and excludes YouTube and WhatsApp', () => {
    expect(DISTRACTING_DOMAINS).toEqual([
      'facebook.com',
      'instagram.com',
      'netflix.com',
    ]);
    expect(isExcludedWebsiteControlsHost('youtube.com')).toBe(true);
    expect(isExcludedWebsiteControlsHost('www.youtube.com')).toBe(true);
    expect(isExcludedWebsiteControlsHost('m.youtube.com')).toBe(true);
    expect(isExcludedWebsiteControlsHost('youtu.be')).toBe(true);
    expect(isExcludedWebsiteControlsHost('whatsapp.com')).toBe(true);
    expect(isExcludedWebsiteControlsHost('web.whatsapp.com')).toBe(true);
    expect(isExcludedWebsiteControlsHost('instagram.com')).toBe(false);
  });

  it('matches domains and subdomains cleanly by enabled category', () => {
    const adultOnly = {
      blockAdultWebsites: true,
      blockDistractingWebsites: false,
    };
    expect(
      websiteControlBlockCategory(
        'https://pornhub.com/view',
        adultOnly,
        adultSet,
      ),
    ).toBe('website_adult');
    expect(
      websiteControlBlockCategory(
        'https://www.xvideos.com/',
        adultOnly,
        adultSet,
      ),
    ).toBe('website_adult');
    expect(
      websiteControlBlockCategory(
        'https://www.instagram.com/',
        adultOnly,
        adultSet,
      ),
    ).toBe(null);
    expect(
      websiteControlBlockCategory(
        'https://www.youtube.com/',
        adultOnly,
        adultSet,
      ),
    ).toBe(null);
    expect(
      websiteControlBlockCategory(
        'https://web.whatsapp.com/',
        adultOnly,
        adultSet,
      ),
    ).toBe(null);

    const distractingOnly = {
      blockAdultWebsites: false,
      blockDistractingWebsites: true,
    };
    expect(
      websiteControlBlockCategory(
        'https://www.instagram.com/reels/',
        distractingOnly,
        adultSet,
      ),
    ).toBe('website_distracting');
    expect(
      websiteControlBlockCategory(
        'https://m.facebook.com/',
        distractingOnly,
        adultSet,
      ),
    ).toBe('website_distracting');
    expect(
      websiteControlBlockCategory(
        'https://www.netflix.com/browse',
        distractingOnly,
        adultSet,
      ),
    ).toBe('website_distracting');
    expect(
      websiteControlBlockCategory(
        'https://www.youtube.com/watch?v=x',
        distractingOnly,
        adultSet,
      ),
    ).toBe(null);
    expect(
      websiteControlBlockCategory(
        'https://web.whatsapp.com/',
        distractingOnly,
        adultSet,
      ),
    ).toBe(null);
    expect(
      websiteControlBlockCategory(
        'https://www.pornhub.com/',
        distractingOnly,
        adultSet,
      ),
    ).toBe(null);
    expect(
      websiteControlBlockCategory(
        'http://127.0.0.1:47832/sync',
        {
          blockAdultWebsites: true,
          blockDistractingWebsites: true,
        },
        adultSet,
      ),
    ).toBe(null);
  });

  it('compiles disjoint, quota-safe DNR rules with additive priority above session allows', () => {
    expect(
      compileWebsiteControlRules(normalizeWebsiteControls(null), adultDomains),
    ).toEqual([]);
    expect(countActiveWebsiteControls({ blockAdultWebsites: true })).toBe(1);

    const both = compileWebsiteControlRules(
      { blockAdultWebsites: true, blockDistractingWebsites: true },
      adultDomains,
    );
    expect(both.length).toBeLessThan(20); // well within MV3 dynamic rule limits
    expect(both.every((r) => isWebsiteControlRuleId(r.id))).toBe(true);
    expect(
      both.every((r) => r.priority === WEBSITE_CONTROL_RULE_PRIORITY),
    ).toBe(true);
    expect(both.every((r) => r.action.type === 'block')).toBe(true);

    const distractingRule = both.find((r) => r.id === DISTRACTING_RULE_ID);
    expect(distractingRule).toBeDefined();
    expect(distractingRule.condition.requestDomains).toEqual([
      'facebook.com',
      'instagram.com',
      'netflix.com',
    ]);

    const adultRules = both.filter((r) => r.id >= ADULT_RULE_ID_BASE);
    expect(adultRules.length).toBe(Math.ceil(adultDomains.length / 5000));
    const flatDomains = adultRules.flatMap((r) => r.condition.requestDomains);
    expect(flatDomains.length).toBe(adultDomains.length);
  });
});
