import { useEffect } from 'react';
import { useAppStore } from '../state/store';
export function useApplication() {
  const largeText = useAppStore((s) => s.data.preferences.large_text);
  useEffect(() => {
    document.documentElement.dataset.textSize = largeText
      ? 'large'
      : 'standard';
  }, [largeText]);
  const theme = useAppStore((s) => s.data.preferences.theme);
  useEffect(() => {
    void useAppStore.getState().initialize();
    const timer = window.setInterval(() => {
      void useAppStore.getState().refresh();
    }, 1000);
    return () => window.clearInterval(timer);
  }, []);
  useEffect(() => {
    const media = window.matchMedia('(prefers-color-scheme: dark)');
    const apply = () => {
      document.documentElement.dataset.theme =
        theme === 'system' ? (media.matches ? 'dark' : 'light') : theme;
    };
    apply();
    media.addEventListener('change', apply);
    return () => media.removeEventListener('change', apply);
  }, [theme]);
}
