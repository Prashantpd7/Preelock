import { beforeEach, describe, expect, it, vi } from 'vitest';

const invokeMock = vi.hoisted(() => vi.fn());
vi.mock('@tauri-apps/api/core', () => ({
  invoke: invokeMock,
  isTauri: () => true,
}));

import { appIcon, faviconUrl, initialLetter } from '../services/appIcons';

describe('faviconUrl', () => {
  it('points at the site’s own favicon.ico', () => {
    expect(faviconUrl('example.com')).toBe('https://example.com/favicon.ico');
  });
});

describe('initialLetter', () => {
  it('uppercases the first non-space letter', () => {
    expect(initialLetter('brave')).toBe('B');
    expect(initialLetter('  comet')).toBe('C');
  });
  it('falls back to ? for an empty name', () => {
    expect(initialLetter('')).toBe('?');
    expect(initialLetter('   ')).toBe('?');
  });
});

describe('appIcon', () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it('returns the backend icon as a data URL and fetches once per identity', async () => {
    invokeMock.mockResolvedValue('AAA');
    await expect(appIcon('com.example.one')).resolves.toBe(
      'data:image/png;base64,AAA',
    );
    await expect(appIcon('com.example.one')).resolves.toBe(
      'data:image/png;base64,AAA',
    );
    expect(invokeMock).toHaveBeenCalledTimes(1);
    expect(invokeMock).toHaveBeenCalledWith('get_app_icon', {
      identity: 'com.example.one',
    });
  });

  it('returns null and caches it when the backend has no icon', async () => {
    invokeMock.mockResolvedValue(null);
    await expect(appIcon('com.example.two')).resolves.toBeNull();
    await expect(appIcon('com.example.two')).resolves.toBeNull();
    expect(invokeMock).toHaveBeenCalledTimes(1);
  });

  it('returns null when the backend call fails', async () => {
    invokeMock.mockRejectedValue(new Error('unavailable'));
    await expect(appIcon('com.example.three')).resolves.toBeNull();
  });
});
