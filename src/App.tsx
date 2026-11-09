import { useEffect, useState } from 'react';
import { HashRouter, Route, Routes } from 'react-router-dom';
import { FrozenOverlay, FrozenOverlayWindow } from './components/FrozenOverlay';
import {
  BlockedAppOverlay,
  BlockedAppOverlayWindow,
} from './components/BlockedAppOverlay';
import { AppLayout } from './layouts/AppLayout';
export function App() {
  // Step 12: the frozen overlay window loads this same frontend.
  // It renders the overlay (not the app shell) when its own Tauri
  // window label is `frozen`. The "why am I blocked" app page works
  // the same way on the `blocked-app` label. The check is async and
  // best-effort: in a browser preview there is no Tauri window, so
  // it stays 'app' and the normal app renders. The `/frozen` and
  // `/blocked-app` routes are the same overlays for browser-preview
  // testing.
  const [overlay, setOverlay] = useState<'app' | 'frozen' | 'blocked-app'>(
    'app',
  );
  useEffect(() => {
    let live = true;
    void import('@tauri-apps/api/window')
      .then(({ getCurrentWindow }) => getCurrentWindow().label)
      .then((label) => {
        if (!live) return;
        if (label === 'frozen') setOverlay('frozen');
        else if (label === 'blocked-app') setOverlay('blocked-app');
      })
      .catch(() => {});
    return () => {
      live = false;
    };
  }, []);
  if (overlay === 'frozen') return <FrozenOverlayWindow />;
  if (overlay === 'blocked-app') return <BlockedAppOverlayWindow />;
  return (
    <HashRouter>
      <Routes>
        <Route path="/frozen" element={<FrozenOverlay />} />
        <Route path="/blocked-app" element={<BlockedAppOverlay />} />
        <Route path="*" element={<AppLayout />} />
      </Routes>
    </HashRouter>
  );
}
