import { api } from './api';

// Screen Time row icons. App icons come from the backend (the real macOS
// app icon as a PNG data URL) and are cached by identity so a long list
// does not re-fetch on every render; website icons are the site's own
// favicon.ico. Anything that fails to load falls back to a letter tile.
const iconCache = new Map<string, string | null>();
const iconPending = new Map<string, Promise<string | null>>();

export function faviconUrl(domain: string): string {
  return `https://${domain}/favicon.ico`;
}

export function appIcon(identity: string): Promise<string | null> {
  const cached = iconCache.get(identity);
  if (cached !== undefined) return Promise.resolve(cached);
  const flight = iconPending.get(identity);
  if (flight) return flight;
  const run = api
    .appIcon(identity)
    .then((b64) => {
      // The backend returns raw base64 PNG bytes; the <img> needs a
      // data URL.
      const url = b64 ? `data:image/png;base64,${b64}` : null;
      iconCache.set(identity, url);
      iconPending.delete(identity);
      return url;
    })
    .catch(() => {
      iconCache.set(identity, null);
      iconPending.delete(identity);
      return null;
    });
  iconPending.set(identity, run);
  return run;
}

export function initialLetter(name: string): string {
  const letter = name.trim().charAt(0);
  return letter ? letter.toUpperCase() : '?';
}
