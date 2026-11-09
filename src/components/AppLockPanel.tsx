import { useEffect, useState } from 'react';
import { ChevronRight, Globe, LayoutGrid, X } from 'lucide-react';
import { useAppStore } from '../state/store';
import { Dropdown } from './Dropdown';
import { RowIcon } from './AppIcon';
import { localDayStart } from '../services/screenTimeWeek';
import {
  LIMIT_MAX_MINUTES,
  LIMIT_PRESETS,
  TIMED_MAX_MINUTES,
  TIMED_PRESETS,
  appLockStatus,
  normalizeAppLockDomain,
  recentApps,
  validateAppLockForm,
} from '../services/appLocks';
import type { Action, AppLock, ScreenTimeBucket } from '../types';

type PanelKind = 'apps' | 'sites';
type LockMode = 'timed' | 'daily';

type Execute = (action: Action) => Promise<boolean>;

/** Strip leading zeros as the user types ("010" → "10"); keep "" and "0". */
function normalizeMinutesInput(raw: string): string {
  if (raw === '') return '';
  const stripped = raw.replace(/^0+/, '');
  return stripped === '' ? '0' : stripped;
}

/** App lock section of the Screen Time page: expandable "Block apps" /
 *  "Block websites" panels, each with a timed-block and a daily-limit
 *  form plus its Active blocks list. Enforcement lives in the Rust
 *  engine (no session required); this is the UI over
 *  `data.app_locks`, refreshed through the same snapshot as the usage
 *  list above it. */
export function AppLockPanel() {
  const { data, execute, busy } = useAppStore();
  const locks = data.app_locks ?? [];
  const buckets = data.screen_time_usage ?? [];
  // Accordion: opening one panel closes the other, keeping the page
  // short while a form is open.
  const [openPanel, setOpenPanel] = useState<PanelKind | null>(null);

  // Live countdown for timed locks. The clock lives in an effect,
  // never in render (the same purity rule as the Dashboard's grace
  // timer), and only ticks while a timed lock exists — daily-limit
  // status follows the snapshot refresh on its own.
  const hasTimedLock = locks.some((lock) => lock.rule.type === 'timed');
  const [nowMs, setNowMs] = useState(() => Date.now());
  useEffect(() => {
    if (!hasTimedLock) return;
    const timer = window.setInterval(() => setNowMs(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, [hasTimedLock]);

  const todayMs = localDayStart(new Date());

  function toggle(panel: PanelKind) {
    setOpenPanel((current) => (current === panel ? null : panel));
  }

  const body = (kind: 'app' | 'site') => (
    <AppLockPanelBody
      kind={kind}
      locks={locks}
      buckets={buckets}
      todayMs={todayMs}
      nowMs={nowMs}
      busy={busy}
      execute={execute}
    />
  );

  return (
    <section className="panel screen-time-lock">
      <h2>App lock</h2>
      <div className="screen-time-lock-options">
        <button
          type="button"
          className="screen-time-lock-option"
          aria-expanded={openPanel === 'apps'}
          onClick={() => toggle('apps')}
        >
          <LayoutGrid size={20} aria-hidden="true" />
          <span>Block apps</span>
          <ChevronRight size={18} aria-hidden="true" />
        </button>
        {openPanel === 'apps' && body('app')}
        <button
          type="button"
          className="screen-time-lock-option"
          aria-expanded={openPanel === 'sites'}
          onClick={() => toggle('sites')}
        >
          <Globe size={20} aria-hidden="true" />
          <span>Block websites</span>
          <ChevronRight size={18} aria-hidden="true" />
        </button>
        {openPanel === 'sites' && body('site')}
      </div>
    </section>
  );
}

function AppLockPanelBody({
  kind,
  locks,
  buckets,
  todayMs,
  nowMs,
  busy,
  execute,
}: {
  kind: 'app' | 'site';
  locks: AppLock[];
  buckets: ScreenTimeBucket[];
  todayMs: number;
  nowMs: number;
  busy: boolean;
  execute: Execute;
}) {
  const [mode, setMode] = useState<LockMode | null>(null);
  return (
    <div className="screen-time-lock-panel">
      <div
        className="screen-time-lock-modes"
        role="group"
        aria-label={kind === 'app' ? 'App block type' : 'Website block type'}
      >
        <button
          type="button"
          aria-pressed={mode === 'timed'}
          onClick={() =>
            setMode((current) => (current === 'timed' ? null : 'timed'))
          }
        >
          Block for a set time
        </button>
        <button
          type="button"
          aria-pressed={mode === 'daily'}
          onClick={() =>
            setMode((current) => (current === 'daily' ? null : 'daily'))
          }
        >
          Block after daily limit
        </button>
      </div>
      {mode && (
        <AppLockForm
          key={mode}
          kind={kind}
          mode={mode}
          buckets={buckets}
          busy={busy}
          execute={execute}
          onDone={() => setMode(null)}
        />
      )}
      <AppLockList
        kind={kind}
        locks={locks}
        buckets={buckets}
        todayMs={todayMs}
        nowMs={nowMs}
        busy={busy}
        execute={execute}
      />
    </div>
  );
}

function AppLockForm({
  kind,
  mode,
  buckets,
  busy,
  execute,
  onDone,
}: {
  kind: 'app' | 'site';
  mode: LockMode;
  buckets: ScreenTimeBucket[];
  busy: boolean;
  execute: Execute;
  onDone: () => void;
}) {
  const apps = recentApps(buckets);
  const [appTarget, setAppTarget] = useState('');
  const [siteInput, setSiteInput] = useState('');
  // Text state (not a number) so typed leading zeros can be stripped
  // before they stick in the field; parsed to a number on submit.
  const [minutesText, setMinutesText] = useState(
    mode === 'timed' ? '60' : '30',
  );
  const [problem, setProblem] = useState('');

  // The picker defaults to the most-used app until the user picks one.
  const effectiveAppTarget = appTarget || apps[0]?.target || '';
  const selectedApp = apps.find((app) => app.target === effectiveAppTarget);
  const presets = mode === 'timed' ? TIMED_PRESETS : LIMIT_PRESETS;
  const maxMinutes = mode === 'timed' ? TIMED_MAX_MINUTES : LIMIT_MAX_MINUTES;

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    setProblem('');
    const target =
      kind === 'site' ? normalizeAppLockDomain(siteInput) : effectiveAppTarget;
    const title =
      kind === 'site' ? target : (selectedApp?.name ?? effectiveAppTarget);
    const minutes = Number(minutesText);
    const validation = validateAppLockForm({
      title,
      kind,
      target,
      mode,
      minutes,
    });
    if (validation) {
      setProblem(validation);
      return;
    }
    const ok = await execute({
      type: 'create_app_lock',
      title: title.trim(),
      kind,
      target,
      rule:
        mode === 'timed'
          ? { kind: 'timed', minutes }
          : { kind: 'daily_limit', limit_minutes: minutes },
    });
    if (ok) onDone();
    else
      setProblem(useAppStore.getState().error || 'Could not save the block.');
  }

  return (
    <form className="screen-time-lock-form" onSubmit={submit}>
      {kind === 'app' ? (
        apps.length ? (
          <div className="screen-time-lock-field">
            <span className="screen-time-lock-label">App</span>
            <span className="screen-time-lock-picker">
              {selectedApp && (
                <RowIcon
                  kind="app"
                  target={selectedApp.target}
                  name={selectedApp.name}
                />
              )}
              <Dropdown
                value={effectiveAppTarget}
                onChange={(value) => {
                  setAppTarget(value);
                  setProblem('');
                }}
                options={apps.map((app) => ({
                  value: app.target,
                  label: app.name,
                }))}
              />
            </span>
          </div>
        ) : (
          <p className="muted small">
            No recently used apps yet — they appear here as you use apps.
          </p>
        )
      ) : (
        <>
          <label>
            Website
            <input
              value={siteInput}
              onChange={(e) => {
                setSiteInput(e.target.value);
                setProblem('');
              }}
              placeholder="e.g. youtube.com"
              autoComplete="off"
              spellCheck={false}
            />
          </label>
          <p className="muted small">
            Domain only — no paths or search: keywords.
          </p>
        </>
      )}
      <span className="screen-time-lock-label">Duration</span>
      <div
        className="screen-time-lock-chips"
        role="group"
        aria-label="Duration presets"
      >
        {presets.map((preset) => (
          <button
            key={preset.minutes}
            type="button"
            className="secondary"
            aria-pressed={minutesText === String(preset.minutes)}
            onClick={() => setMinutesText(String(preset.minutes))}
          >
            {preset.label}
          </button>
        ))}
      </div>
      <label>
        Custom (minutes)
        <input
          type="number"
          required
          min={1}
          max={maxMinutes}
          value={minutesText}
          onChange={(e) =>
            setMinutesText(normalizeMinutesInput(e.target.value))
          }
        />
      </label>
      {problem && (
        <p className="error small" role="alert">
          {problem}
        </p>
      )}
      <div className="screen-time-lock-actions">
        <button type="button" className="secondary" onClick={onDone}>
          Cancel
        </button>
        <button disabled={busy || (kind === 'app' && !apps.length)}>
          {mode === 'timed' ? 'Start block' : 'Set limit'}
        </button>
      </div>
    </form>
  );
}

function AppLockList({
  kind,
  locks,
  buckets,
  todayMs,
  nowMs,
  busy,
  execute,
}: {
  kind: 'app' | 'site';
  locks: AppLock[];
  buckets: ScreenTimeBucket[];
  todayMs: number;
  nowMs: number;
  busy: boolean;
  execute: Execute;
}) {
  // Newest first.
  const panelLocks = locks
    .filter((lock) => lock.kind === kind)
    .sort((a, b) => b.created_at - a.created_at);
  return (
    <div>
      <h3 className="screen-time-lock-subhead">Active blocks</h3>
      {panelLocks.length ? (
        <ul className="screen-time-lock-list">
          {panelLocks.map((lock) => {
            const status = appLockStatus(lock, buckets, todayMs, nowMs);
            return (
              <li key={lock.id} className="screen-time-lock-row">
                <RowIcon
                  kind={lock.kind}
                  target={lock.target}
                  name={lock.title}
                />
                <span className="screen-time-lock-row-meta">
                  <strong>{lock.title}</strong>
                  <span className="screen-time-lock-status muted small">
                    <span className="screen-time-lock-tag">
                      {lock.rule.type === 'timed' ? 'Timed' : 'Daily limit'}
                    </span>
                    {status.text}
                  </span>
                </span>
                <button
                  type="button"
                  className="icon-button danger"
                  aria-label={`Remove block: ${lock.title}`}
                  disabled={busy}
                  onClick={async () => {
                    await execute({ type: 'delete_app_lock', id: lock.id });
                  }}
                >
                  <X size={16} aria-hidden="true" />
                </button>
              </li>
            );
          })}
        </ul>
      ) : (
        <p className="muted small">
          {kind === 'app'
            ? 'No active app blocks.'
            : 'No active website blocks.'}
        </p>
      )}
    </div>
  );
}
