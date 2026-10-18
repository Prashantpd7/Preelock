import { useState } from 'react';
import { ShieldCheck } from 'lucide-react';
import { PageHeading } from '../components/ui';
import { useAppStore } from '../state/store';
import { desktop } from '../services/api';
import type { Preferences } from '../types';
export function Settings() {
  const { data, capabilities, execute, busy } = useAppStore();
  const [theme, setTheme] = useState(data.preferences.theme);
  const [minutes, setMinutes] = useState(data.preferences.default_minutes);
  const [saved, setSaved] = useState(false);
  return (
    <>
      <PageHeading
        eyebrow="MAKE IT YOURS"
        title="Settings"
        description="A workspace that respects your preferences and your privacy."
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
        <h2>Workspace preferences</h2>
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
            Default session minutes
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
          <button disabled={busy || !desktop}>Save preferences</button>
          {saved && (
            <span role="status" className="muted">
              Preferences saved.
            </span>
          )}
        </div>
      </form>
      <section className="panel privacy-panel">
        <ShieldCheck size={24} />
        <h2>Local by design</h2>
        <p className="muted">
          Tasks, sessions and preferences stay in SQLite on this device. No
          account, cloud connection, camera, screen capture or activity tracking
          is active.
        </p>
        <p className="muted">
          Session timers are a focus aid. You can always end a session or quit
          the app. They do not lock your device.
        </p>
      </section>
      <div className="section-heading">
        <h2>Future capabilities</h2>
        <span className="badge">Not implemented</span>
      </div>
      <section className="panel">
        {capabilities.length ? (
          capabilities.map((c) => (
            <div className="capability-row" key={c.id}>
              <h3>{c.name}</h3>
              <p className="muted small">{c.reason}</p>
            </div>
          ))
        ) : (
          <p className="muted">
            App and website restrictions, device monitoring, camera analysis, AI
            verification and reminders are planned. No permissions are requested
            in Phase 1.
          </p>
        )}
      </section>
    </>
  );
}
