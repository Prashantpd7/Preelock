import { useCallback, useEffect, useState } from 'react';
import { api, desktop } from '../services/api';
import type { FocusSetup } from '../types';
export function useFocusSetup() {
  const [setup, setSetup] = useState<FocusSetup | null>(null);
  const [error, setError] = useState('');
  const [loading, setLoading] = useState(false);
  const refresh = useCallback(async () => {
    if (!desktop) return;
    setLoading(true);
    try {
      setSetup(await api.focusSetup());
      setError('');
    } catch (e) {
      setError(String(e));
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
  return { setup, error, loading, refresh };
}
