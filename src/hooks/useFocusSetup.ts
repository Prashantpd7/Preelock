import { useCallback, useEffect, useState } from 'react';
import { api, desktop } from '../services/api';
import type { FocusSetup } from '../types';

export function useFocusSetup(browserId?: string) {
  const [setup, setSetup] = useState<FocusSetup | null>(null);
  const [error, setError] = useState('');
  const [loading, setLoading] = useState(false);

  // `forBrowser` lets a caller re-read the setup for a browser it just
  // chose (the session wizard opens it and waits for it to appear)
  // without the answer being attributed to a stale selection.
  const refresh = useCallback(
    async (forBrowser?: string) => {
      if (!desktop) return null;
      setLoading(true);
      let attempts = 0;
      let lastError = '';
      while (attempts < 3) {
        attempts += 1;
        try {
          const value = await api.focusSetup(forBrowser ?? browserId);
          setSetup(value);
          setError('');
          setLoading(false);
          return value;
        } catch (e) {
          lastError = String(e);
          if (attempts < 3) {
            await new Promise((r) => setTimeout(r, 200 * attempts));
          }
        }
      }
      setError(lastError);
      setLoading(false);
      return null;
    },
    [browserId],
  );

  useEffect(() => {
    if (!desktop) return;
    let active = true;
    let timer: ReturnType<typeof setTimeout> | null = null;
    let attempt = 0;

    const fetchSetup = async () => {
      try {
        const value = await api.focusSetup(browserId);
        if (!active) return;
        setSetup(value);
        setError('');
        // If pairing token is missing/empty during initial startup, retry in background
        if (!value.pairing_token && attempt < 3) {
          attempt += 1;
          timer = setTimeout(fetchSetup, 300 * attempt);
        }
      } catch (e) {
        if (!active) return;
        if (attempt < 3) {
          attempt += 1;
          timer = setTimeout(fetchSetup, 300 * attempt);
        } else {
          setError(String(e));
        }
      }
    };

    void fetchSetup();

    return () => {
      active = false;
      if (timer) clearTimeout(timer);
    };
  }, [browserId]);

  useEffect(() => {
    if (!desktop) return;
    const onFocus = () => void refresh();
    window.addEventListener('focus', onFocus);
    return () => window.removeEventListener('focus', onFocus);
  }, [refresh]);

  return { setup, error, loading, refresh };
}
