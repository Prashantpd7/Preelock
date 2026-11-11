import { describe, it, expect } from 'vitest';
import {
  browserIdentity,
  browserNames,
  detectFromSignals,
  isSupportedBrowser,
} from './identity.js';
// These are the real signal shapes each browser produces. Brave and
// Chrome share a user agent exactly; Brave exposes navigator.brave in
// documents only. Edge, Comet and Chromium differ in the brand list.
const ua = {
  chrome:
    'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36',
  edge: 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36 Edg/131.0.0.0',
  vivaldi:
    'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36 Vivaldi/6.9.3447.48',
  chromium:
    'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chromium/131.0.0.0 Safari/537.36',
  opera:
    'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36 OPR/115.0.0.0',
  comet:
    'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36 Comet/1.2.3',
  firefox:
    'Mozilla/5.0 (Macintosh; Intel Mac OS X 14.0; rv:130.0) Gecko/20100101 Firefox/130.0',
};
const chromeBrands = [{ brand: 'Chromium' }, { brand: 'Google Chrome' }];
describe('browser identification signals', () => {
  it('identifies Brave from its own API, even with Chrome’s user agent', () => {
    expect(
      detectFromSignals(
        { brave: true, brands: chromeBrands, userAgent: ua.chrome },
        true,
      ),
    ).toBe('com.brave.browser');
  });
  it('identifies Chrome only where the Brave probe is conclusive', () => {
    expect(
      detectFromSignals(
        { brave: false, brands: chromeBrands, userAgent: ua.chrome },
        true,
      ),
    ).toBe('com.google.chrome');
    // In a worker, the same signals could be Brave: never claim Chrome.
    expect(
      detectFromSignals({ brands: chromeBrands, userAgent: ua.chrome }, false),
    ).toBe('');
  });
  it('identifies Edge, Vivaldi and Comet from brands and user agent', () => {
    expect(
      detectFromSignals(
        {
          brands: [{ brand: 'Chromium' }, { brand: 'Microsoft Edge' }],
          userAgent: ua.edge,
        },
        false,
      ),
    ).toBe('com.microsoft.edgemac');
    // Edge's UA token works without a brand list (a worker context).
    expect(detectFromSignals({ userAgent: ua.edge }, false)).toBe(
      'com.microsoft.edgemac',
    );
    expect(detectFromSignals({ userAgent: ua.vivaldi }, false)).toBe(
      'com.vivaldi.vivaldi',
    );
    expect(detectFromSignals({ userAgent: ua.comet }, false)).toBe(
      'ai.perplexity.comet',
    );
    expect(
      detectFromSignals(
        { brands: [{ brand: 'Comet' }], userAgent: ua.chrome },
        false,
      ),
    ).toBe('ai.perplexity.comet');
  });
  it('identifies Chromium from its own brand or user agent', () => {
    expect(
      detectFromSignals(
        { brands: [{ brand: 'Chromium' }], userAgent: ua.chrome },
        true,
      ),
    ).toBe('org.chromium.chromium');
    expect(detectFromSignals({ userAgent: ua.chromium }, true)).toBe(
      'org.chromium.chromium',
    );
  });
  it('never assigns an identity it cannot verify', () => {
    // Opera is Chromium-based but not supported: not Chrome either.
    expect(detectFromSignals({ userAgent: ua.opera }, true)).toBe('');
    expect(detectFromSignals({ userAgent: ua.firefox }, true)).toBe('');
    expect(detectFromSignals({ userAgent: '' }, true)).toBe('');
    expect(detectFromSignals({}, true)).toBe('');
    // The supported set is exactly the browsers the app can drive.
    expect(Object.keys(browserNames)).toEqual([
      'ai.perplexity.comet',
      'com.google.chrome',
      'com.microsoft.edgemac',
      'com.brave.browser',
      'com.vivaldi.vivaldi',
      'org.chromium.chromium',
    ]);
    expect(isSupportedBrowser('com.google.chrome')).toBe(true);
    expect(isSupportedBrowser('com.operasoftware.opera')).toBe(false);
    expect(isSupportedBrowser('')).toBe(false);
    expect(isSupportedBrowser(undefined)).toBe(false);
  });
  it('falls back to the identity the popup detected, never to a guess', async () => {
    const local = {};
    globalThis.chrome = {
      storage: {
        local: {
          get: async (keys) => {
            const wanted = Array.isArray(keys) ? keys : [keys];
            return Object.fromEntries(
              wanted.filter((k) => k in local).map((k) => [k, local[k]]),
            );
          },
          set: async (values) => Object.assign(local, values),
        },
      },
    };
    try {
      // Nothing stored and nothing provable in this context.
      await expect(browserIdentity()).resolves.toBe('');
      // The popup's document detection is reused by the worker.
      local.browser_identity = 'com.brave.browser';
      await expect(browserIdentity()).resolves.toBe('com.brave.browser');
      // A legacy manual pick still keeps an existing pairing alive…
      delete local.browser_identity;
      local.browser_choice = 'ai.perplexity.comet';
      await expect(browserIdentity()).resolves.toBe('ai.perplexity.comet');
      // …but an unrecognised stored value is never reported.
      local.browser_choice = 'com.operasoftware.opera';
      await expect(browserIdentity()).resolves.toBe('');
    } finally {
      delete globalThis.chrome;
    }
  });
});
