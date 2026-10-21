import { useEffect, useState } from 'react';
import { PermissionsPanel } from '../components/PermissionsPanel';
import { PageHeading } from '../components/ui';
import {
  Choices,
  Modal,
  SettingRow,
  Toggle,
} from '../components/SettingsControls';
import { useAppStore } from '../state/store';
import { desktop } from '../services/api';
import {
  preferenceDefaults,
  settingsAction,
  type SettingsAction,
  type SettingsInfo,
} from '../services/settings';
const tabs = [
  'General',
  'Focus',
  'Alerts',
  'Connections',
  'Data & help',
] as const;
type Tab = (typeof tabs)[number];
export function Settings() {
  const { data, execute, busy, refresh } = useAppStore();
  const [tab, setTab] = useState<Tab>('General');
  const [draft, setDraft] = useState({
    ...preferenceDefaults,
    ...data.preferences,
  });
  const [dirty, setDirty] = useState(false);
  const [notice, setNotice] = useState('');
  const [working, setWorking] = useState(false);
  const [info, setInfo] = useState<SettingsInfo>({});
  const [confirm, setConfirm] = useState<
    'restore' | 'delete_history' | 'reset_preferences' | null
  >(null);
  const [typed, setTyped] = useState('');
  const [preset, setPreset] = useState(false);
  const [presetName, setPresetName] = useState('');
  const [source, setSource] = useState('');
  const disabled = !desktop || busy || working;
  useEffect(() => {
    if (!desktop) return;
    const load = () => {
      void settingsAction({ type: 'info' })
        .then(setInfo)
        .catch((e) => setNotice(String(e)));
    };
    load();
    window.addEventListener('focus', load);
    return () => window.removeEventListener('focus', load);
  }, []);
  function change<K extends keyof typeof draft>(
    key: K,
    value: (typeof draft)[K],
  ) {
    setDraft((p) => ({ ...p, [key]: value }));
    setDirty(true);
    setNotice('');
  }
  async function save(next = draft) {
    if (await execute({ type: 'update_preferences', preferences: next })) {
      setDraft({
        ...preferenceDefaults,
        ...useAppStore.getState().data.preferences,
      });
      setDirty(false);
      setNotice('Saved');
      return true;
    }
    return false;
  }
  async function run(action: SettingsAction, message = '') {
    setWorking(true);
    setNotice('');
    try {
      const result = await settingsAction(action);
      await refresh();
      setInfo(await settingsAction({ type: 'info' }));
      if (!result?.cancelled) setNotice(message);
      return !result?.cancelled;
    } catch (e) {
      setNotice(String(e));
      return false;
    } finally {
      setWorking(false);
    }
  }
  function switchRow(
    label: string,
    key:
      | 'large_text'
      | 'menu_timer'
      | 'session_alert'
      | 'sound'
      | 'session_start_alert'
      | 'session_start_sound'
      | 'reminder',
    hint?: string,
  ) {
    return (
      <SettingRow title={label} hint={hint}>
        <Toggle
          label={label}
          checked={draft[key]}
          onChange={(v) => change(key, v)}
          disabled={disabled}
        />
      </SettingRow>
    );
  }
  const recent = data.sessions
    .filter((s) => s.focus?.attention?.enabled)
    .slice()
    .reverse()
    .slice(0, 12);
  return (
    <>
      <PageHeading
        eyebrow=""
        title="Settings"
        description="Your app. Your preferences."
      />
      <div className="settings-layout">
        <nav className="settings-tabs" aria-label="Settings sections">
          {tabs.map((t) => (
            <button
              type="button"
              key={t}
              aria-current={tab === t ? 'page' : undefined}
              onClick={() => {
                setTab(t);
                setNotice('');
              }}
            >
              {t}
            </button>
          ))}
        </nav>
        <div className="panel settings-content">
          {tab === 'General' && (
            <section className="settings-section">
              <h2>General</h2>
              <SettingRow title="Appearance">
                <Choices
                  label="Appearance"
                  value={draft.theme}
                  choices={[
                    ['system', 'System'],
                    ['light', 'Light'],
                    ['dark', 'Dark'],
                  ]}
                  onChange={(v) => change('theme', v)}
                  disabled={disabled}
                />
              </SettingRow>
              <SettingRow title="Text size">
                <Choices
                  label="Text size"
                  value={draft.large_text ? 'large' : 'standard'}
                  choices={[
                    ['standard', 'Standard'],
                    ['large', 'Large'],
                  ]}
                  onChange={(v) => change('large_text', v === 'large')}
                  disabled={disabled}
                />
              </SettingRow>
              <SettingRow
                title="Open at login"
                hint={
                  info.login === 2
                    ? 'Approve VYSTRIQ in Login Items.'
                    : info.login === -1
                      ? 'Unavailable on this system.'
                      : 'Open the app when you sign in.'
                }
              >
                <Toggle
                  label="Open at login"
                  checked={info.login === 1 || info.login === 2}
                  disabled={
                    disabled || info.login === undefined || info.login === -1
                  }
                  onChange={(enabled) => void run({ type: 'login', enabled })}
                />
              </SettingRow>
              {switchRow(
                'Show timer in menu bar',
                'menu_timer',
                'Keep off for the icon only.',
              )}
            </section>
          )}
          {tab === 'Focus' && (
            <section className="settings-section">
              <h2>Focus</h2>
              <SettingRow title="Default focus time">
                <label className="number-control">
                  <input
                    aria-label="Default focus minutes"
                    type="number"
                    min={1}
                    max={480}
                    value={draft.default_minutes}
                    onChange={(e) =>
                      change('default_minutes', Number(e.target.value))
                    }
                  />{' '}
                  min
                </label>
              </SettingRow>
              <SettingRow
                title="Daily goal"
                hint="A target for your daily progress."
              >
                <Toggle
                  label="Daily goal"
                  checked={draft.daily_goal > 0}
                  onChange={(v) => change('daily_goal', v ? 120 : 0)}
                  disabled={disabled}
                />
              </SettingRow>
              {draft.daily_goal > 0 && (
                <SettingRow title="Goal length">
                  <label className="number-control">
                    <input
                      aria-label="Daily goal minutes"
                      type="number"
                      min={1}
                      max={1440}
                      value={draft.daily_goal}
                      onChange={(e) =>
                        change('daily_goal', Number(e.target.value))
                      }
                    />{' '}
                    min
                  </label>
                </SettingRow>
              )}
              <SettingRow
                title="Saved setups"
                hint="Reuse your apps, websites and focus time."
              >
                <button
                  className="secondary"
                  disabled={disabled}
                  onClick={() => {
                    setPresetName('');
                    setSource(recent[0]?.id ?? '');
                    setPreset(true);
                  }}
                >
                  Add setup
                </button>
              </SettingRow>
              <div className="preset-list">
                {draft.saved_setups.map((p) => (
                  <div key={p.id} className="preset-row">
                    <div>
                      <strong>{p.name}</strong>
                      <small>{p.minutes} min</small>
                    </div>
                    <button
                      className="text-button"
                      aria-label={`Remove ${p.name}`}
                      onClick={() =>
                        change(
                          'saved_setups',
                          draft.saved_setups.filter((x) => x.id !== p.id),
                        )
                      }
                    >
                      Remove
                    </button>
                  </div>
                ))}
              </div>
            </section>
          )}
          {tab === 'Alerts' && (
            <section className="settings-section">
              <h2>Alerts</h2>
              <SettingRow
                title="Notifications"
                hint="Allow focus and reminder alerts."
              >
                <button
                  className={
                    info.notifications === 2 ? 'connected-button' : 'secondary'
                  }
                  disabled={
                    disabled ||
                    info.notifications === 2 ||
                    info.notifications === -1
                  }
                  onClick={() =>
                    void run(
                      { type: 'allow_notifications' },
                      'Notifications are ready.',
                    )
                  }
                >
                  {info.notifications === 2
                    ? 'Allowed'
                    : info.notifications === -1
                      ? 'Unavailable'
                      : 'Allow notifications'}
                </button>
              </SettingRow>
              {switchRow('Session finished', 'session_alert')}
              {switchRow('Play a sound', 'sound')}
              {switchRow('Session started', 'session_start_alert')}
              {switchRow('Play a sound', 'session_start_sound')}
              {(draft.sound || draft.session_start_sound) && (
                <SettingRow title="Volume">
                  <div className="volume-control">
                    <input
                      type="range"
                      aria-label="Sound volume"
                      min={0}
                      max={100}
                      value={draft.volume}
                      onChange={(e) => change('volume', Number(e.target.value))}
                    />
                    <button
                      className="secondary"
                      disabled={disabled}
                      onClick={() =>
                        void run({
                          type: 'preview_sound',
                          volume: draft.volume,
                        })
                      }
                    >
                      Preview
                    </button>
                  </div>
                </SettingRow>
              )}
              {switchRow(
                'Focus reminder',
                'reminder',
                'A reminder, not an automatic session.',
              )}
              {draft.reminder && (
                <>
                  <SettingRow
                    title="Reminder time"
                    hint="Your computer’s local time."
                  >
                    <input
                      aria-label="Reminder time"
                      className="time-input"
                      type="time"
                      value={draft.reminder_time}
                      onChange={(e) => change('reminder_time', e.target.value)}
                    />
                  </SettingRow>
                  <SettingRow title="Repeat">
                    <div
                      className="day-picker"
                      role="group"
                      aria-label="Reminder days"
                    >
                      {['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'].map(
                        (d, i) => (
                          <button
                            type="button"
                            key={d}
                            aria-pressed={draft.reminder_days.includes(i)}
                            onClick={() =>
                              change(
                                'reminder_days',
                                draft.reminder_days.includes(i)
                                  ? draft.reminder_days.filter((x) => x !== i)
                                  : [...draft.reminder_days, i],
                              )
                            }
                          >
                            {d}
                          </button>
                        ),
                      )}
                    </div>
                  </SettingRow>
                  <p className="settings-footnote">
                    Reminders work while VYSTRIQ is running.
                  </p>
                </>
              )}
            </section>
          )}
          {tab === 'Connections' && <PermissionsPanel />}
          {tab === 'Data & help' && (
            <section className="settings-section">
              <h2>Data & help</h2>
              <SettingRow
                title="Export history"
                hint="Session records as a spreadsheet file."
              >
                <button
                  className="secondary"
                  disabled={disabled}
                  onClick={() =>
                    void run({ type: 'export_history' }, 'History exported.')
                  }
                >
                  Export CSV
                </button>
              </SettingRow>
              <SettingRow
                title="Backup"
                hint="Save your history, tasks and setups."
              >
                <div className="row controls">
                  <button
                    className="secondary"
                    disabled={disabled}
                    onClick={() =>
                      void run({ type: 'backup' }, 'Backup saved.')
                    }
                  >
                    Back up
                  </button>
                  <button
                    className="secondary"
                    disabled={disabled}
                    onClick={() => {
                      setTyped('');
                      setConfirm('restore');
                    }}
                  >
                    Restore
                  </button>
                </div>
              </SettingRow>
              <SettingRow
                title="Reset preferences"
                hint="Keep your history, tasks and saved setups."
              >
                <button
                  className="secondary"
                  disabled={disabled}
                  onClick={() => setConfirm('reset_preferences')}
                >
                  Reset
                </button>
              </SettingRow>
              <SettingRow
                title="Delete history"
                hint="Remove session records. Keep your tasks."
              >
                <button
                  className="danger secondary"
                  disabled={disabled}
                  onClick={() => {
                    setTyped('');
                    setConfirm('delete_history');
                  }}
                >
                  Delete history
                </button>
              </SettingRow>
              <div className="privacy-grid">
                <article>
                  <h3>Your data</h3>
                  <p>
                    Stored on this device. No video, typing or webpage content
                    is saved.
                  </p>
                </article>
                <article>
                  <h3>Focus checks</h3>
                  <p>
                    During a session, VYSTRIQ checks your current app, idle time
                    and connected website name.
                  </p>
                </article>
                <article>
                  <h3>Camera</h3>
                  <p>
                    Optional presence checks. It cannot prove attention,
                    learning or phone use.
                  </p>
                </article>
                <article>
                  <h3>Blocking</h3>
                  <p>
                    Websites are filtered in the connected browser profile.
                    Other profiles and apps keep their internet access.
                  </p>
                </article>
                <article>
                  <h3>App restrictions</h3>
                  <p>
                    Distracting app windows are hidden. System recovery tools
                    remain available.
                  </p>
                </article>
                <article>
                  <h3>Session completion</h3>
                  <p>
                    Restrictions are released when the session finishes. You can
                    quit VYSTRIQ from the menu bar when no session is active.
                  </p>
                </article>
              </div>
              <div className="help-card">
                <h3>Need help?</h3>
                <p>
                  Browser not working? Open Connections, select Connected, then
                  Test blocking. Reload the extension after an app update.
                </p>
                <p>
                  Manage camera and notification access in Connections →
                  Permissions.
                </p>
                <p>
                  During a session, closing and quitting are blocked. When idle,
                  use the tray icon to reopen or quit.
                </p>
              </div>
              <p className="settings-footnote">
                VYSTRIQ {info.version ?? '0.4.0'} ·{' '}
                {info.platform === 'windows'
                  ? 'Windows'
                  : info.platform === 'macos'
                    ? 'Mac'
                    : 'Desktop'}
              </p>
            </section>
          )}
          {notice && (
            <p role="status" className="settings-feedback">
              {notice}
            </p>
          )}
          {tab !== 'Connections' && (
            <div className="settings-save">
              <span className="muted small">
                {dirty ? 'Unsaved changes' : 'Preferences saved'}
              </span>
              <button disabled={disabled || !dirty} onClick={() => void save()}>
                Save changes
              </button>
            </div>
          )}
        </div>
      </div>
      {preset && (
        <Modal title="Save a setup" onClose={() => setPreset(false)}>
          <div className="dialog-stack">
            {recent.length ? (
              <>
                <label>
                  Setup name
                  <input
                    maxLength={80}
                    value={presetName}
                    placeholder="e.g. Coding"
                    onChange={(e) => setPresetName(e.target.value)}
                  />
                </label>
                <p className="muted small">
                  Copy apps and websites from a session.
                </p>
                <div className="source-picker">
                  {recent.map((s) => (
                    <button
                      key={s.id}
                      type="button"
                      aria-pressed={source === s.id}
                      onClick={() => setSource(s.id)}
                    >
                      {s.title}
                      <small>{s.duration_minutes} min</small>
                    </button>
                  ))}
                </div>
                <button
                  disabled={!presetName.trim() || disabled}
                  onClick={async () => {
                    const s = recent.find((s) => s.id === source);
                    if (!s?.focus) return;
                    const next = {
                      ...draft,
                      saved_setups: [
                        ...draft.saved_setups,
                        {
                          id: crypto.randomUUID(),
                          name: presetName.trim(),
                          minutes: s.duration_minutes,
                          focus: {
                            ...s.focus,
                            activity_consent: false,
                            camera: false,
                          },
                        },
                      ],
                    };
                    if (await save(next)) setPreset(false);
                  }}
                >
                  Save setup
                </button>
              </>
            ) : (
              <p>
                Create your first session, then save its apps and websites here.
              </p>
            )}
          </div>
        </Modal>
      )}
      {confirm && (
        <Modal
          title={
            confirm === 'restore'
              ? 'Restore backup?'
              : confirm === 'delete_history'
                ? 'Delete session history?'
                : 'Reset preferences?'
          }
          onClose={() => setConfirm(null)}
        >
          <div className="dialog-stack">
            <p>
              {confirm === 'restore'
                ? 'Replaces your history, tasks and preferences. A recovery backup is saved on this device first.'
                : confirm === 'delete_history'
                  ? 'This permanently deletes session history and analytics. Export a backup first if you need a copy.'
                  : 'Restores default preferences. Your history, tasks and saved setups stay.'}
            </p>
            {confirm !== 'reset_preferences' && (
              <label>
                Type {confirm === 'restore' ? 'RESTORE' : 'DELETE HISTORY'}
                <input
                  aria-label="Confirmation"
                  value={typed}
                  onChange={(e) => setTyped(e.target.value)}
                />
              </label>
            )}
            <button
              className={confirm === 'delete_history' ? 'danger' : ''}
              disabled={
                disabled ||
                (confirm !== 'reset_preferences' &&
                  typed !==
                    (confirm === 'restore' ? 'RESTORE' : 'DELETE HISTORY'))
              }
              onClick={async () => {
                const ok = await run(
                  confirm === 'reset_preferences'
                    ? { type: confirm }
                    : { type: confirm, confirmation: typed },
                  confirm === 'restore'
                    ? 'Backup restored. Session permissions must be chosen again.'
                    : confirm === 'delete_history'
                      ? 'Session history deleted.'
                      : 'Preferences reset.',
                );
                if (ok) {
                  setDraft({
                    ...preferenceDefaults,
                    ...useAppStore.getState().data.preferences,
                  });
                  setDirty(false);
                  setConfirm(null);
                }
              }}
            >
              {confirm === 'restore'
                ? 'Choose backup'
                : confirm === 'delete_history'
                  ? 'Delete history'
                  : 'Reset preferences'}
            </button>
          </div>
        </Modal>
      )}
    </>
  );
}
