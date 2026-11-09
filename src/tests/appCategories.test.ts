import { describe, expect, it } from 'vitest';
import {
  appCategory,
  type CategoryOverrides,
} from '../services/appCategories';

describe('appCategory', () => {
  it('labels social and entertainment apps Distracting', () => {
    expect(appCategory('app', 'com.instagram.ios', 'Instagram')).toBe(
      'Distracting',
    );
    expect(appCategory('app', 'com.snapchat.ios', 'Snapchat')).toBe(
      'Distracting',
    );
    expect(appCategory('app', 'com.netflix.ios', 'Netflix')).toBe(
      'Distracting',
    );
  });

  it('labels browsers Others even when the vendor is a work tool', () => {
    expect(appCategory('app', 'ai.perplexity.comet', 'Comet')).toBe('Others');
    expect(appCategory('app', 'com.brave.Browser', 'Brave Browser')).toBe(
      'Others',
    );
  });

  it('labels work and developer tools Productive', () => {
    expect(
      appCategory(
        'app',
        '/Users/Prashant/My_Workspace/Preelock/src-tauri/target/debug/preelock',
        'preelock',
      ),
    ).toBe('Productive');
    expect(appCategory('app', 'com.meta.endo', 'Muse')).toBe('Productive');
    expect(appCategory('app', 'com.freebuff.desktop', 'Freebuff')).toBe(
      'Productive',
    );
    expect(appCategory('app', 'com.apple.Terminal', 'Terminal')).toBe(
      'Productive',
    );
  });

  it('labels WhatsApp, system apps, and unknown apps Others', () => {
    expect(appCategory('app', 'com.whatsapp.ios', 'WhatsApp')).toBe('Others');
    expect(appCategory('app', 'com.apple.finder', 'Finder')).toBe('Others');
    expect(appCategory('app', 'com.apple.loginwindow', 'loginwindow')).toBe(
      'Others',
    );
    expect(appCategory('app', 'com.example.something', 'Something')).toBe(
      'Others',
    );
  });

  it('categorizes websites by domain, including subdomains', () => {
    expect(appCategory('site', 'youtube.com', 'youtube.com')).toBe(
      'Distracting',
    );
    expect(appCategory('site', 'www.youtube.com', 'www.youtube.com')).toBe(
      'Distracting',
    );
    expect(appCategory('site', 'github.com', 'github.com')).toBe('Productive');
    expect(appCategory('site', 'stackoverflow.com', 'stackoverflow.com')).toBe(
      'Productive',
    );
    expect(appCategory('site', 'example.com', 'example.com')).toBe('Others');
  });
});

describe('appCategory overrides', () => {
  it('lets a saved correction beat the built-in mapping for apps', () => {
    // Comet is Others by default; a correction moves it to Distracting.
    expect(
      appCategory('app', 'ai.perplexity.comet', 'Comet', {
        'ai.perplexity.comet': 'Distracting',
      }),
    ).toBe('Distracting');
    // Instagram is Distracting by default; a correction moves it back.
    expect(
      appCategory('app', 'com.instagram.ios', 'Instagram', {
        'com.instagram.ios': 'Productive',
      }),
    ).toBe('Productive');
  });

  it('keys website overrides by domain, including subdomains', () => {
    expect(
      appCategory('site', 'www.youtube.com', 'www.youtube.com', {
        'youtube.com': 'Productive',
      }),
    ).toBe('Productive');
  });

  it('ignores override values that are not a known category', () => {
    const corrupt = {
      'com.instagram.ios': 'Bogus',
    } as unknown as CategoryOverrides;
    expect(appCategory('app', 'com.instagram.ios', 'Instagram', corrupt)).toBe(
      'Distracting',
    );
  });
});
