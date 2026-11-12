import { useState } from 'react';
import { Plus } from 'lucide-react';
import { useAppStore } from '../state/store';
import { Empty } from './ui';
import { Dropdown } from './Dropdown';
import { SessionCard } from './SessionCard';
import { AllowanceForm } from './AllowanceForm';
import {
  inDateRange,
  isHistoricalSession,
  isOneShotSession,
} from '../services/activitySection';
import type { AnalyticsRange } from '../services/analytics';
import {
  DAY_LABELS,
  formatDays,
  isScheduleActive,
  isScheduleLockedActive,
  scheduleLockDelayStatus,
  scheduleTotalTime,
  validateScheduleForm,
} from '../services/schedules';
import { oneShotFireAtMs, validateOneShotForm } from '../services/one_shots';
import {
  allowanceExhausted,
  allowancePeriodLabel,
  allowanceRemainingMs,
  formatAllowanceMinutes,
} from '../services/allowances';
import {
  defaultFocus,
  lockLabel,
  type Allowance,
  type FocusSetup,
  type LockConfig,
  type OneShot,
  type Schedule,
} from '../types';
// Schedules, one-shot timers, and allowances: they live in the
// Activity dropdown on the Focus Sessions page, one section at a
// time, with the same lists and behavior. `only` renders a single
// section; with no `only`, all three render (as before).
// `onNewSchedule` / `onNewOneShot` open the session wizard above;
// `onNewAllowance` / `onEditAllowance` open the allowance form above
// so Activity stays list-focused. When omitted (standalone use), inline
// forms remain available as a fallback.
export function PlanningSections({
  only,
  onNewSchedule,
  onNewOneShot,
  onNewAllowance,
  onEditAllowance,
  range = 'all',
  setup,
  onSetupRefresh,
}: {
  only?: 'schedules' | 'oneshots' | 'allowances';
  onNewSchedule?: () => void;
  onNewOneShot?: () => void;
  onNewAllowance?: () => void;
  onEditAllowance?: (allowance: Allowance) => void;
  /** Date range from the Activity toolbar; 'all' shows everything.
   *  Only the displayed lists are filtered — edit forms and lookups
   *  keep using the full lists below. */
  range?: AnalyticsRange;
  setup?: FocusSetup | null;
  onSetupRefresh?: () => Promise<FocusSetup | null>;
} = {}) {
  const { data, execute, busy } = useAppStore();
  // Scheduled blocks (step 5): the list and form live on the
  // Dashboard and refresh through the same snapshot as the sessions.
  const schedules = data.schedules ?? [];
  const [schedTemplateId, setSchedTemplateId] = useState('');
  const [schedTitle, setSchedTitle] = useState('');
  const [schedDays, setSchedDays] = useState<boolean[]>([
    false,
    true,
    true,
    true,
    true,
    true,
    false,
  ]);
  const [schedStart, setSchedStart] = useState('09:00');
  const [schedEnd, setSchedEnd] = useState('10:00');
  const [schedEnabled, setSchedEnabled] = useState(true);
  // Step 10 lock for schedules: the password lock is the only one
  // this form offers now — window changes and the other lock kinds
  // are always off (their previous defaults), so there is no state
  // for them.
  const [schedLockPasswordOn, setSchedLockPasswordOn] = useState(false);
  const [schedLockPassword, setSchedLockPassword] = useState('');
  // Proofs for changing a locked-active schedule, entered in its
  // edit form (the password is never shown back: editing keeps
  // the stored hash unless a new password is typed above).
  const [schedUnlockPassword, setSchedUnlockPassword] = useState('');
  const [schedUnlockChallenge, setSchedUnlockChallenge] = useState('');
  const [schedEditingId, setSchedEditingId] = useState<string | null>(null);
  const [schedFormOpen, setSchedFormOpen] = useState(false);
  const [schedProblem, setSchedProblem] = useState('');
  const editingSchedule = schedEditingId
    ? schedules.find((s) => s.id === schedEditingId)
    : undefined;
  const editingLockedActive = editingSchedule
    ? isScheduleLockedActive(editingSchedule)
    : false;
  // One-shot start timers (step 6): the list and form live on the
  // Dashboard, refreshing through the same snapshot.
  const oneShots: OneShot[] = data.one_shots ?? [];
  const [oneTemplateId, setOneTemplateId] = useState('');
  const [oneTitle, setOneTitle] = useState('');
  const [oneWhen, setOneWhen] = useState('');
  const [oneFormOpen, setOneFormOpen] = useState(false);
  const [oneProblem, setOneProblem] = useState('');
  // Time allowances (step 8): the list and form live on the
  // Dashboard below the one-shot section, refreshing through the
  // same snapshot.
  const allowances: Allowance[] = data.allowances ?? [];
  // Display lists for the Activity toolbar's date range. One-shots
  // filter on their scheduled start; schedules and allowances on
  // their creation date (they recur / are ongoing budgets, so no
  // single event date exists). The full lists above stay in use for
  // edit forms and lookups.
  const now = new Date();
  const visibleSchedules = schedules.filter((s) =>
    inDateRange(s.created_at, range, now),
  );
  const visibleOneShots = oneShots.filter((o) =>
    inDateRange(o.fire_at_ms, range, now),
  );
  // Once a one-shot fires, the backend removes the pending `OneShot`
  // timer and creates a `Session` tagged with `one_shot_started`. Show
  // finished/interrupted one-shot sessions here so past one-shot
  // activity lives under "Start once at" rather than "Focus Sessions".
  const historicalOneShots = (data.sessions ?? []).filter(
    (s) => isHistoricalSession(s) && isOneShotSession(s),
  );
  const visibleHistoricalOneShots = historicalOneShots
    .filter((s) => inDateRange(s.started_at, range, now))
    .slice()
    .reverse();
  const visibleAllowances = allowances.filter((a) =>
    inDateRange(a.created_at, range, now),
  );
  const [editingAllowance, setEditingAllowance] = useState<Allowance | null>(
    null,
  );
  const [allowFormOpen, setAllowFormOpen] = useState(false);
  function resetOneForm() {
    setOneTemplateId('');
    setOneTitle('');
    setOneWhen('');
    setOneProblem('');
  }
  async function submitOneShot(e: React.FormEvent) {
    e.preventDefault();
    setOneProblem('');
    const fireAtMs = oneShotFireAtMs(oneWhen);
    const problem = validateOneShotForm({
      title: oneTitle,
      templateId: oneTemplateId,
      fireAtMs,
    });
    if (problem) {
      setOneProblem(problem);
      return;
    }
    const template = data.sessions.find((s) => s.id === oneTemplateId);
    if (!template || fireAtMs === null) {
      setOneProblem('Pick a session to use as the template.');
      return;
    }
    // Snapshot semantics, as with schedules: the template's focus
    // and pause setting are copied into the timer now; editing the
    // template later does not change it.
    if (
      await execute({
        type: 'create_one_shot',
        title: oneTitle,
        minutes: template.duration_minutes,
        allow_pause: template.allow_pause,
        focus: structuredClone(template.focus ?? defaultFocus),
        fire_at_ms: fireAtMs,
      })
    ) {
      resetOneForm();
      setOneFormOpen(false);
    }
  }
  function resetSchedLockForm() {
    setSchedLockPasswordOn(false);
    setSchedLockPassword('');
    setSchedUnlockPassword('');
    setSchedUnlockChallenge('');
  }
  function schedLockProblem(): string {
    // Editing: an empty field keeps the stored hash, so it is
    // only a problem when no hash exists yet.
    const keepingHash =
      !!schedEditingId && !!editingSchedule?.lock?.password_hash;
    if (schedLockPasswordOn && !schedLockPassword.trim() && !keepingHash)
      return 'Enter a password for the password lock.';
    return '';
  }
  function schedLockValue(
    existingHash?: string | null,
    force = false,
  ): {
    lock?: LockConfig;
    lock_password?: string;
  } {
    // The password lock is the only lock this form offers now:
    // challenge, delay and restart locks are always off (their
    // previous defaults), so an edit also clears a stored one.
    if (!schedLockPasswordOn && !force) return {};
    return {
      lock: {
        // The stored hash is sent back verbatim while the
        // password lock stays on; omitting it turns it off.
        password_hash: schedLockPasswordOn
          ? (existingHash ?? undefined)
          : undefined,
        challenge_length: 0,
        delay_minutes: 0,
        restart_lock: false,
      },
      lock_password:
        schedLockPasswordOn && schedLockPassword
          ? schedLockPassword
          : undefined,
    };
  }
  function resetSchedForm() {
    setSchedTemplateId('');
    setSchedTitle('');
    setSchedDays([false, true, true, true, true, true, false]);
    setSchedStart('09:00');
    setSchedEnd('10:00');
    setSchedEnabled(true);
    setSchedEditingId(null);
    setSchedProblem('');
    resetSchedLockForm();
  }
  function startSchedEdit(s: Schedule) {
    setSchedTitle(s.title);
    setSchedDays(DAY_LABELS.map((_, i) => s.days.includes(i)));
    setSchedStart(s.start_time);
    setSchedEnd(s.end_time);
    setSchedEnabled(s.enabled);
    setSchedEditingId(s.id);
    setSchedProblem('');
    // Load the lock for editing: the password itself is never
    // shown back (an empty field keeps the stored hash). A stored
    // challenge, delay or restart lock is dropped on save — the
    // form no longer offers those kinds.
    setSchedLockPasswordOn(!!s.lock?.password_hash);
    setSchedLockPassword('');
    setSchedUnlockPassword('');
    setSchedUnlockChallenge('');
    setSchedFormOpen(true);
  }
  async function submitSchedule(e: React.FormEvent) {
    e.preventDefault();
    setSchedProblem('');
    const days = schedDays.map((on, i) => (on ? i : -1)).filter((i) => i >= 0);
    const problem = validateScheduleForm({
      title: schedTitle,
      days,
      start_time: schedStart,
      end_time: schedEnd,
    });
    if (problem) {
      setSchedProblem(problem);
      return;
    }
    const lockProblem = schedLockProblem();
    if (lockProblem) {
      setSchedProblem(lockProblem);
      return;
    }
    if (schedEditingId) {
      // Editing a schedule edits its own snapshot: the stored focus
      // is sent back unchanged, never the template session's focus.
      const existing = schedules.find((s) => s.id === schedEditingId);
      if (!existing) {
        setSchedProblem('That schedule no longer exists.');
        return;
      }
      if (
        await execute({
          type: 'update_schedule',
          id: schedEditingId,
          title: schedTitle,
          days,
          start_time: schedStart,
          end_time: schedEnd,
          focus: existing.focus,
          enabled: schedEnabled,
          // The password lock is the only lock offered: window
          // changes and the other lock kinds stay off.
          lock_changes: false,
          ...schedLockValue(existing.lock?.password_hash ?? null, true),
          password: schedUnlockPassword || undefined,
          challenge_answer: schedUnlockChallenge || undefined,
        })
      ) {
        resetSchedForm();
        setSchedFormOpen(false);
      }
      return;
    }
    const template = data.sessions.find((s) => s.id === schedTemplateId);
    if (!template) {
      setSchedProblem('Pick a session to use as the template.');
      return;
    }
    // Snapshot semantics: the template's focus is copied into the
    // schedule now; editing the template later does not change it.
    if (
      await execute({
        type: 'create_schedule',
        title: schedTitle,
        days,
        start_time: schedStart,
        end_time: schedEnd,
        focus: structuredClone(template.focus ?? defaultFocus),
        enabled: schedEnabled,
        // The password lock is the only lock offered: window
        // changes and the other lock kinds stay off.
        lock_changes: false,
        ...schedLockValue(),
      })
    ) {
      resetSchedForm();
      setSchedFormOpen(false);
    }
  }
  return (
    <>
      {(!only || only === 'schedules') && (
        <section className="panel focus-planning-card">
          <div className="section-heading section-start">
            <h2>Schedules</h2>
          </div>
          <p className="small muted section-desc">
            Automatic blocks on your weekly schedule. Needs open at login to
            run.
          </p>
          {visibleSchedules.length ? (
            <div className="session-list">
              {visibleSchedules.map((s) => {
                const active = isScheduleActive(s);
                const lockedActive = isScheduleLockedActive(s);
                const schedLockText = lockLabel(s.lock);
                const hasStep10Gate =
                  !!s.lock &&
                  (!!s.lock.password_hash ||
                    (s.lock.challenge_length ?? 0) > 0 ||
                    (s.lock.delay_minutes ?? 0) > 0);
                const delayStatus = scheduleLockDelayStatus(s);
                const delayPending =
                  s.lock_state?.delay_for === 'change' &&
                  delayStatus.kind === 'pending';
                const delayDone =
                  s.lock_state?.delay_for === 'change' &&
                  delayStatus.kind === 'done';
                return (
                  <div className="panel" key={s.id}>
                    <h3>{s.title}</h3>
                    <p className="small muted">
                      {formatDays(s.days)} · {s.start_time}–{s.end_time}
                    </p>
                    <div className="row controls">
                      {active && <span className="status-pill">Active</span>}
                      {s.lock_changes && (
                        <span className="status-pill">Locked</span>
                      )}
                      {s.focus?.frozen?.enabled && (
                        <span className="status-pill">Frozen</span>
                      )}
                      {schedLockText && (
                        <span className="status-pill">
                          Lock: {schedLockText}
                        </span>
                      )}
                      {!s.enabled && <span className="status-pill">Off</span>}
                    </div>
                    {lockedActive && !hasStep10Gate && (
                      <p className="small muted">
                        Locked until this window ends.
                      </p>
                    )}
                    {lockedActive && hasStep10Gate && (
                      <p className="small muted">
                        Locked; unlock from Edit below.
                      </p>
                    )}
                    {s.focus?.frozen?.enabled && active && (
                      <p className="small muted">
                        Frozen until this window ends.
                      </p>
                    )}
                    {(s.lock?.delay_minutes ?? 0) > 0 && active && (
                      <div className="row controls">
                        {delayPending && (
                          <>
                            <span className="small muted">
                              Change unlocks in {delayStatus.minutesLeft} min
                            </span>
                            <button
                              type="button"
                              className="secondary"
                              disabled={busy}
                              onClick={async () => {
                                await execute({
                                  type: 'cancel_schedule_lock_delay',
                                  id: s.id,
                                });
                              }}
                            >
                              Cancel delay
                            </button>
                          </>
                        )}
                        {delayDone && (
                          <span className="small muted">
                            Delay done; you can change now.
                          </span>
                        )}
                        {!delayPending && !delayDone && (
                          <button
                            type="button"
                            className="secondary"
                            disabled={busy}
                            onClick={async () => {
                              await execute({
                                type: 'request_schedule_lock_delay',
                                id: s.id,
                              });
                            }}
                          >
                            Request change delay ({s.lock!.delay_minutes} min)
                          </button>
                        )}
                      </div>
                    )}
                    <div className="row controls">
                      <label className="check-label">
                        <input
                          type="checkbox"
                          checked={s.enabled}
                          disabled={lockedActive || busy}
                          onChange={async () => {
                            await execute({
                              type: 'update_schedule',
                              id: s.id,
                              title: s.title,
                              days: s.days,
                              start_time: s.start_time,
                              end_time: s.end_time,
                              focus: s.focus,
                              enabled: !s.enabled,
                              lock_changes: s.lock_changes,
                            });
                          }}
                        />
                        Enabled
                      </label>
                      <button
                        type="button"
                        className="secondary"
                        disabled={busy}
                        onClick={() => startSchedEdit(s)}
                      >
                        Edit
                      </button>
                      <button
                        type="button"
                        className="secondary"
                        disabled={lockedActive || busy}
                        onClick={async () => {
                          await execute({ type: 'delete_schedule', id: s.id });
                        }}
                      >
                        Delete
                      </button>
                    </div>
                  </div>
                );
              })}
            </div>
          ) : schedules.length ? (
            <Empty>No schedules in this period.</Empty>
          ) : (
            <Empty>No schedules yet.</Empty>
          )}
          {!schedFormOpen ? (
            <div className="row controls">
              <button
                type="button"
                onClick={() => {
                  // New schedules always start from the session wizard
                  // now — no saved session is needed first. The inline
                  // form below is only for editing an existing one.
                  if (onNewSchedule) {
                    onNewSchedule();
                    return;
                  }
                  resetSchedForm();
                  setSchedFormOpen(true);
                }}
              >
                <Plus size={16} />
                New schedule
              </button>
            </div>
          ) : (
            <form className="panel form-panel" onSubmit={submitSchedule}>
              <h2>{schedEditingId ? 'Edit schedule' : 'New schedule'}</h2>
              {!schedEditingId && (
                <label>
                  Template session
                  <Dropdown
                    value={schedTemplateId}
                    onChange={(value) => {
                      const chosen = data.sessions.find((s) => s.id === value);
                      setSchedTemplateId(value);
                      if (chosen) setSchedTitle(chosen.title);
                    }}
                    options={[
                      { value: '', label: 'Pick a session…' },
                      ...data.sessions.map((s) => ({
                        value: s.id,
                        label: `${s.title} · ${s.duration_minutes} min`,
                      })),
                    ]}
                  />
                </label>
              )}
              <label className="grow">
                Schedule name
                <input
                  required
                  maxLength={160}
                  value={schedTitle}
                  onChange={(e) => setSchedTitle(e.target.value)}
                  placeholder="e.g. Morning deep work"
                />
              </label>
              <fieldset>
                <legend>Days</legend>
                <div className="row controls">
                  {DAY_LABELS.map((label, i) => (
                    <label className="check-label" key={label}>
                      <input
                        type="checkbox"
                        checked={schedDays[i]}
                        onChange={() =>
                          setSchedDays((days) =>
                            days.map((on, j) => (j === i ? !on : on)),
                          )
                        }
                      />
                      {label}
                    </label>
                  ))}
                </div>
              </fieldset>
              <div className="simple-grid time-grid">
                <label>
                  Start time
                  <input
                    type="time"
                    required
                    value={schedStart}
                    onChange={(e) => setSchedStart(e.target.value)}
                  />
                </label>
                <label>
                  End time
                  <input
                    type="time"
                    required
                    value={schedEnd}
                    onChange={(e) => setSchedEnd(e.target.value)}
                  />
                </label>
                <label>
                  Total time
                  <input
                    type="text"
                    readOnly
                    tabIndex={-1}
                    value={scheduleTotalTime(schedStart, schedEnd)}
                  />
                </label>
              </div>
              <p className="small muted">Ends earlier? It runs overnight.</p>
              <label className="check-label">
                <input
                  type="checkbox"
                  checked={schedLockPasswordOn}
                  onChange={(e) => setSchedLockPasswordOn(e.target.checked)}
                />
                Lock with a password
              </label>
              {schedLockPasswordOn && (
                <label>
                  Lock password
                  <input
                    type="password"
                    value={schedLockPassword}
                    onChange={(e) => setSchedLockPassword(e.target.value)}
                    autoComplete="off"
                    placeholder={
                      editingSchedule?.lock?.password_hash
                        ? 'Leave empty to keep the existing password'
                        : undefined
                    }
                  />
                </label>
              )}
              {editingLockedActive && (
                <>
                  <p className="small muted">Locked now; enter proofs below.</p>
                  {editingSchedule?.lock?.password_hash && (
                    <label>
                      Password
                      <input
                        type="password"
                        value={schedUnlockPassword}
                        onChange={(e) => setSchedUnlockPassword(e.target.value)}
                        autoComplete="off"
                      />
                    </label>
                  )}
                  {(editingSchedule?.lock?.challenge_length ?? 0) > 0 &&
                    editingSchedule?.lock_state?.challenge && (
                      <label>
                        Type this challenge exactly:{' '}
                        <code>{editingSchedule.lock_state.challenge}</code>
                        <input
                          value={schedUnlockChallenge}
                          onChange={(e) =>
                            setSchedUnlockChallenge(e.target.value)
                          }
                          autoComplete="off"
                        />
                      </label>
                    )}
                </>
              )}
              <label className="check-label">
                <input
                  type="checkbox"
                  checked={schedEnabled}
                  onChange={(e) => setSchedEnabled(e.target.checked)}
                />
                Enabled
              </label>
              {schedProblem && (
                <p className="error small" role="alert">
                  {schedProblem}
                </p>
              )}
              <div
                className="row controls"
                style={{ justifyContent: 'flex-end' }}
              >
                {schedEditingId && editingLockedActive && (
                  <button
                    type="button"
                    className="secondary"
                    disabled={busy}
                    onClick={async () => {
                      if (
                        await execute({
                          type: 'delete_schedule',
                          id: schedEditingId,
                          password: schedUnlockPassword || undefined,
                          challenge_answer: schedUnlockChallenge || undefined,
                        })
                      ) {
                        resetSchedForm();
                        setSchedFormOpen(false);
                      }
                    }}
                  >
                    Delete schedule
                  </button>
                )}
                <button
                  type="button"
                  className="secondary"
                  onClick={() => {
                    resetSchedForm();
                    setSchedFormOpen(false);
                  }}
                >
                  Cancel
                </button>
                <button disabled={busy}>
                  {schedEditingId ? 'Save schedule' : 'Create schedule'}
                </button>
              </div>
            </form>
          )}
        </section>
      )}
      {(!only || only === 'oneshots') && (
        <section className="panel focus-planning-card">
          <div className="section-heading section-start">
            <h2>Start once at</h2>
          </div>
          <p className="small muted section-desc">
            One block at a chosen time.
          </p>
          {visibleOneShots.length || visibleHistoricalOneShots.length ? (
            <div className="session-list">
              {visibleOneShots.map((o) => (
                <div className="panel" key={o.id}>
                  <h3>{o.title}</h3>
                  <p className="small muted">
                    {new Date(o.fire_at_ms).toLocaleString()} · {o.minutes} min
                  </p>
                  <div className="row controls">
                    <button
                      type="button"
                      className="secondary"
                      disabled={busy}
                      aria-label="Cancel timer"
                      onClick={async () => {
                        await execute({ type: 'delete_one_shot', id: o.id });
                      }}
                    >
                      Cancel
                    </button>
                  </div>
                </div>
              ))}
              {visibleHistoricalOneShots.map((s) => (
                <SessionCard
                  key={s.id}
                  session={s}
                  showStartedAt
                  hideStart
                  setup={setup}
                  onSetupRefresh={onSetupRefresh}
                />
              ))}
            </div>
          ) : oneShots.length || historicalOneShots.length ? (
            <Empty>No one-shot timers in this period.</Empty>
          ) : (
            <Empty>No one-shot timers yet.</Empty>
          )}
          {!oneFormOpen ? (
            <div className="row controls">
              <button
                type="button"
                onClick={() => {
                  // New one-shots always start from the session wizard
                  // now — no saved session is needed first.
                  if (onNewOneShot) {
                    onNewOneShot();
                    return;
                  }
                  resetOneForm();
                  setOneFormOpen(true);
                }}
              >
                <Plus size={16} />
                New one-shot
              </button>
            </div>
          ) : (
            <form className="panel form-panel" onSubmit={submitOneShot}>
              <h2>New one-shot</h2>
              <label>
                One-shot template
                <Dropdown
                  value={oneTemplateId}
                  onChange={(value) => {
                    const chosen = data.sessions.find((s) => s.id === value);
                    setOneTemplateId(value);
                    if (chosen) setOneTitle(chosen.title);
                  }}
                  options={[
                    { value: '', label: 'Pick a session…' },
                    ...data.sessions.map((s) => ({
                      value: s.id,
                      label: `${s.title} · ${s.duration_minutes} min`,
                    })),
                  ]}
                />
              </label>
              <label className="grow">
                One-shot name
                <input
                  required
                  maxLength={160}
                  value={oneTitle}
                  onChange={(e) => setOneTitle(e.target.value)}
                  placeholder="e.g. Evening deep work"
                />
              </label>
              <label>
                Start at
                <input
                  type="datetime-local"
                  required
                  value={oneWhen}
                  onChange={(e) => setOneWhen(e.target.value)}
                />
              </label>
              {oneProblem && (
                <p className="error small" role="alert">
                  {oneProblem}
                </p>
              )}
              <div
                className="row controls"
                style={{ justifyContent: 'flex-end' }}
              >
                <button
                  type="button"
                  className="secondary"
                  onClick={() => {
                    resetOneForm();
                    setOneFormOpen(false);
                  }}
                >
                  Cancel
                </button>
                <button disabled={busy}>Create one-shot</button>
              </div>
            </form>
          )}
        </section>
      )}
      {(!only || only === 'allowances') && (
        <section className="panel focus-planning-card">
          <div className="section-heading section-start">
            <h2>Allowances</h2>
          </div>
          <p className="small muted section-desc">
            Daily or weekly time allowances. Blocks always lift on reset.
          </p>
          {visibleAllowances.length ? (
            <div className="session-list">
              {visibleAllowances.map((a) => {
                const exhausted = allowanceExhausted(a);
                const remaining = allowanceRemainingMs(a);
                return (
                  <div className="panel" key={a.id}>
                    <h3>{a.title}</h3>
                    <p className="small muted">
                      {a.kind === 'site' ? 'Site' : 'App'} · {a.target} ·{' '}
                      {allowancePeriodLabel(a)}
                    </p>
                    <p className="small muted">
                      {exhausted
                        ? 'Allowance used up; blocked until reset.'
                        : `${formatAllowanceMinutes(remaining)} left of ${formatAllowanceMinutes(a.budget_minutes * 60_000)}`}
                    </p>
                    <div className="row controls">
                      {exhausted && (
                        <span className="status-pill">Exhausted</span>
                      )}
                      {!a.enabled && <span className="status-pill">Off</span>}
                    </div>
                    <div className="row controls">
                      <label className="check-label">
                        <input
                          type="checkbox"
                          checked={a.enabled}
                          disabled={busy}
                          onChange={async () => {
                            await execute({
                              type: 'update_allowance',
                              allowance: { ...a, enabled: !a.enabled },
                            });
                          }}
                        />
                        Enabled
                      </label>
                      <button
                        type="button"
                        className="secondary"
                        disabled={busy}
                        onClick={() => {
                          if (onEditAllowance) {
                            onEditAllowance(a);
                            return;
                          }
                          setEditingAllowance(a);
                          setAllowFormOpen(true);
                        }}
                      >
                        Edit
                      </button>
                      <button
                        type="button"
                        className="secondary"
                        disabled={busy}
                        onClick={async () => {
                          await execute({ type: 'delete_allowance', id: a.id });
                        }}
                      >
                        Delete
                      </button>
                    </div>
                  </div>
                );
              })}
            </div>
          ) : allowances.length ? (
            <Empty>No allowances in this period.</Empty>
          ) : (
            <Empty>No allowances yet.</Empty>
          )}
          {!allowFormOpen ? (
            <div className="row controls">
              <button
                type="button"
                onClick={() => {
                  if (onNewAllowance) {
                    onNewAllowance();
                    return;
                  }
                  setEditingAllowance(null);
                  setAllowFormOpen(true);
                }}
              >
                <Plus size={16} />
                New allowance
              </button>
            </div>
          ) : (
            <AllowanceForm
              key={editingAllowance?.id ?? 'new'}
              className="panel form-panel"
              editing={editingAllowance}
              onSaved={() => {
                setEditingAllowance(null);
                setAllowFormOpen(false);
              }}
              onCancel={() => {
                setEditingAllowance(null);
                setAllowFormOpen(false);
              }}
            />
          )}
        </section>
      )}
    </>
  );
}
