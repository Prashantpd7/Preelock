import { useState } from 'react';
import { PermissionsPanel } from '../components/PermissionsPanel';
import { PageHeading } from '../components/ui';
import { useAppStore } from '../state/store';
import { desktop } from '../services/api';
import type { Preferences } from '../types';
export function Settings() {
  const { data, execute, busy } = useAppStore();
  const [theme, setTheme] = useState(data.preferences.theme);
  const [minutes, setMinutes] = useState(data.preferences.default_minutes);
  const [saved, setSaved] = useState(false);
  return (
    <>
      <PageHeading
        eyebrow=""
        title="Settings"
        description="Make VYSTRIQ work for you."
      />
      <form
        className="panel form-panel"
        onSubmit={async (e) => {
          e.preventDefault();
          setSaved(
            await execute({
              type: 'set_preferences',
              theme,
              default_minutes: minutes,
            }),
          );
        }}
      >
        <h2>General</h2>
        <div className="form-grid">
          <label className="grow">
            Appearance
            <select
              value={theme}
              onChange={(e) => {
                setTheme(e.target.value as Preferences['theme']);
                setSaved(false);
              }}
            >
              <option value="dark">Dark</option>
              <option value="light">Light</option>
              <option value="system">Follow system</option>
            </select>
          </label>
          <label>
            Default focus time (minutes)
            <input
              type="number"
              required
              min={1}
              max={480}
              value={minutes}
              onChange={(e) => {
                setMinutes(Number(e.target.value));
                setSaved(false);
              }}
            />
          </label>
        </div>
        <div className="row">
          <button disabled={busy || !desktop}>Save</button>
          {saved && (
            <span role="status" className="muted">
              Saved.
            </span>
          )}
        </div>
      </form>
      <PermissionsPanel />
    </>
  );
}
