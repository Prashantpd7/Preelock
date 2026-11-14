import { useEffect, useState } from 'react';
import {
  AlertTriangle,
  ChevronRight,
  Globe,
  LayoutGrid,
  ShieldAlert,
  X,
} from 'lucide-react';
import { useAppStore } from '../state/store';
import { Dropdown } from './Dropdown';
import { RowIcon } from './AppIcon';
import { Toggle } from './SettingsControls';
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
import type {
  Action,
  AdultBlock,
  AppLock,
  ScreenTimeBucket,
  StrictBlockState,
} from '../types';

type PanelKind = 'apps' | 'sites' | 'adult';
type LockMode = 'timed' | 'daily';

type Execute = (action: Action) => Promise<boolean>;

/** Strip leading zeros as the user types ("010" → "10"); keep "" and "0". */
function normalizeMinutesInput(raw: string): string {
  if (raw === '') return '';
  const stripped = raw.replace(/^0+/, '');
  return stripped === '' ? '0' : stripped;
}

/** App lock section of the Screen Time page: expandable "Block apps" /
 *  "Block websites" / "Block adult websites" panels, each with
 *  independent background enforcement. */
export function AppLockPanel() {
  const { data, execute, busy } = useAppStore();
  const locks = data.app_locks ?? [];
  const adultBlock = data.adult_block;
  const buckets = data.screen_time_usage ?? [];
  // Accordion: opening one panel closes the other, keeping the page
  // short while a form is open.
  const [openPanel, setOpenPanel] = useState<PanelKind | null>(null);

  const [nowMs, setNowMs] = useState(() => Date.now());
  useEffect(() => {
    const timer = window.setInterval(() => setNowMs(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, []);

  const todayMs = localDayStart(new Date());

  function toggle(panel: PanelKind) {
    setOpenPanel((current) => (current === panel ? null : panel));
  }

  const body = (kind: 'app' | 'site') => (
    <AppLockPanelBody
      kind={kind}
      locks={locks}
      strictState={kind === 'app' ? data.strict_apps : data.strict_sites}
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
        <button
          type="button"
          className="screen-time-lock-option"
          aria-expanded={openPanel === 'adult'}
          onClick={() => toggle('adult')}
        >
          <ShieldAlert size={20} aria-hidden="true" />
          <span>Block adult websites</span>
          <ChevronRight size={18} aria-hidden="true" />
        </button>
        {openPanel === 'adult' && (
          <AdultBlockPanelBody
            adultBlock={adultBlock}
            nowMs={nowMs}
            busy={busy}
            execute={execute}
          />
        )}
      </div>
    </section>
  );
}

function AppLockPanelBody({
  kind,
  locks,
  strictState,
  buckets,
  todayMs,
  nowMs,
  busy,
  execute,
}: {
  kind: 'app' | 'site';
  locks: AppLock[];
  strictState?: StrictBlockState;
  buckets: ScreenTimeBucket[];
  todayMs: number;
  nowMs: number;
  busy: boolean;
  execute: Execute;
}) {
  const [mode, setMode] = useState<LockMode | null>(null);
  const target = kind === 'app' ? 'apps' : 'sites';
  const label = kind === 'app' ? 'apps' : 'websites';
  return (
    <div className="screen-time-lock-panel">
      <StrictBlockSection
        description={`Lock ${label} blocking for the chosen duration. Active ${label} blocks cannot be deleted or bypassed until expired.`}
        strictState={strictState}
        target={target}
        nowMs={nowMs}
        busy={busy}
        execute={execute}
      />
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
        strictState={strictState}
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
  strictState,
  buckets,
  todayMs,
  nowMs,
  busy,
  execute,
}: {
  kind: 'app' | 'site';
  locks: AppLock[];
  strictState?: StrictBlockState;
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

  const isStrictLocked = Boolean(
    strictState?.strict &&
    strictState?.ends_at_ms &&
    nowMs < strictState.ends_at_ms,
  );

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
                {isStrictLocked ? (
                  <span
                    className="screen-time-lock-tag"
                    style={{
                      borderColor: 'var(--amber, #f59e0b)',
                      color: 'var(--amber, #f59e0b)',
                      fontSize: '11px',
                      padding: '2px 8px',
                    }}
                  >
                    Strictly locked
                  </span>
                ) : (
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
                )}
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

const STRICT_DAY_PRESETS = [1, 2, 3, 5, 7];

function formatStrictRemaining(ms: number): string {
  const mins = Math.max(0, Math.floor(ms / 60_000));
  const days = Math.floor(mins / 1440);
  const hours = Math.floor((mins % 1440) / 60);
  const remainingMins = mins % 60;
  if (days > 0) return hours > 0 ? `${days}d ${hours}h left` : `${days}d left`;
  if (hours > 0)
    return remainingMins > 0
      ? `${hours}h ${remainingMins}m left`
      : `${hours}h left`;
  return `${remainingMins}m left`;
}

function formatStrictDateTime(ms: number): string {
  return new Date(ms).toLocaleString(undefined, {
    weekday: 'short',
    month: 'short',
    day: 'numeric',
    hour: 'numeric',
    minute: '2-digit',
  });
}

function StrictBlockSection({
  description,
  strictState,
  target,
  nowMs,
  busy,
  execute,
}: {
  description: string;
  strictState?: StrictBlockState;
  target: 'apps' | 'sites';
  nowMs: number;
  busy: boolean;
  execute: Execute;
}) {
  const [selectedDays, setSelectedDays] = useState(1);

  const isEnabled = Boolean(strictState?.enabled);
  const isStrict = Boolean(strictState?.strict);
  const endsAt = strictState?.ends_at_ms ?? null;
  const isExpired = endsAt != null && endsAt <= nowMs;
  const isStrictActive = isEnabled && isStrict && !isExpired;

  const activatingUntil = strictState?.activating_until_ms ?? null;
  const isActivating = activatingUntil != null && nowMs < activatingUntil;
  const countdownSec = isActivating
    ? Math.max(1, Math.ceil((activatingUntil - nowMs) / 1000))
    : 0;

  const displayDays =
    endsAt && activatingUntil
      ? Math.max(1, Math.round((endsAt - activatingUntil) / 86_400_000))
      : selectedDays;

  const label = target === 'apps' ? 'apps' : 'websites';

  if (isStrictActive && endsAt) {
    return (
      <div className="adult-block-locked-card" style={{ marginBottom: '12px' }}>
        <div className="screen-time-lock-row">
          {target === 'apps' ? (
            <LayoutGrid size={20} aria-hidden="true" />
          ) : (
            <Globe size={20} aria-hidden="true" />
          )}
          <div className="screen-time-lock-row-meta">
            <strong>
              {target === 'apps'
                ? 'Apps blocked strictly'
                : 'Websites blocked strictly'}
            </strong>
            <span className="screen-time-lock-status muted small">
              <span
                className="screen-time-lock-tag"
                style={{
                  borderColor: 'var(--amber, #f59e0b)',
                  color: 'var(--amber, #f59e0b)',
                }}
              >
                Strict block
              </span>
              {formatStrictRemaining(endsAt - nowMs)}
            </span>
          </div>
        </div>
        <p className="adult-block-strict-note">
          Strict block is active until{' '}
          <strong>{formatStrictDateTime(endsAt)}</strong> (
          {formatStrictRemaining(endsAt - nowMs)}). Active {label} blocks cannot
          be deleted or disabled until expired.
        </p>
      </div>
    );
  }

  if (isActivating) {
    return (
      <div
        className="adult-block-countdown-card"
        style={{ marginBottom: '12px' }}
      >
        <div className="adult-block-countdown-info">
          <div className="adult-block-countdown-header">
            <AlertTriangle size={16} aria-hidden="true" />
            <span>Strict block activating in {countdownSec}s...</span>
          </div>
          <p
            className="muted small"
            style={{ fontSize: '11px', margin: '2px 0 0' }}
          >
            Locks {label} strictly for {displayDays}{' '}
            {displayDays === 1 ? 'day' : 'days'}. Turn back off now to cancel.
          </p>
        </div>
        <button
          type="button"
          className="secondary"
          style={{
            padding: '6px 12px',
            fontSize: '12px',
            whiteSpace: 'nowrap',
          }}
          disabled={busy}
          onClick={async () => {
            await execute({ type: 'cancel_strict_block', target });
          }}
        >
          Turn back off
        </button>
      </div>
    );
  }

  return (
    <div
      className="adult-block-strict-section"
      style={{ marginBottom: '12px' }}
    >
      <div className="adult-block-strict-row">
        <div className="adult-block-strict-meta">
          <span className="screen-time-lock-label">Strict block</span>
          <p
            className="muted small"
            style={{ margin: '2px 0 0', fontSize: '12px' }}
          >
            {description}
          </p>
        </div>
        <div className="adult-block-strict-controls">
          <div
            className="adult-block-compact-chips"
            role="group"
            aria-label={`Strict duration days for ${label}`}
          >
            {STRICT_DAY_PRESETS.map((d) => (
              <button
                key={d}
                type="button"
                aria-pressed={selectedDays === d}
                onClick={() => setSelectedDays(d)}
              >
                {d} {d === 1 ? 'day' : 'days'}
              </button>
            ))}
          </div>
          <Toggle
            label={`Strict block for ${label}`}
            checked={false}
            disabled={busy}
            onChange={async (checked) => {
              if (checked) {
                await execute({
                  type: 'start_strict_block',
                  target,
                  days: selectedDays,
                });
              }
            }}
          />
        </div>
      </div>
    </div>
  );
}

function AdultBlockPanelBody({
  adultBlock,
  nowMs,
  busy,
  execute,
}: {
  adultBlock?: AdultBlock;
  nowMs: number;
  busy: boolean;
  execute: Execute;
}) {
  const [selectedDays, setSelectedDays] = useState(1);

  const isEnabled = Boolean(adultBlock?.enabled);
  const isStrict = Boolean(adultBlock?.strict);
  const endsAt = adultBlock?.ends_at_ms ?? null;
  const isExpired = endsAt != null && endsAt <= nowMs;
  const isStrictActive = isEnabled && isStrict && !isExpired;

  const activatingUntil = adultBlock?.activating_until_ms ?? null;
  const isActivating = activatingUntil != null && nowMs < activatingUntil;
  const countdownSec = isActivating
    ? Math.max(1, Math.ceil((activatingUntil - nowMs) / 1000))
    : 0;

  const isNormalActive = isEnabled && !isStrict && !isExpired && !isActivating;

  const displayDays =
    endsAt && activatingUntil
      ? Math.max(1, Math.round((endsAt - activatingUntil) / 86_400_000))
      : selectedDays;

  async function handleToggleMain(turnOn: boolean) {
    if (isActivating) {
      await execute({ type: 'cancel_strict_block', target: 'adult' });
    }
    if (turnOn) {
      await execute({
        type: 'set_adult_block',
        enabled: true,
        strict: false,
      });
    } else {
      await execute({
        type: 'set_adult_block',
        enabled: false,
        strict: false,
      });
    }
  }

  return (
    <div className="screen-time-lock-panel">
      {/* 1. Turn on / Turn off Toggle row */}
      <div className="adult-block-toggle-row">
        <div className="adult-block-toggle-meta">
          <strong>Block adult websites</strong>
          <p className="muted small">
            {isStrictActive
              ? 'Strict block is active system-wide across all browsers.'
              : isActivating
                ? 'Strict block is activating system-wide...'
                : isNormalActive
                  ? 'Adult websites blocked system-wide across all browsers.'
                  : 'Turn on to block adult websites across Safari, Chrome, Brave, Comet, Edge, and Arc.'}
          </p>
        </div>
        <Toggle
          label="Block adult websites"
          checked={isStrictActive || isNormalActive || isActivating}
          disabled={busy || isStrictActive}
          onChange={handleToggleMain}
        />
      </div>

      {/* 2. When Strict block is locked and active */}
      {isStrictActive && endsAt && (
        <div className="adult-block-locked-card">
          <div className="screen-time-lock-row">
            <ShieldAlert size={20} aria-hidden="true" />
            <div className="screen-time-lock-row-meta">
              <strong>Adult websites blocked system-wide</strong>
              <span className="screen-time-lock-status muted small">
                <span
                  className="screen-time-lock-tag"
                  style={{
                    borderColor: 'var(--amber, #f59e0b)',
                    color: 'var(--amber, #f59e0b)',
                  }}
                >
                  Strict block
                </span>
                {formatStrictRemaining(endsAt - nowMs)}
              </span>
            </div>
          </div>
          <p className="adult-block-strict-note">
            Strict block is active until{' '}
            <strong>{formatStrictDateTime(endsAt)}</strong> (
            {formatStrictRemaining(endsAt - nowMs)}). It cannot be canceled or
            disabled until expired.
          </p>
        </div>
      )}

      {/* 3. When filtering is turned on (Normal mode or countdown running) */}
      {(isNormalActive || isActivating) && !isStrictActive && (
        <>
          {isActivating ? (
            <div className="adult-block-countdown-card">
              <div className="adult-block-countdown-info">
                <div className="adult-block-countdown-header">
                  <AlertTriangle size={16} aria-hidden="true" />
                  <span>Strict block activating in {countdownSec}s...</span>
                </div>
                <p
                  className="muted small"
                  style={{ fontSize: '11px', margin: '2px 0 0' }}
                >
                  Filters adult websites strictly for {displayDays}{' '}
                  {displayDays === 1 ? 'day' : 'days'}. Turn back off now to
                  cancel.
                </p>
              </div>
              <button
                type="button"
                className="secondary"
                style={{
                  padding: '6px 12px',
                  fontSize: '12px',
                  whiteSpace: 'nowrap',
                }}
                disabled={busy}
                onClick={async () => {
                  await execute({
                    type: 'cancel_strict_block',
                    target: 'adult',
                  });
                }}
              >
                Turn back off
              </button>
            </div>
          ) : (
            <div className="adult-block-strict-section">
              <div className="adult-block-strict-row">
                <div className="adult-block-strict-meta">
                  <span className="screen-time-lock-label">Strict block</span>
                  <p
                    className="muted small"
                    style={{ margin: '2px 0 0', fontSize: '12px' }}
                  >
                    Lock filtering for the chosen duration. Cannot be canceled
                    or disabled until expired.
                  </p>
                </div>
                <div className="adult-block-strict-controls">
                  <div
                    className="adult-block-compact-chips"
                    role="group"
                    aria-label="Strict duration days"
                  >
                    {STRICT_DAY_PRESETS.map((d) => (
                      <button
                        key={d}
                        type="button"
                        aria-pressed={selectedDays === d}
                        onClick={() => setSelectedDays(d)}
                      >
                        {d} {d === 1 ? 'day' : 'days'}
                      </button>
                    ))}
                  </div>
                  <Toggle
                    label="Strict block"
                    checked={false}
                    disabled={busy}
                    onChange={async (checked) => {
                      if (checked) {
                        await execute({
                          type: 'start_strict_block',
                          target: 'adult',
                          days: selectedDays,
                        });
                      }
                    }}
                  />
                </div>
              </div>
            </div>
          )}
        </>
      )}
    </div>
  );
}
