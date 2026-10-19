import { useCallback, useEffect, useState } from 'react';
import { api, desktop } from '../services/api';
import type { FocusSetup } from '../types';
export function useFocusSetup() {
  const [setup, setSetup] = useState<FocusSetup | null>(null);
  const [error, setError] = useState('');
  const [loading, setLoading] = useState(false);
  const refresh = useCallback(async () => {
    if (!desktop) return null;
    setLoading(true);
    try {
      const value = await api.focusSetup();
      setSetup(value);
      setError('');
      return value;
    } catch (e) {
      setError(String(e));
      return null;
    } finally {
      setLoading(false);
    }
  }, []);
  useEffect(() => {
    if (!desktop) return;
    let active = true;
    api
      .focusSetup()
      .then((value) => {
        if (active) setSetup(value);
      })
      .catch((e) => {
        if (active) setError(String(e));
      });
    return () => {
      active = false;
    };
  }, []);
  useEffect(() => {
    if (!desktop) return;
    const onFocus = () => void refresh();
    window.addEventListener('focus', onFocus);
    return () => window.removeEventListener('focus', onFocus);
  }, [refresh]);
  return { setup, error, loading, refresh };
}
