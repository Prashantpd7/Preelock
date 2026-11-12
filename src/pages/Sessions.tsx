import { useEffect, useRef, useState } from 'react';
import {
  ArrowLeft,
  CalendarDays,
  Clock,
  Hourglass,
  Plus,
  RefreshCw,
  Timer,
} from 'lucide-react';
import { useAppStore } from '../state/store';
import { api, desktop } from '../services/api';
import { setupProblem, splitDomains } from '../services/focusSetup';
import { PageHeading } from '../components/ui';
import {
  defaultActivitySection,
  type ActivitySection,
} from '../services/activitySection';
import { Dropdown } from '../components/Dropdown';
import { BrowserSetup } from '../components/BrowserSetup';
import { SafariBrowserNotice } from '../components/SafariBrowserNotice';
import { isSafariId, sessionNeedsExtension } from '../services/browserSupport';
import { SessionConfirmDialog } from '../components/SessionConfirmDialog';
import { ScheduleConfirmDialog } from '../components/ScheduleConfirmDialog';
import type { ScheduleCreateFields } from '../components/ScheduleConfirmDialog';
import { OneShotConfirmDialog } from '../components/OneShotConfirmDialog';
import { AllowanceForm } from '../components/AllowanceForm';
import { Activity } from './Activity';
import {
  cleanDomain,
  cleanYouTubeChannel,
  domainInputError,
  isYouTubeBlockedByWebsiteRules,
  normalizeDomainInput,
  normalizeYouTubeChannelInput,
  youTubeChannelInputError,
} from '../services/domains';
import { useFocusSetup } from '../hooks/useFocusSetup';
import {
  defaultFocus,
  sameBrowser,
  type Allowance,
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
  const [websiteEnabled, setWebsiteEnabled] = useState(false);
  const [websiteSubMode, setWebsiteSubMode] = useState<
    'blocklist' | 'allowlist' | null
  >(null);
  const [ytEnabled, setYtEnabled] = useState(false);
  const [ytBlockChannels, setYtBlockChannels] = useState(false);
  const [ytBlockEntertainment, setYtBlockEntertainment] = useState(false);
  const [ytWatchLater, setYtWatchLater] = useState(false);
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
  // Activity dropdown selection, owned here so the cards and the
  // dropdown stay in sync. The smart default is applied exactly once,
  // when the snapshot first loads; after that the user's own choice
  // sticks across re-renders and data changes.
  const storeLoading = useAppStore((s) => s.loading);
  const [section, setSection] = useState<ActivitySection | null>(null);
  const [allowanceOpen, setAllowanceOpen] = useState(false);
  const [editingAllowance, setEditingAllowance] = useState<Allowance | null>(
    null,
  );
  const defaultSectionApplied = useRef(false);
  useEffect(() => {
    if (defaultSectionApplied.current || storeLoading) return;
    defaultSectionApplied.current = true;
    setSection(defaultActivitySection(data));
  }, [storeLoading, data]);
  function startNewAllowance() {
    setCreating(false);
    setEditingAllowance(null);
    setAllowanceOpen(true);
  }
  function startEditAllowance(allowance: Allowance) {
    setCreating(false);
    setEditingAllowance(allowance);
    setAllowanceOpen(true);
  }
  function closeAllowanceForm() {
    setAllowanceOpen(false);
    setEditingAllowance(null);
  }
  function startWizard(mode: 'start' | 'schedule' | 'oneshot') {
    closeAllowanceForm();
    setProblem('');
    setSafariNotice(false);
    setMinutes(data.preferences.default_minutes);
    setWebsiteEnabled(false);
    setWebsiteSubMode(null);
    setBlockSites('');
    setBlockSitesError('');
    setYtEnabled(false);
    setYtBlockChannels(false);
    setYtBlockEntertainment(false);
    setYtBlocked('');
    setYtBlockedError('');
    setYtWatchLater(false);
    setStep(0);
    setMaxStep(0);
    setWizardMode(mode);
    setCreating(true);
  }
  function resetWizard() {
    setSafariNotice(false);
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
  // True while a browser chosen in the compatibility notice is being
  // opened and waited for.
  const [browserOpening, setBrowserOpening] = useState(false);
  const [maxStep, setMaxStep] = useState(0);
  const [creating, setCreating] = useState(false);
  const [confirmOpen, setConfirmOpen] = useState(false);
  // Session-only Safari compatibility notice (Safari has no Preelock
  // extension, so it cannot enforce website rules during a session).
  const [safariNotice, setSafariNotice] = useState(false);
  function applySavedSetup(preset: SavedSetup) {
    const f = structuredClone(preset.focus);
    // Saved before Manual check-ins was removed: use Apps only instead.
    if (
      f.attention &&
      (f.attention.verification as string) === 'manual_checkpoints'
    )
      f.attention.verification = 'activity_only';
    f.camera = false;
    // Other apps are always blocked now: the wizard no longer offers
    // the choice, and `selected` forces activity_consent on too, so a
    // saved setup can never carry a false value into a session.
    f.activity_consent = true;
    // Same for the distraction grace: the wizard no longer offers the
    // choice, so a saved setup can never carry a different value in.
    if (f.attention) f.attention.grace_seconds = 10;
    setTitle(preset.name);
    setMinutes(preset.minutes);
    setFocus(f);
    setWorkApps(f.mode === 'strict' ? f.apps : (f.attention?.work_apps ?? []));
    const savedDomains = f.domains ?? [];
    setBlockSites(savedDomains.join('\n'));
    setBlockSitesError('');
    setYtBlocked((f.youtube_blocked_channels ?? []).join('\n'));
    setYtBlockedError('');
    const hasBlockedChannels = (f.youtube_blocked_channels?.length ?? 0) > 0;
    const hasEntertainment = !!f.youtube_block_entertainment;
    setYtBlockChannels(hasBlockedChannels);
    setYtBlockEntertainment(hasEntertainment);
    setYtEnabled(hasBlockedChannels || hasEntertainment);
    setYtWatchLater(!!f.youtube_allow_watch_later);
    const wb = f.website_blocking ?? 'auto';
    if (wb === 'blocklist' || wb === 'allowlist') {
      setWebsiteEnabled(true);
      setWebsiteSubMode(wb);
    } else if (wb === 'auto' && savedDomains.length > 0) {
      setWebsiteEnabled(true);
      setWebsiteSubMode(f.mode === 'strict' ? 'allowlist' : 'blocklist');
    } else {
      setWebsiteEnabled(false);
      setWebsiteSubMode(null);
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
  // Website blocking is unchecked by default; checking it reveals the two
  // mutually-exclusive sub-options ('blocklist' / 'allowlist').
  const blockMode: 'none' | 'blocklist' | 'allowlist' = websiteEnabled
    ? (websiteSubMode ?? 'none')
    : 'none';
  const chooseWebsiteSubMode = (mode: 'blocklist' | 'allowlist') =>
    setWebsiteSubMode(websiteSubMode === mode ? null : mode);
  const watchLaterAvailable = isYouTubeBlockedByWebsiteRules(
    blockMode,
    blockSites,
  );
  const activeYtBlockChannels = ytEnabled && ytBlockChannels;
  const activeYtEntertainment = ytEnabled && ytBlockEntertainment;
  const { setup, error, loading, refresh } = useFocusSetup(
    focus.browser_app || undefined,
  );
  const browsers = setup?.browsers ?? [];
  const needsBrowser = websiteEnabled || ytEnabled || !focus.general_internet;
  const connectedBrowser = setup?.connected_browsers?.[0];
  const preferredBrowserId =
    connectedBrowser &&
    browsers.some((b) => sameBrowser(b.id, connectedBrowser))
      ? (browsers.find((b) => sameBrowser(b.id, connectedBrowser))?.id ??
        connectedBrowser)
      : browsers[0]?.id || '';
  const browserId =
    focus.browser_app || (needsBrowser ? preferredBrowserId : '');
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
    // The "Block all other apps" choice is gone from the wizard: other
    // apps are always blocked, so force consent on (a saved setup
    // could still carry it off).
    activity_consent: true,
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
    // YouTube channel and entertainment rules are controlled by the
    // independent YouTube blocking section; Watch Later is controlled by
    // whether Website blocking currently blocks YouTube.
    youtube_blocked_channels: activeYtBlockChannels
      ? splitDomains(ytBlocked).flatMap((entry) => {
          const cleaned = cleanYouTubeChannel(entry);
          return cleaned ? [cleaned] : [];
        })
      : [],
    youtube_allowed_channels: [],
    youtube_study_mode: false,
    youtube_block_entertainment: activeYtEntertainment,
    youtube_allow_watch_later: watchLaterAvailable && ytWatchLater,
    website_blocking: blockMode,
    camera:
      setup?.platform !== 'windows' &&
      attention.verification === 'camera_and_activity',
    attention: {
      ...attention,
      // The "Wait before labeling a distraction" choice is gone from
      // the wizard: always use the old default of 10 seconds (a saved
      // setup could still carry a different value).
      grace_seconds: 10,
      verification:
        setup?.platform === 'windows'
          ? 'activity_only'
          : attention.verification,
      work_apps: workApps,
      work_domains: [],
    },
  };
  // Safari has no Preelock extension, so it can never enforce a session's
  // website rules. The compatibility notice appears only while configuring
  // such a session — never during normal use or screen-time tracking, and
  // never for a session that blocks no websites.
  const safariSelected =
    workApps.some(isSafariId) ||
    focus.apps.some(isSafariId) ||
    isSafariId(focus.browser_app);
  const extensionNeeded = sessionNeedsExtension(selected);
  useEffect(() => {
    if (creating && safariSelected && extensionNeeded) setSafariNotice(true);
  }, [creating, safariSelected, extensionNeeded]);
  // Choose a supported browser from the notice: Safari is removed, every
  // other selected app is kept, and the browser is selected for this
  // session. One browser per session, so switching browsers replaces the
  // previous browser selection instead of adding a second one.
  function chooseSupportedBrowser(id: string) {
    setSafariNotice(false);
    setBrowserWarn('');
    const replacesBrowser = (x: string) =>
      x === id || browsers.some((b) => b.id === x);
    setWorkApps((ids) => [
      ...ids.filter((x) => !isSafariId(x) && !replacesBrowser(x)),
      id,
    ]);
    update({
      browser_app: id,
      apps: focus.apps.filter((x) => !isSafariId(x) && !replacesBrowser(x)),
    });
    const connected = (setup?.connected_browsers ?? []).some((c) =>
      sameBrowser(c, id),
    );
    setGuide(!connected);
    // The browser may not be running yet, so it would not be in the app
    // list. Open it (the desktop launches the installed app by its own
    // identity) and wait briefly for it to appear. Launching says nothing
    // about the extension: an unconnected browser still gets the existing
    // Connect browser guide above.
    void openChosenBrowser(id);
  }
  async function openChosenBrowser(id: string) {
    setBrowserOpening(true);
    setBrowserWarn('');
    try {
      await api.openSetup('browser', id);
    } catch {
      setBrowserOpening(false);
      setBrowserWarn(
        "Couldn't open browser. Open the browser manually and refresh the app list.",
      );
      return;
    }
    try {
      // Six polls, half a second apart: enough for a cold browser start,
      // bounded so a browser that never appears cannot spin here.
      for (let attempt = 0; attempt < 6; attempt++) {
        const next = await refresh(id);
        if (next?.applications.some((app) => app.id === id)) return;
        if (attempt < 5)
          await new Promise((resolve) => setTimeout(resolve, 500));
      }
    } finally {
      setBrowserOpening(false);
    }
  }
  function cancelSafariSelection() {
    setSafariNotice(false);
    setWorkApps((ids) => ids.filter((x) => !isSafariId(x)));
    update({ apps: focus.apps.filter((x) => !isSafariId(x)) });
  }
  const apps =
    setup?.applications.filter(
      (a) =>
        !/(preelock|finder|activitymonitor|systempreferences|explorer\.exe|taskmgr\.exe|systemsettings\.exe)/i.test(
          a.id,
        ),
    ) ?? [];
  // A selected application must always be visible so it can be unchecked,
  // even when application discovery has not listed it yet (a browser that
  // was just opened, or one this machine reports as installed only).
  const knownNames = new Map<string, string>(
    [...(setup?.applications ?? []), ...(setup?.browsers ?? [])].map(
      (a) => [a.id, a.name] as [string, string],
    ),
  );
  const appChoices = [
    ...apps,
    ...workApps
      .filter((id) => !apps.some((a) => a.id === id))
      .map((id) => ({ id, name: knownNames.get(id) ?? id })),
  ];
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
      {!creating &&
        !allowanceOpen &&
        !!data.preferences.saved_setups?.length && (
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
              if (websiteEnabled && !websiteSubMode) {
                setProblem(
                  'Choose a website blocking option below, or uncheck Website blocking.',
                );
                return;
              }
              if (
                selected.general_internet &&
                blockMode !== 'none' &&
                !blockSites.trim()
              ) {
                setProblem(
                  'Add at least one website below, or turn off website blocking.',
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
              if (ytEnabled && !ytBlockChannels && !ytBlockEntertainment) {
                setProblem(
                  'Choose a YouTube blocking option below, or uncheck YouTube blocking.',
                );
                return;
              }
              if (activeYtBlockChannels && !ytBlocked.trim()) {
                setProblem(
                  'Add at least one YouTube channel below, or uncheck Block these channels.',
                );
                return;
              }
              // Guards on the channel lists actually submitted.
              const badBlocked = activeYtBlockChannels
                ? splitDomains(ytBlocked).find(
                    (entry) => cleanYouTubeChannel(entry) === null,
                  )
                : undefined;
              if (badBlocked) {
                setProblem(`Invalid YouTube channel: ${badBlocked}`);
                return;
              }
              // Safari can never enforce this session's website rules:
              // keep the user in the wizard with the compatibility notice
              // instead of a generic problem message.
              if (safariSelected && extensionNeeded) {
                setSafariNotice(true);
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
                  {appChoices.map((app) => (
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
                          } else if (
                            !e.target.checked &&
                            isBrowser &&
                            focus.browser_app === app.id
                          ) {
                            update({ browser_app: '' });
                          }
                        }}
                      />
                      {app.name}
                    </label>
                  ))}
                  {!appChoices.length && (
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
                  {browserOpening && (
                    <p className="small muted" role="status">
                      Opening the browser…
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
                  {guide && !browserConnected && (
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
                    <label className="blocking-option">
                      <input
                        type="checkbox"
                        checked={websiteEnabled}
                        onChange={(e) => setWebsiteEnabled(e.target.checked)}
                      />
                      <strong>Website blocking</strong>
                    </label>
                    {websiteEnabled && (
                      <div className="blocking-suboptions">
                        <label className="blocking-option">
                          <input
                            type="checkbox"
                            checked={websiteSubMode === 'blocklist'}
                            onChange={() => chooseWebsiteSubMode('blocklist')}
                          />
                          <strong>Block distracting websites</strong>
                        </label>
                        {websiteSubMode === 'blocklist' && (
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
                              One per line. These websites will be blocked
                              during the session.
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
                            checked={websiteSubMode === 'allowlist'}
                            onChange={() => chooseWebsiteSubMode('allowlist')}
                          />
                          <strong>Block all websites except these</strong>
                        </label>
                        {websiteSubMode === 'allowlist' && (
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
                        {watchLaterAvailable && (
                          <label className="blocking-option">
                            <input
                              type="checkbox"
                              checked={ytWatchLater}
                              onChange={(e) =>
                                setYtWatchLater(e.target.checked)
                              }
                            />
                            <strong>
                              Allow Watch Later playlist when YouTube is blocked
                            </strong>
                          </label>
                        )}
                      </div>
                    )}
                  </div>
                  <div className="blocking-section">
                    <label className="blocking-option">
                      <input
                        type="checkbox"
                        checked={ytEnabled}
                        onChange={(e) => setYtEnabled(e.target.checked)}
                      />
                      <strong>YouTube blocking</strong>
                    </label>
                    {ytEnabled && (
                      <div className="blocking-suboptions">
                        <label className="blocking-option">
                          <input
                            type="checkbox"
                            checked={ytBlockChannels}
                            onChange={(e) =>
                              setYtBlockChannels(e.target.checked)
                            }
                          />
                          <strong>Block these channels</strong>
                        </label>
                        {ytBlockChannels && (
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
                            checked={ytBlockEntertainment}
                            onChange={(e) =>
                              setYtBlockEntertainment(e.target.checked)
                            }
                          />
                          <strong>Block all entertainment content</strong>
                        </label>
                        {ytBlockEntertainment && (
                          <p className="small muted">
                            Blocks comedy, pranks, vlogs, funny clips, animal
                            vlogs, gaming, music, and Shorts on YouTube.
                          </p>
                        )}
                      </div>
                    )}
                  </div>
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
      ) : allowanceOpen ? (
        <>
          <div className="page-back">
            <button
              type="button"
              className="secondary"
              onClick={closeAllowanceForm}
            >
              <ArrowLeft size={16} />
              Back
            </button>
          </div>
          <AllowanceForm
            key={editingAllowance?.id ?? 'new'}
            editing={editingAllowance}
            onSaved={() => {
              closeAllowanceForm();
              setSection('allowances');
            }}
            onCancel={closeAllowanceForm}
          />
        </>
      ) : (
        <div className="focus-planning-grid">
          <div className="panel focus-planning-card">
            <span className="planning-card-icon" aria-hidden="true">
              <Timer size={20} />
            </span>
            <div className="section-heading section-start">
              <h2>Start instant focus session</h2>
            </div>
            <p className="small muted section-desc">
              Choose what matters, set your time, and give it your full
              attention.
            </p>
            <div className="row controls">
              <button type="button" onClick={() => startWizard('start')}>
                <Plus size={16} />
                New session
              </button>
            </div>
          </div>
          <div className="panel focus-planning-card">
            <span className="planning-card-icon" aria-hidden="true">
              <CalendarDays size={20} />
            </span>
            <div className="section-heading section-start">
              <h2>Schedules</h2>
            </div>
            <p className="small muted section-desc">
              Automatic blocks on your weekly schedule. Needs open at login to
              run.
            </p>
            <div className="row controls">
              <button type="button" onClick={() => startWizard('schedule')}>
                <Plus size={16} />
                New schedule
              </button>
            </div>
          </div>
          <div className="panel focus-planning-card">
            <span className="planning-card-icon" aria-hidden="true">
              <Clock size={20} />
            </span>
            <div className="section-heading section-start">
              <h2>Start once at</h2>
            </div>
            <p className="small muted section-desc">
              One block at a chosen time.
            </p>
            <div className="row controls">
              <button type="button" onClick={() => startWizard('oneshot')}>
                <Plus size={16} />
                New one-shot
              </button>
            </div>
          </div>
          <div className="panel focus-planning-card">
            <span className="planning-card-icon" aria-hidden="true">
              <Hourglass size={20} />
            </span>
            <div className="section-heading section-start">
              <h2>Allowances</h2>
            </div>
            <p className="small muted section-desc">
              Daily or weekly time allowances. Blocks always lift on reset.
            </p>
            <div className="row controls">
              <button type="button" onClick={startNewAllowance}>
                <Plus size={16} />
                New allowance
              </button>
            </div>
          </div>
        </div>
      )}
      <Activity
        section={section ?? 'sessions'}
        onSectionChange={setSection}
        onNewSchedule={() => startWizard('schedule')}
        onNewOneShot={() => startWizard('oneshot')}
        onNewAllowance={startNewAllowance}
        onEditAllowance={startEditAllowance}
      />
      {safariNotice && creating && (
        <SafariBrowserNotice
          setup={setup}
          onChoose={chooseSupportedBrowser}
          onCancel={cancelSafariSelection}
          onRefresh={async () => {
            await refresh();
          }}
        />
      )}
      {confirmOpen && (
        <SessionConfirmDialog
          title={title}
          minutes={minutes}
          strict={strict}
          busy={busy}
          focus={selected}
          appNames={Object.fromEntries(knownNames)}
          browserName={
            browsers.find((b) => sameBrowser(b.id, browserId))?.name ?? ''
          }
          browserConnected={browserConnected}
          cameraPermission={setup?.camera_permission}
          platform={setup?.platform}
          safariWarning={safariSelected && extensionNeeded}
          onStart={() => void confirmStartSession()}
          onCancel={resetWizard}
          onConnectBrowser={() => {
            setConfirmOpen(false);
            setGuide(true);
          }}
        />
      )}
      {scheduleConfirmOpen && (
        <ScheduleConfirmDialog
          title={title}
          minutes={minutes}
          strict={strict}
          busy={busy}
          focus={selected}
          appNames={Object.fromEntries(knownNames)}
          browserName={
            browsers.find((b) => sameBrowser(b.id, browserId))?.name ?? ''
          }
          browserConnected={browserConnected}
          cameraPermission={setup?.camera_permission}
          platform={setup?.platform}
          safariWarning={safariSelected && extensionNeeded}
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
          focus={selected}
          appNames={Object.fromEntries(knownNames)}
          browserName={
            browsers.find((b) => sameBrowser(b.id, browserId))?.name ?? ''
          }
          browserConnected={browserConnected}
          cameraPermission={setup?.camera_permission}
          platform={setup?.platform}
          safariWarning={safariSelected && extensionNeeded}
          onCreate={(fields) => void confirmCreateOneShot(fields)}
          onCancel={() => setOneShotConfirmOpen(false)}
        />
      )}
    </>
  );
}
