import { useState } from 'react';
import { Plus } from 'lucide-react';
import { useAppStore } from '../state/store';
import { Empty } from './ui';
import { Dropdown } from './Dropdown';
import { cleanDomain } from '../services/domains';
import {
  DAY_LABELS,
  formatDays,
  isScheduleActive,
  isScheduleLockedActive,
  scheduleLockDelayStatus,
  validateScheduleForm,
} from '../services/schedules';
import { oneShotFireAtMs, validateOneShotForm } from '../services/one_shots';
import {
  allowanceExhausted,
  allowancePeriodLabel,
  allowanceRemainingMs,
  formatAllowanceMinutes,
  validateAllowanceForm,
} from '../services/allowances';
import {
  defaultFocus,
  lockLabel,
  type Allowance,
  type LockConfig,
  type OneShot,
  type Schedule,
} from '../types';
// Schedules, one-shot timers, and allowances, moved here from the
// Focus Sessions page: they live on the Focus Sessions page as cards
// in a 2x2 grid next to the new-session card, in the same order, with
// the same forms, lists, and behavior. `only` renders a single section
// as one grid card; with no `only`, all three render (as before).
// `onNewSchedule` / `onNewOneShot` open the session wizard in
// schedule / one-shot mode instead of the inline template form; the
// inline form is still used for editing, and when no callback is
// passed (standalone use) the New button opens it as before.
export function PlanningSections({
  only,
  onNewSchedule,
  onNewOneShot,
}: {
  only?: 'schedules' | 'oneshots' | 'allowances';
  onNewSchedule?: () => void;
  onNewOneShot?: () => void;
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
  const [schedLock, setSchedLock] = useState(false);
  const [schedEnabled, setSchedEnabled] = useState(true);
  // Step 10 lock for schedules (same fields as the session wizard
  // lock, kept separate because the two forms are independent).
  const [schedLockPasswordOn, setSchedLockPasswordOn] = useState(false);
  const [schedLockPassword, setSchedLockPassword] = useState('');
  const [schedLockChallengeOn, setSchedLockChallengeOn] = useState(false);
  const [schedLockChallengeLength, setSchedLockChallengeLength] = useState(32);
  const [schedLockDelayOn, setSchedLockDelayOn] = useState(false);
  const [schedLockDelayMinutes, setSchedLockDelayMinutes] = useState(5);
  const [schedLockRestart, setSchedLockRestart] = useState(false);
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
  const [allowKind, setAllowKind] = useState<'site' | 'app'>('site');
  const [allowTitle, setAllowTitle] = useState('');
  const [allowTarget, setAllowTarget] = useState('');
  const [allowBudget, setAllowBudget] = useState(30);
  const [allowPeriod, setAllowPeriod] = useState<'day' | 'week'>('day');
  const [allowMode, setAllowMode] = useState<'rolling' | 'fixed'>('rolling');
  const [allowEnabled, setAllowEnabled] = useState(true);
  const [allowEditingId, setAllowEditingId] = useState<string | null>(null);
  const [allowFormOpen, setAllowFormOpen] = useState(false);
  const [allowProblem, setAllowProblem] = useState('');
  function resetAllowForm() {
    setAllowKind('site');
    setAllowTitle('');
    setAllowTarget('');
    setAllowBudget(30);
    setAllowPeriod('day');
    setAllowMode('rolling');
    setAllowEnabled(true);
    setAllowEditingId(null);
    setAllowProblem('');
  }
  function startAllowEdit(a: Allowance) {
    setAllowKind(a.kind);
    setAllowTitle(a.title);
    setAllowTarget(a.target);
    setAllowBudget(a.budget_minutes);
    setAllowPeriod(a.period);
    setAllowMode(a.mode);
    setAllowEnabled(a.enabled);
    setAllowEditingId(a.id);
    setAllowProblem('');
    setAllowFormOpen(true);
  }
  async function submitAllowance(e: React.FormEvent) {
    e.preventDefault();
    setAllowProblem('');
    // Site targets go through the same cleaner as the wizard's
    // website rules (scheme stripped, host lowercased); the backend
    // still validates, and rejects paths and search: keywords.
    const target =
      allowKind === 'site'
        ? (cleanDomain(allowTarget) ?? allowTarget.trim())
        : allowTarget.trim();
    const problem = validateAllowanceForm({
      title: allowTitle,
      kind: allowKind,
      target,
      budgetMinutes: allowBudget,
      period: allowPeriod,
    });
    if (problem) {
      setAllowProblem(problem);
      return;
    }
    if (allowEditingId) {
      const existing = allowances.find((a) => a.id === allowEditingId);
      if (!existing) {
        setAllowProblem('That allowance no longer exists.');
        return;
      }
      if (
        await execute({
          type: 'update_allowance',
          allowance: {
            ...existing,
            title: allowTitle.trim(),
            kind: allowKind,
            target,
            budget_minutes: allowBudget,
            period: allowPeriod,
            mode: allowMode,
            enabled: allowEnabled,
          },
        })
      ) {
        resetAllowForm();
        setAllowFormOpen(false);
      }
      return;
    }
    if (
      await execute({
        type: 'create_allowance',
        title: allowTitle.trim(),
        kind: allowKind,
        target,
        budget_minutes: allowBudget,
        period: allowPeriod,
        mode: allowMode,
        enabled: allowEnabled,
      })
    ) {
      resetAllowForm();
      setAllowFormOpen(false);
    }
  }
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
    setSchedLockChallengeOn(false);
    setSchedLockChallengeLength(32);
    setSchedLockDelayOn(false);
    setSchedLockDelayMinutes(5);
    setSchedLockRestart(false);
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
    if (
      schedLockChallengeOn &&
      !(schedLockChallengeLength >= 8 && schedLockChallengeLength <= 64)
    )
      return 'Challenge length must be 8-64 characters.';
    if (
      schedLockDelayOn &&
      !(schedLockDelayMinutes >= 1 && schedLockDelayMinutes <= 60)
    )
      return 'Delay must be 1-60 minutes.';
    return '';
  }
  function schedLockValue(
    existingHash?: string | null,
    force = false,
  ): {
    lock?: LockConfig;
    lock_password?: string;
  } {
    const any =
      schedLockPasswordOn ||
      schedLockChallengeOn ||
      schedLockDelayOn ||
      schedLockRestart;
    if (!any && !force) return {};
    return {
      lock: {
        // The stored hash is sent back verbatim while the
        // password lock stays on; omitting it turns it off.
        password_hash: schedLockPasswordOn
          ? (existingHash ?? undefined)
          : undefined,
        challenge_length: schedLockChallengeOn ? schedLockChallengeLength : 0,
        delay_minutes: schedLockDelayOn ? schedLockDelayMinutes : 0,
        restart_lock: schedLockRestart,
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
    setSchedLock(false);
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
    setSchedLock(s.lock_changes);
    setSchedEnabled(s.enabled);
    setSchedEditingId(s.id);
    setSchedProblem('');
    // Load the lock for editing: the password itself is never
    // shown back (an empty field keeps the stored hash).
    setSchedLockPasswordOn(!!s.lock?.password_hash);
    setSchedLockPassword('');
    setSchedLockChallengeOn((s.lock?.challenge_length ?? 0) > 0);
    setSchedLockChallengeLength(s.lock?.challenge_length ?? 32);
    setSchedLockDelayOn((s.lock?.delay_minutes ?? 0) > 0);
    setSchedLockDelayMinutes(s.lock?.delay_minutes ?? 5);
    setSchedLockRestart(!!s.lock?.restart_lock);
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
          lock_changes: schedLock,
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
        lock_changes: schedLock,
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
          {schedules.length ? (
            <div className="session-list">
              {schedules.map((s) => {
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
          ) : (
            <div className="panel">
              <Empty>No schedules yet.</Empty>
            </div>
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
              </div>
              <p className="small muted">Ends earlier? It runs overnight.</p>
              <label className="check-label">
                <input
                  type="checkbox"
                  checked={schedLock}
                  onChange={(e) => setSchedLock(e.target.checked)}
                />
                Lock changes during the window
              </label>
              <p className="small muted">
                Locked windows can&apos;t be changed early.
              </p>
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
              <label className="check-label">
                <input
                  type="checkbox"
                  checked={schedLockChallengeOn}
                  onChange={(e) => setSchedLockChallengeOn(e.target.checked)}
                />
                Lock with a typed challenge
              </label>
              {schedLockChallengeOn && (
                <label>
                  Challenge length (characters, 8-64)
                  <input
                    type="number"
                    min={8}
                    max={64}
                    value={schedLockChallengeLength}
                    onChange={(e) =>
                      setSchedLockChallengeLength(Number(e.target.value))
                    }
                  />
                </label>
              )}
              <label className="check-label">
                <input
                  type="checkbox"
                  checked={schedLockDelayOn}
                  onChange={(e) => setSchedLockDelayOn(e.target.checked)}
                />
                Lock with a delay
              </label>
              {schedLockDelayOn && (
                <label>
                  Delay (minutes, 1-60)
                  <input
                    type="number"
                    min={1}
                    max={60}
                    value={schedLockDelayMinutes}
                    onChange={(e) =>
                      setSchedLockDelayMinutes(Number(e.target.value))
                    }
                  />
                </label>
              )}
              <label className="check-label">
                <input
                  type="checkbox"
                  checked={schedLockRestart}
                  onChange={(e) => setSchedLockRestart(e.target.checked)}
                />
                Keep the lock after a restart
              </label>
              {(schedLockPasswordOn ||
                schedLockChallengeOn ||
                schedLockDelayOn) && (
                <p className="small muted">Locks pass together, lift at end.</p>
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
          {oneShots.length ? (
            <div className="session-list">
              {oneShots.map((o) => (
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
            </div>
          ) : (
            <div className="panel">
              <Empty>No one-shot timers yet.</Empty>
            </div>
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
            Daily or weekly time budgets. Blocks always lift on reset.
          </p>
          {allowances.length ? (
            <div className="session-list">
              {allowances.map((a) => {
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
                        ? 'Budget used up; blocked until reset.'
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
                        onClick={() => startAllowEdit(a)}
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
          ) : (
            <div className="panel">
              <Empty>No allowances yet.</Empty>
            </div>
          )}
          {!allowFormOpen ? (
            <div className="row controls">
              <button
                type="button"
                onClick={() => {
                  resetAllowForm();
                  setAllowFormOpen(true);
                }}
              >
                <Plus size={16} />
                New allowance
              </button>
            </div>
          ) : (
            <form className="panel form-panel" onSubmit={submitAllowance}>
              <h2>{allowEditingId ? 'Edit allowance' : 'New allowance'}</h2>
              <label className="grow">
                Allowance name
                <input
                  required
                  maxLength={160}
                  value={allowTitle}
                  onChange={(e) => setAllowTitle(e.target.value)}
                  placeholder="e.g. YouTube"
                />
              </label>
              <label>
                Budget for
                <Dropdown
                  value={allowKind}
                  onChange={(value) => {
                    setAllowKind(value as 'site' | 'app');
                    setAllowProblem('');
                  }}
                  options={[
                    { value: 'site', label: 'A site' },
                    { value: 'app', label: 'An app' },
                  ]}
                />
              </label>
              <label className="grow">
                {allowKind === 'site' ? 'Site' : 'App id'}
                <input
                  required
                  value={allowTarget}
                  onChange={(e) => setAllowTarget(e.target.value)}
                  placeholder={
                    allowKind === 'site'
                      ? 'e.g. youtube.com or *.example.com'
                      : 'e.g. com.example.game'
                  }
                />
              </label>
              {allowKind === 'site' && (
                <p className="small muted">
                  Domain only; no paths or searches.
                </p>
              )}
              <div className="simple-grid allowance-grid">
                <label>
                  Budget (minutes)
                  <input
                    type="number"
                    required
                    min={1}
                    max={allowPeriod === 'day' ? 1440 : 10080}
                    value={allowBudget}
                    onChange={(e) => setAllowBudget(Number(e.target.value))}
                  />
                </label>
                <label>
                  Period
                  <Dropdown
                    value={allowPeriod}
                    onChange={(value) =>
                      setAllowPeriod(value as 'day' | 'week')
                    }
                    options={[
                      { value: 'day', label: 'Daily' },
                      { value: 'week', label: 'Weekly' },
                    ]}
                  />
                </label>
                <label>
                  Reset
                  <Dropdown
                    value={allowMode}
                    onChange={(value) =>
                      setAllowMode(value as 'rolling' | 'fixed')
                    }
                    options={[
                      {
                        value: 'rolling',
                        label: 'Rolling (last 24h / 7 days)',
                      },
                      {
                        value: 'fixed',
                        label: 'Fixed (calendar day / Monday week)',
                      },
                    ]}
                  />
                </label>
              </div>
              <label className="check-label">
                <input
                  type="checkbox"
                  checked={allowEnabled}
                  onChange={(e) => setAllowEnabled(e.target.checked)}
                />
                Enabled
              </label>
              {allowProblem && (
                <p className="error small" role="alert">
                  {allowProblem}
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
                    resetAllowForm();
                    setAllowFormOpen(false);
                  }}
                >
                  Cancel
                </button>
                <button disabled={busy}>
                  {allowEditingId ? 'Save allowance' : 'Create allowance'}
                </button>
              </div>
            </form>
          )}
        </section>
      )}
    </>
  );
}
