import { useState } from 'react';
import { ArrowLeft, Plus, RefreshCw } from 'lucide-react';
import { useAppStore } from '../state/store';
import { api, desktop } from '../services/api';
import { setupProblem, splitDomains } from '../services/focusSetup';
import { Empty, PageHeading } from '../components/ui';
import { Dropdown } from '../components/Dropdown';
import { BrowserSetup } from '../components/BrowserSetup';
import { SessionConfirmDialog } from '../components/SessionConfirmDialog';
import { ScheduleConfirmDialog } from '../components/ScheduleConfirmDialog';
import type { ScheduleCreateFields } from '../components/ScheduleConfirmDialog';
import { OneShotConfirmDialog } from '../components/OneShotConfirmDialog';
import { PlanningSections } from '../components/PlanningSections';
import { Activity } from './Activity';
import {
  cleanDomain,
  cleanYouTubeChannel,
  domainInputError,
  normalizeDomainInput,
  normalizeYouTubeChannelInput,
  youTubeChannelInputError,
} from '../services/domains';
import { useFocusSetup } from '../hooks/useFocusSetup';
import {
  defaultFocus,
  sameBrowser,
  type AttentionPolicy,
  type FocusConfig,
  type SavedSetup,
} from '../types';
const CAMERA_GUIDE =
  'Choose Allow in the macOS prompt, then click Check camera.';
export function Sessions() {
  const { data, execute, busy } = useAppStore();
  const [step, setStep] = useState(0),
    [title, setTitle] = useState(''),
    [minutes, setMinutes] = useState(data.preferences.default_minutes);
  const [focus, setFocus] = useState<FocusConfig>(
    structuredClone(defaultFocus),
  );
  const [workApps, setWorkApps] = useState<string[]>([]),
    [blockSites, setBlockSites] = useState('');
  const [blockSitesError, setBlockSitesError] = useState('');
  const [ytBlocked, setYtBlocked] = useState('');
  const [ytBlockedError, setYtBlockedError] = useState('');
  const [ytAllowed, setYtAllowed] = useState('');
  const [ytAllowedError, setYtAllowedError] = useState('');
  const [ytMode, setYtMode] = useState<'none' | 'block' | 'study'>('none');
  const [ytWatchLater, setYtWatchLater] = useState(false);
  const [blockModeOverride, setBlockModeOverride] = useState<
    'none' | 'blocklist' | 'allowlist' | null
  >(null);
  const [guide, setGuide] = useState(false),
    [rechecking, setRechecking] = useState(false),
    [problem, setProblem] = useState('');
  // Confirmation popup: "Start session" creates the session and starts
  // it immediately; "Cancel" discards the wizard state — nothing is
  // saved, and old sessions can no longer be reused from this page.
  // The wizard can also run in schedule or one-shot mode (opened from
  // the planning cards): same steps, but the final popup collects the
  // timing and creates a schedule / one-shot instead of a session.
  const [wizardMode, setWizardMode] = useState<
    'start' | 'schedule' | 'oneshot'
  >('start');
  const [scheduleConfirmOpen, setScheduleConfirmOpen] = useState(false);
  const [oneShotConfirmOpen, setOneShotConfirmOpen] = useState(false);
  function startWizard(mode: 'start' | 'schedule' | 'oneshot') {
    setProblem('');
    setMinutes(data.preferences.default_minutes);
    setStep(0);
    setMaxStep(0);
    setWizardMode(mode);
    setCreating(true);
  }
  function resetWizard() {
    setTitle('');
    setStep(0);
    setMaxStep(0);
    setCreating(false);
    setConfirmOpen(false);
    setScheduleConfirmOpen(false);
    setOneShotConfirmOpen(false);
    setWizardMode('start');
    setProblem('');
  }
  async function confirmStartSession() {
    const before = new Set(data.sessions.map((s) => s.id));
    const ok = await execute({
      type: 'create_session',
      title,
      minutes,
      allow_pause: !strict,
      focus: selected,
    });
    if (!ok) return;
    const created = useAppStore
      .getState()
      .data.sessions.find((s) => !before.has(s.id));
    resetWizard();
    if (created) await execute({ type: 'start_session', id: created.id });
  }
  // Schedule / one-shot mode: the wizard's FocusConfig is the snapshot,
  // exactly as creating from a template session would copy it.
  async function confirmCreateSchedule(fields: ScheduleCreateFields) {
    const ok = await execute({
      type: 'create_schedule',
      title: fields.title,
      days: fields.days,
      start_time: fields.start_time,
      end_time: fields.end_time,
      focus: selected,
      enabled: fields.enabled,
      lock_changes: fields.lock_changes,
      ...(fields.lock ? { lock: fields.lock } : {}),
      ...(fields.lock_password ? { lock_password: fields.lock_password } : {}),
    });
    if (ok) resetWizard();
  }
  async function confirmCreateOneShot(fields: {
    title: string;
    fire_at_ms: number;
  }) {
    const ok = await execute({
      type: 'create_one_shot',
      title: fields.title,
      minutes,
      allow_pause: !strict,
      focus: selected,
      fire_at_ms: fields.fire_at_ms,
    });
    if (ok) resetWizard();
  }
  // One browser per session: shown when the user tries to pick a second.
  const [browserWarn, setBrowserWarn] = useState('');
  const [maxStep, setMaxStep] = useState(0);
  const [creating, setCreating] = useState(false);
  const [confirmOpen, setConfirmOpen] = useState(false);
  function applySavedSetup(preset: SavedSetup) {
    const f = structuredClone(preset.focus);
    // Saved before Manual check-ins was removed: use Apps only instead.
    if (
      f.attention &&
      (f.attention.verification as string) === 'manual_checkpoints'
    )
      f.attention.verification = 'activity_only';
    f.camera = false;
    f.activity_consent = false;
    setTitle(preset.name);
    setMinutes(preset.minutes);
    setFocus(f);
    setWorkApps(f.mode === 'strict' ? f.apps : (f.attention?.work_apps ?? []));
    const savedDomains = f.domains ?? [];
    setBlockSites(savedDomains.join('\n'));
    setBlockSitesError('');
    setYtBlocked((f.youtube_blocked_channels ?? []).join('\n'));
    setYtBlockedError('');
    setYtAllowed((f.youtube_allowed_channels ?? []).join('\n'));
    setYtAllowedError('');
    setYtMode(
      f.youtube_study_mode
        ? 'study'
        : (f.youtube_blocked_channels?.length ?? 0) > 0
          ? 'block'
          : 'none',
    );
    setYtWatchLater(!!f.youtube_allow_watch_later);
    // Channel rules live under website blocking too: a setup saved
    // with only channel rules still opens the blocking section.
    const hasYouTube =
      (f.youtube_blocked_channels?.length ?? 0) > 0 ||
      (f.youtube_allowed_channels?.length ?? 0) > 0 ||
      !!f.youtube_study_mode ||
      !!f.youtube_allow_watch_later;
    if (savedDomains.length || hasYouTube) {
      const wb = f.website_blocking ?? 'auto';
      const effectiveWb = hasYouTube && wb === 'none' ? 'blocklist' : wb;
      setBlockModeOverride(
        effectiveWb === 'auto'
          ? f.mode === 'strict'
            ? 'allowlist'
            : 'blocklist'
          : effectiveWb,
      );
    } else {
      setBlockModeOverride(f.mode === 'strict' ? 'allowlist' : null);
    }
    setStep(0);
    setMaxStep(0);
    setProblem('');
    // Saved setups always start a session now, never a schedule.
    setWizardMode('start');
    setCreating(true);
  }
  const attention = focus.attention!;
  // Pomodoro config for the wizard: a session saved before step 7 has
  // none, so fall back to the defaults until the user touches it.
  const pomodoro = focus.pomodoro ?? defaultFocus.pomodoro!;
  const update = (patch: Partial<FocusConfig>) =>
    setFocus((c) => ({ ...c, ...patch }));
  const updatePomodoro = (patch: Partial<typeof pomodoro>) =>
    update({ pomodoro: { ...pomodoro, ...patch } });
  const updateAttention = (patch: Partial<AttentionPolicy>) =>
    setFocus((c) => ({ ...c, attention: { ...c.attention!, ...patch } }));
  const strict = focus.mode === 'strict';
  // Website blocking choice. Defaults follow the session style unless the
  // user picks an option explicitly.
  const blockMode = blockModeOverride ?? (strict ? 'allowlist' : 'none');
  const setBlockMode = (mode: 'none' | 'blocklist' | 'allowlist') =>
    setBlockModeOverride(blockMode === mode ? 'none' : mode);
  // YouTube channel rules mirror the website blocking choice: the two
  // options are mutually exclusive, and unticking the active one clears
  // the choice. Typed text is kept, so re-ticking restores it.
  const setYtModeChoice = (mode: 'block' | 'study') =>
    setYtMode(ytMode === mode ? 'none' : mode);
  const { setup, error, loading, refresh } = useFocusSetup(
    focus.browser_app || undefined,
  );
  const browsers = setup?.browsers ?? [];
  const browserId = focus.browser_app || browsers[0]?.id || '';
  // "Connected" counts only when the paired extension reports that it
  // lives in the browser picked above. A connection from another browser —
  // or one that cannot say which browser it is in — does not count.
  const browserConnected =
    !!setup?.browser.connected &&
    !!setup?.browser.client_browser &&
    (!browserId || sameBrowser(browserId, setup.browser.client_browser));
  const selected: FocusConfig = {
    ...focus,
    pomodoro,
    // Rewards and Frozen are no longer offered in the wizard: force
    // both off (a saved setup could still carry them enabled).
    reward: { ...defaultFocus.reward!, enabled: false },
    frozen: { ...defaultFocus.frozen!, enabled: false },
    apps: strict ? workApps : focus.apps,
    blocked_folders: [],
    blocked_titles: [],
    browser_app: browserId,
    general_internet: focus.general_internet,
    // Submit the cleaned rules, not the raw text: a pasted URL's port or
    // query is stripped by cleanDomain, and the submit guard above has
    // already rejected entries that clean to nothing.
    domains:
      blockMode !== 'none'
        ? splitDomains(blockSites).flatMap((entry) => {
            const cleaned = cleanDomain(entry);
            return cleaned ? [cleaned] : [];
          })
        : [],
    // YouTube channel rules: the block list is not submitted in
    // study mode (the allowed list decides what plays), but its
    // textarea state is kept, so turning study mode off restores
    // what was typed.
    youtube_blocked_channels:
      ytMode === 'block'
        ? splitDomains(ytBlocked).flatMap((entry) => {
            const cleaned = cleanYouTubeChannel(entry);
            return cleaned ? [cleaned] : [];
          })
        : [],
    youtube_allowed_channels:
      ytMode === 'study'
        ? splitDomains(ytAllowed).flatMap((entry) => {
            const cleaned = cleanYouTubeChannel(entry);
            return cleaned ? [cleaned] : [];
          })
        : [],
    youtube_study_mode: ytMode === 'study',
    youtube_allow_watch_later: ytWatchLater,
    website_blocking: blockMode,
    camera:
      setup?.platform !== 'windows' &&
      attention.verification === 'camera_and_activity',
    attention: {
      ...attention,
      verification:
        setup?.platform === 'windows'
          ? 'activity_only'
          : attention.verification,
      work_apps: workApps,
      work_domains: [],
    },
  };
  const apps =
    setup?.applications.filter(
      (a) =>
        !/(preelock|finder|activitymonitor|systempreferences|explorer\.exe|taskmgr\.exe|systemsettings\.exe)/i.test(
          a.id,
        ),
    ) ?? [];
  async function cameraPermission() {
    try {
      await api.requestCamera();
      const s = await refresh();
      setProblem(s?.camera_permission === 'granted' ? '' : CAMERA_GUIDE);
    } catch (e) {
      setProblem(String(e));
    }
  }
  // Stale problems clear themselves once the underlying issue is fixed.
  let visibleProblem = problem;
  if (problem === CAMERA_GUIDE && setup?.camera_permission === 'granted')
    visibleProblem = '';
  if (problem.includes('extension') && browserConnected) visibleProblem = '';
  return (
    <>
      <PageHeading
        eyebrow=""
        title="Focus Sessions"
        description="Choose a task. Pick your work apps. Start focusing."
      />
      {!creating && !!data.preferences.saved_setups?.length && (
        <div className="saved-setup-picker">
          <h2>Saved setups</h2>
          <div className="row controls">
            {data.preferences.saved_setups.map((p) => (
              <button
                className="secondary"
                key={p.id}
                onClick={() => applySavedSetup(p)}
              >
                {p.name}
                <span className="small muted">{p.minutes} min</span>
              </button>
            ))}
          </div>
        </div>
      )}
      {creating ? (
        <>
          <div className="page-back">
            <button
              type="button"
              className="secondary"
              onClick={() => {
                setProblem('');
                if (step === 0) {
                  setCreating(false);
                  setMaxStep(0);
                } else setStep(step - 1);
              }}
            >
              <ArrowLeft size={16} />
              Back
            </button>
          </div>
          <form
            className="panel form-panel simple-session"
            onSubmit={async (e) => {
              e.preventDefault();
              setProblem('');
              if (step === 0) {
                setStep(1);
                setMaxStep(1);
                return;
              }
              // In blocklist mode a channel rule counts as a rule too:
              // a block-listed channel, or study mode on its own
              // (which blocks every channel until one is allowed).
              // Allowlist mode still needs a domain list.
              const hasYouTubeRule = ytBlocked.trim() || ytMode === 'study';
              if (
                selected.general_internet &&
                blockMode !== 'none' &&
                !blockSites.trim() &&
                !(blockMode === 'blocklist' && hasYouTubeRule)
              ) {
                setProblem(
                  'Add at least one website above, or turn off website blocking.',
                );
                return;
              }
              // Guard on the entries actually submitted (split on lines
              // and commas), not just the per-line error: a comma list
              // that the line checker flags is still valid input.
              const badEntry =
                blockMode !== 'none'
                  ? splitDomains(blockSites).find(
                      (entry) => cleanDomain(entry) === null,
                    )
                  : undefined;
              if (badEntry) {
                setProblem(`Invalid website: ${badEntry}`);
                return;
              }
              // Guards on the channel lists actually submitted.
              const badBlocked =
                blockMode !== 'none' && ytMode === 'block'
                  ? splitDomains(ytBlocked).find(
                      (entry) => cleanYouTubeChannel(entry) === null,
                    )
                  : undefined;
              if (badBlocked) {
                setProblem(`Invalid YouTube channel: ${badBlocked}`);
                return;
              }
              const badAllowed =
                blockMode !== 'none'
                  ? splitDomains(ytAllowed).find(
                      (entry) => cleanYouTubeChannel(entry) === null,
                    )
                  : undefined;
              if (badAllowed) {
                setProblem(`Invalid YouTube channel: ${badAllowed}`);
                return;
              }
              const issue = setupProblem(selected, setup, step === 2);
              if (issue) {
                setProblem(issue);
                if (issue.includes('extension')) setGuide(true);
                return;
              }
              if (step === 1) {
                setStep(2);
                setMaxStep(2);
                return;
              }
              // The final popup depends on how the wizard was opened:
              // start the session now, or set the timing for a
              // schedule / one-shot built from this setup.
              if (wizardMode === 'schedule') {
                setScheduleConfirmOpen(true);
              } else if (wizardMode === 'oneshot') {
                setOneShotConfirmOpen(true);
              } else {
                setConfirmOpen(true);
              }
            }}
          >
            <div className="wizard-steps" aria-label="Setup progress">
              {['Task', 'Apps & websites', 'Ready'].map((label, i) => (
                <button
                  key={label}
                  type="button"
                  className={step === i ? 'current' : ''}
                  disabled={i > maxStep}
                  onClick={() => {
                    setProblem('');
                    setStep(i);
                  }}
                  aria-current={step === i ? 'step' : undefined}
                >
                  {i + 1}. {label}
                </button>
              ))}
            </div>
            {step === 0 && (
              <>
                <h2>What will you work on?</h2>
                <div className="simple-grid">
                  <label className="grow">
                    Task name
                    <input
                      required
                      maxLength={160}
                      value={title}
                      onChange={(e) => setTitle(e.target.value)}
                      placeholder="e.g. Finish my coding assignment"
                    />
                  </label>
                  <label>
                    Focus time (minutes)
                    <input
                      type="number"
                      required
                      min={1}
                      max={480}
                      value={minutes}
                      onChange={(e) => setMinutes(Number(e.target.value))}
                    />
                  </label>
                </div>
                <label>
                  Type of work
                  <Dropdown
                    value={attention.task}
                    onChange={(value) => updateAttention({ task: value })}
                    options={[
                      { value: 'other', label: 'General work' },
                      { value: 'coding', label: 'Coding' },
                      { value: 'reading', label: 'Reading / study' },
                      { value: 'lecture', label: 'Watching a lesson' },
                      { value: 'writing', label: 'Writing' },
                    ]}
                  />
                </label>
                <fieldset>
                  <legend>How should we check your focus?</legend>
                  <div className="choice-grid two">
                    {[
                      [
                        'activity_only',
                        'Apps only',
                        'Count time in your chosen apps and websites.',
                      ],
                      [
                        'camera_and_activity',
                        'Apps + camera',
                        'Also check that you are at your desk.',
                      ],
                    ].map(([value, label, help]) => (
                      <label
                        className={`choice-card ${attention.verification === value ? 'selected' : ''}`}
                        key={value}
                      >
                        <input
                          type="radio"
                          name="verification"
                          value={value}
                          disabled={
                            value === 'camera_and_activity' &&
                            setup?.platform === 'windows'
                          }
                          checked={
                            (setup?.platform === 'windows'
                              ? 'activity_only'
                              : attention.verification) === value
                          }
                          onChange={() =>
                            updateAttention({
                              verification:
                                value as AttentionPolicy['verification'],
                            })
                          }
                        />
                        <span>
                          <strong>{label}</strong>
                          <small>
                            {value === 'camera_and_activity' &&
                            setup?.platform === 'windows'
                              ? 'Not available on Windows.'
                              : help}
                          </small>
                        </span>
                      </label>
                    ))}
                  </div>
                </fieldset>
                <fieldset>
                  <legend>How strict should the session be?</legend>
                  <div className="choice-grid two">
                    {[
                      [
                        'standard',
                        'Flexible',
                        'Pause or stop whenever you need.',
                      ],
                      [
                        'strict',
                        'Strict',
                        'Only chosen apps. Finish your target to stop.',
                      ],
                    ].map(([value, label, help]) => (
                      <label
                        key={value}
                        className={`choice-card ${focus.mode === value ? 'selected' : ''}`}
                      >
                        <input
                          type="radio"
                          name="mode"
                          checked={focus.mode === value}
                          onChange={() =>
                            update({
                              mode: value as FocusConfig['mode'],
                              apps: [],
                            })
                          }
                        />
                        <span>
                          <strong>{label}</strong>
                          <small>{help}</small>
                        </span>
                      </label>
                    ))}
                  </div>
                </fieldset>
                {strict && (
                  <p className="small muted">
                    Strict sessions cannot be paused or ended early in the app.
                    App blocking hides windows; it is not a device lock.
                  </p>
                )}
              </>
            )}
            {step === 1 && (
              <>
                <div className="section-heading">
                  <h2>Which apps will you use?</h2>
                  <button
                    type="button"
                    className="secondary"
                    disabled={loading}
                    onClick={() => void refresh()}
                  >
                    Refresh apps
                  </button>
                </div>
                <p className="small muted">
                  {strict || focus.activity_consent
                    ? 'Other apps will be hidden.'
                    : 'Time counts in the apps you choose.'}{' '}
                  If an app you want to use is missing, open it on your laptop,
                  then click “Refresh apps.”
                </p>
                <div className="application-picker">
                  {apps.map((app) => (
                    <label className="check-label" key={app.id}>
                      <input
                        type="checkbox"
                        checked={workApps.includes(app.id)}
                        onChange={(e) => {
                          const isBrowser = browsers.some(
                            (b) => b.id === app.id,
                          );
                          if (e.target.checked && isBrowser) {
                            // One browser per session: refuse a second one
                            // with a warning instead of silently adding it.
                            const other = workApps.find(
                              (id) =>
                                id !== app.id &&
                                browsers.some((b) => b.id === id),
                            );
                            if (other) {
                              const otherName =
                                browsers.find((b) => b.id === other)?.name ??
                                'another browser';
                              setBrowserWarn(
                                `Only one browser per session. Uncheck ${otherName} to use ${app.name} instead.`,
                              );
                              return;
                            }
                            setBrowserWarn('');
                          }
                          if (!e.target.checked && isBrowser) {
                            setBrowserWarn('');
                          }
                          setWorkApps(
                            e.target.checked
                              ? [...workApps, app.id]
                              : workApps.filter((id) => id !== app.id),
                          );
                          if (e.target.checked) {
                            update({
                              apps: focus.apps.filter((id) => id !== app.id),
                            });
                          }
                          if (e.target.checked && isBrowser) {
                            update({ browser_app: app.id });
                          }
                        }}
                      />
                      {app.name}
                    </label>
                  ))}
                  {!apps.length && (
                    <p className="muted small">
                      No apps found. Open a work app and refresh.
                    </p>
                  )}
                </div>
                <div className="setup-guide">
                  {browserWarn && (
                    <p role="alert" className="camera-warn">
                      {browserWarn}
                    </p>
                  )}
                  <div className="setting-row">
                    <span>Browser extension</span>
                    <div className="row">
                      <button
                        type="button"
                        className="icon-button"
                        aria-label="Re-check browser connection"
                        title="Re-check browser connection"
                        disabled={rechecking}
                        onClick={() => {
                          setRechecking(true);
                          void refresh().finally(() => {
                            setTimeout(() => setRechecking(false), 1500);
                          });
                        }}
                      >
                        <RefreshCw
                          size={16}
                          className={rechecking ? 'spin' : undefined}
                        />
                      </button>
                      {browserConnected ? (
                        <button
                          type="button"
                          className="connected-pill"
                          aria-disabled="true"
                          tabIndex={-1}
                        >
                          Connected
                        </button>
                      ) : (
                        <button
                          type="button"
                          className="secondary"
                          onClick={() => setGuide(!guide)}
                        >
                          {guide ? 'Hide setup' : 'Connect browser'}
                        </button>
                      )}
                    </div>
                  </div>
                  {guide && (
                    <BrowserSetup
                      setup={setup}
                      refresh={async () => {
                        await refresh();
                      }}
                      browserId={browserId}
                    />
                  )}
                  {!browsers.length && (
                    <p className="small muted">
                      Open Comet, Chrome, Edge, Brave or Vivaldi, then refresh
                      apps.
                    </p>
                  )}
                  <div className="blocking-section">
                    <p className="blocking-heading">Website blocking</p>
                    <label className="blocking-option">
                      <input
                        type="checkbox"
                        checked={blockMode === 'blocklist'}
                        onChange={() => setBlockMode('blocklist')}
                      />
                      <strong>Block distracting websites</strong>
                    </label>
                    {blockMode === 'blocklist' && (
                      <>
                        <textarea
                          rows={3}
                          className="blocking-sites"
                          aria-label="Websites to block"
                          value={blockSites}
                          onChange={(e) => {
                            setBlockSites(e.target.value);
                            setBlockSitesError(
                              domainInputError(e.target.value),
                            );
                          }}
                          onBlur={() => {
                            const next = normalizeDomainInput(blockSites);
                            setBlockSites(next.text);
                            setBlockSitesError(domainInputError(next.text));
                          }}
                          placeholder={
                            'youtube.com\ninstagram.com\nfacebook.com'
                          }
                        />
                        <p className="small muted">
                          One per line. These websites will be blocked during
                          the session.
                        </p>
                        {blockSitesError && (
                          <p className="error small" role="alert">
                            {blockSitesError}
                          </p>
                        )}
                      </>
                    )}
                    <label className="blocking-option">
                      <input
                        type="checkbox"
                        checked={blockMode === 'allowlist'}
                        onChange={() => setBlockMode('allowlist')}
                      />
                      <strong>Block all websites except these</strong>
                    </label>
                    {blockMode === 'allowlist' && (
                      <>
                        <textarea
                          rows={3}
                          className="blocking-sites"
                          aria-label="Websites to keep open"
                          value={blockSites}
                          onChange={(e) => {
                            setBlockSites(e.target.value);
                            setBlockSitesError(
                              domainInputError(e.target.value),
                            );
                          }}
                          onBlur={() => {
                            const next = normalizeDomainInput(blockSites);
                            setBlockSites(next.text);
                            setBlockSitesError(domainInputError(next.text));
                          }}
                          placeholder={
                            'github.com\nstackoverflow.com\ndocs.python.org'
                          }
                        />
                        <p className="small muted">
                          One per line. Only these websites stay open —
                          everything else is blocked.
                        </p>
                        {blockSitesError && (
                          <p className="error small" role="alert">
                            {blockSitesError}
                          </p>
                        )}
                      </>
                    )}
                  </div>
                  {blockMode !== 'none' && (
                    <div className="blocking-section">
                      <p className="blocking-heading">YouTube blocking</p>
                      <label className="blocking-option">
                        <input
                          type="checkbox"
                          checked={ytMode === 'block'}
                          onChange={() => setYtModeChoice('block')}
                        />
                        <strong>Block these channels</strong>
                      </label>
                      {ytMode === 'block' && (
                        <>
                          <textarea
                            rows={3}
                            className="blocking-sites"
                            aria-label="Channels to block"
                            value={ytBlocked}
                            onChange={(e) => {
                              setYtBlocked(e.target.value);
                              setYtBlockedError(
                                youTubeChannelInputError(e.target.value),
                              );
                            }}
                            onBlur={() => {
                              const next =
                                normalizeYouTubeChannelInput(ytBlocked);
                              setYtBlocked(next.text);
                              setYtBlockedError(
                                youTubeChannelInputError(next.text),
                              );
                            }}
                            placeholder={'@mkbhd\n@veritasium'}
                          />
                          <p className="small muted">
                            One per line. Videos from these channels will not
                            play.
                          </p>
                          {ytBlockedError && (
                            <p className="error small" role="alert">
                              {ytBlockedError}
                            </p>
                          )}
                        </>
                      )}
                      <label className="blocking-option">
                        <input
                          type="checkbox"
                          checked={ytMode === 'study'}
                          onChange={() => setYtModeChoice('study')}
                        />
                        <strong>Block all except these</strong>
                      </label>
                      {ytMode === 'study' && (
                        <>
                          <textarea
                            rows={3}
                            className="blocking-sites"
                            aria-label="Allowed channels"
                            value={ytAllowed}
                            onChange={(e) => {
                              setYtAllowed(e.target.value);
                              setYtAllowedError(
                                youTubeChannelInputError(e.target.value),
                              );
                            }}
                            onBlur={() => {
                              const next =
                                normalizeYouTubeChannelInput(ytAllowed);
                              setYtAllowed(next.text);
                              setYtAllowedError(
                                youTubeChannelInputError(next.text),
                              );
                            }}
                            placeholder={'@khanacademy\n@freecodecamp'}
                          />
                          <p className="small muted">
                            One per line. Only these channels will play.
                          </p>
                          {ytAllowedError && (
                            <p className="error small" role="alert">
                              {ytAllowedError}
                            </p>
                          )}
                        </>
                      )}
                      <label className="blocking-option">
                        <input
                          type="checkbox"
                          checked={ytWatchLater}
                          onChange={(e) => setYtWatchLater(e.target.checked)}
                        />
                        <strong>
                          Allow Watch Later playlist even when YouTube is
                          blocked
                        </strong>
                      </label>
                    </div>
                  )}
                </div>
              </>
            )}
            {step === 2 && (
              <>
                <h2>Ready to focus?</h2>
                <p className="session-summary">
                  {title} · {minutes} min · {strict ? 'Strict' : 'Flexible'}
                </p>
                {setup?.platform !== 'windows' &&
                  attention.verification === 'camera_and_activity' && (
                    <div className="setup-guide camera-status">
                      <div className="setting-row">
                        <strong>Camera</strong>
                        {setup?.camera_permission === 'granted' ? (
                          <span className="status-pill">Ready</span>
                        ) : (
                          <button
                            type="button"
                            onClick={() =>
                              void (async () => {
                                if (setup?.camera_permission === 'denied') {
                                  try {
                                    await api.openSetup('camera_settings');
                                    setProblem(
                                      'Turn on Preelock in System Settings → Privacy & Security → Camera, then come back here.',
                                    );
                                  } catch (e) {
                                    setProblem(String(e));
                                  }
                                } else {
                                  await cameraPermission();
                                }
                              })()
                            }
                          >
                            Allow camera
                          </button>
                        )}
                      </div>
                    </div>
                  )}
                <div className="setup-guide">
                  <label>
                    Wait before labeling a distraction
                    <Dropdown
                      value={String(attention.grace_seconds)}
                      onChange={(value) =>
                        updateAttention({
                          grace_seconds: Number(value),
                        })
                      }
                      options={[
                        { value: '3', label: '3 seconds' },
                        { value: '10', label: '10 seconds (recommended)' },
                        { value: '20', label: '20 seconds' },
                      ]}
                    />
                  </label>
                </div>
              </>
            )}
            {step === 2 && (
              <div className="setup-guide">
                <label className="check-label">
                  <input
                    type="checkbox"
                    checked={pomodoro.enabled}
                    onChange={(e) =>
                      updatePomodoro({ enabled: e.target.checked })
                    }
                  />
                  Use Pomodoro focus/break cycles
                </label>
                {pomodoro.enabled && (
                  <>
                    <p className="muted">
                      Focus in timed phases with real breaks in between.
                      Blocking relaxes during breaks, and break time never
                      counts toward your session target.
                    </p>
                    <div className="simple-grid">
                      <label>
                        Focus (minutes)
                        <input
                          type="number"
                          min={1}
                          max={180}
                          value={pomodoro.focus_minutes}
                          onChange={(e) =>
                            updatePomodoro({
                              focus_minutes: Number(e.target.value),
                            })
                          }
                        />
                      </label>
                      <label>
                        Short break (minutes)
                        <input
                          type="number"
                          min={1}
                          max={60}
                          value={pomodoro.short_break_minutes}
                          onChange={(e) =>
                            updatePomodoro({
                              short_break_minutes: Number(e.target.value),
                            })
                          }
                        />
                      </label>
                      <label>
                        Long break (minutes)
                        <input
                          type="number"
                          min={1}
                          max={90}
                          value={pomodoro.long_break_minutes}
                          onChange={(e) =>
                            updatePomodoro({
                              long_break_minutes: Number(e.target.value),
                            })
                          }
                        />
                      </label>
                      <label>
                        Focus cycles before a long break
                        <input
                          type="number"
                          min={2}
                          max={8}
                          value={pomodoro.cycles_before_long_break}
                          onChange={(e) =>
                            updatePomodoro({
                              cycles_before_long_break: Number(e.target.value),
                            })
                          }
                        />
                      </label>
                    </div>
                  </>
                )}
              </div>
            )}

            {step === 2 && (
              <label className="check-label block-apps">
                <input
                  type="checkbox"
                  checked={focus.activity_consent}
                  onChange={(e) =>
                    update({ activity_consent: e.target.checked })
                  }
                />
                Block all other apps during this session
              </label>
            )}
            {(visibleProblem || error) && (
              <p className="error notice" role="alert">
                {visibleProblem || error}
              </p>
            )}
            <div
              className="row controls"
              style={{ justifyContent: 'flex-end' }}
            >
              <button disabled={busy || !desktop}>
                {step === 2 && wizardMode === 'start'
                  ? 'Create session'
                  : 'Continue'}
              </button>
            </div>
          </form>
        </>
      ) : (
        <div className="focus-planning-grid">
          <div className="panel new-session-card focus-planning-card">
            <div className="section-heading section-start">
              <h2>Start a new focus session</h2>
            </div>
            <p className="small muted section-desc">
              Choose what matters, set your time, and give it your full
              attention.
            </p>
            <div className="panel">
              <Empty>No active session</Empty>
            </div>
            <div className="row controls">
              <button type="button" onClick={() => startWizard('start')}>
                <Plus size={16} />
                New session
              </button>
            </div>
          </div>
          <PlanningSections
            only="schedules"
            onNewSchedule={() => startWizard('schedule')}
          />
          <PlanningSections
            only="oneshots"
            onNewOneShot={() => startWizard('oneshot')}
          />
          <PlanningSections only="allowances" />
        </div>
      )}
      <Activity />
      {confirmOpen && (
        <SessionConfirmDialog
          title={title}
          minutes={minutes}
          strict={strict}
          busy={busy}
          onStart={() => void confirmStartSession()}
          onCancel={resetWizard}
        />
      )}
      {scheduleConfirmOpen && (
        <ScheduleConfirmDialog
          title={title}
          minutes={minutes}
          strict={strict}
          busy={busy}
          onCreate={(fields) => void confirmCreateSchedule(fields)}
          onCancel={() => setScheduleConfirmOpen(false)}
        />
      )}
      {oneShotConfirmOpen && (
        <OneShotConfirmDialog
          title={title}
          minutes={minutes}
          strict={strict}
          busy={busy}
          onCreate={(fields) => void confirmCreateOneShot(fields)}
          onCancel={() => setOneShotConfirmOpen(false)}
        />
      )}
    </>
  );
}
