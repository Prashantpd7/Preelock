import { useState } from 'react';
import { ArrowLeft, Plus, RefreshCw, Timer } from 'lucide-react';
import { useAppStore } from '../state/store';
import { api, desktop } from '../services/api';
import { setupProblem, splitDomains } from '../services/focusSetup';
import { PageHeading, Empty } from '../components/ui';
import { SessionCard } from '../components/SessionCard';
import { BrowserSetup } from '../components/BrowserSetup';
import { domainInputError, normalizeDomainInput } from '../services/domains';
import { useFocusSetup } from '../hooks/useFocusSetup';
import {
  defaultFocus,
  sameBrowser,
  type AttentionPolicy,
  type FocusConfig,
  type Session,
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
  const [blockModeOverride, setBlockModeOverride] = useState<
    'none' | 'blocklist' | 'allowlist' | null
  >(null);
  const [web, setWeb] = useState(false),
    [guide, setGuide] = useState(false),
    [rechecking, setRechecking] = useState(false),
    [problem, setProblem] = useState('');
  // One browser per session: shown when the user tries to pick a second.
  const [browserWarn, setBrowserWarn] = useState('');
  const [maxStep, setMaxStep] = useState(0);
  const [creating, setCreating] = useState(false);
  const [editingId, setEditingId] = useState<string | null>(null);
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
    if (savedDomains.length) {
      const wb = f.website_blocking ?? 'auto';
      setBlockModeOverride(
        wb === 'auto' ? (f.mode === 'strict' ? 'allowlist' : 'blocklist') : wb,
      );
    } else {
      setBlockModeOverride(f.mode === 'strict' ? 'allowlist' : null);
    }
    setWeb(
      !!f.browser_app ||
        f.domains.length > 0 ||
        !!f.attention?.work_domains.length,
    );
    setStep(0);
    setMaxStep(0);
    setEditingId(null);
    setProblem('');
    setCreating(true);
  }
  function startEdit(s: Session) {
    const f = s.focus;
    const strictMode = f?.mode === 'strict';
    setTitle(s.title);
    setMinutes(s.duration_minutes);
    setFocus({
      ...structuredClone(defaultFocus),
      ...f,
      attention: {
        ...(structuredClone(defaultFocus).attention as AttentionPolicy),
        ...f?.attention,
        // Sessions saved before Manual check-ins was removed: Apps only.
        verification:
          (f?.attention?.verification as string) === 'manual_checkpoints'
            ? 'activity_only'
            : (f?.attention?.verification ?? 'activity_only'),
      },
    });
    setWorkApps(f?.attention?.work_apps ? [...f.attention.work_apps] : []);
    const editDomains = f?.domains ?? [];
    setBlockSites(editDomains.join('\n'));
    setBlockSitesError('');
    if (editDomains.length) {
      const wb = f?.website_blocking ?? 'auto';
      setBlockModeOverride(
        wb === 'auto' ? (strictMode ? 'allowlist' : 'blocklist') : wb,
      );
    } else {
      setBlockModeOverride(strictMode ? 'allowlist' : null);
    }
    setWeb(
      !!f?.browser_app ||
        (f?.domains?.length ?? 0) > 0 ||
        (f?.attention?.work_domains?.length ?? 0) > 0,
    );
    setGuide(false);
    setProblem('');
    setStep(0);
    setMaxStep(0);
    setEditingId(s.id);
    setCreating(true);
    window.scrollTo({ top: 0, behavior: 'smooth' });
  }
  const attention = focus.attention!;
  const update = (patch: Partial<FocusConfig>) =>
    setFocus((c) => ({ ...c, ...patch }));
  const updateAttention = (patch: Partial<AttentionPolicy>) =>
    setFocus((c) => ({ ...c, attention: { ...c.attention!, ...patch } }));
  const strict = focus.mode === 'strict';
  // Website blocking choice. Defaults follow the session style unless the
  // user picks an option explicitly.
  const blockMode = blockModeOverride ?? (strict ? 'allowlist' : 'none');
  const setBlockMode = (mode: 'none' | 'blocklist' | 'allowlist') =>
    setBlockModeOverride(blockMode === mode ? 'none' : mode);
  const { setup, error, loading, refresh } = useFocusSetup(
    web && focus.browser_app ? focus.browser_app : undefined,
  );
  const browsers = setup?.browsers ?? [];
  const browserId = web ? focus.browser_app || browsers[0]?.id || '' : '';
  // "Connected" counts only when the paired extension reports that it
  // lives in the browser picked above. A connection from another browser —
  // or one that cannot say which browser it is in — does not count.
  const browserConnected =
    !!setup?.browser.connected &&
    !!setup?.browser.client_browser &&
    (!browserId || sameBrowser(browserId, setup.browser.client_browser));
  const selected: FocusConfig = {
    ...focus,
    apps: strict ? workApps : focus.apps,
    browser_app: browserId,
    general_internet: web ? focus.general_internet : true,
    domains: web && blockMode !== 'none' ? splitDomains(blockSites) : [],
    website_blocking: web ? blockMode : 'none',
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
  // App-only Strict sessions do not require an unused browser connection.
  const isLiveStatus = (s: (typeof data.sessions)[number]) =>
    ['running', 'paused', 'preparing'].includes(s.status);
  // Every session stays as a reusable card; live ones sort first.
  const sessions = data.sessions
    .slice()
    .reverse()
    .sort((a, b) => Number(isLiveStatus(b)) - Number(isLiveStatus(a)));
  const apps =
    setup?.applications.filter(
      (a) =>
        !/(vystriq|finder|activitymonitor|systempreferences|explorer\.exe|taskmgr\.exe|systemsettings\.exe)/i.test(
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
                  setEditingId(null);
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
              if (
                web &&
                selected.general_internet &&
                blockMode !== 'none' &&
                !blockSites.trim()
              ) {
                setProblem(
                  'Add at least one website above, or turn off "I need websites for this task".',
                );
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
              if (
                await execute({
                  type: 'create_session',
                  title,
                  minutes,
                  allow_pause: !strict,
                  focus: selected,
                })
              ) {
                setTitle('');
                setStep(0);
                setMaxStep(0);
                setCreating(false);
                if (editingId) {
                  setEditingId(null);
                  await execute({ type: 'end_session', id: editingId });
                }
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
                  <select
                    value={attention.task}
                    onChange={(e) => updateAttention({ task: e.target.value })}
                  >
                    <option value="other">General work</option>
                    <option value="coding">Coding</option>
                    <option value="reading">Reading / study</option>
                    <option value="lecture">Watching a lesson</option>
                    <option value="writing">Writing</option>
                  </select>
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
                    Emergency exit is always available. App blocking hides
                    windows; it is not a device lock.
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
                  Open a missing app, then refresh.
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
                            setWeb(true);
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
                <label className="check-label consent">
                  <input
                    type="checkbox"
                    checked={web}
                    onChange={(e) => {
                      setWeb(e.target.checked);
                      if (!e.target.checked)
                        setWorkApps(
                          workApps.filter(
                            (id) => !browsers.some((b) => b.id === id),
                          ),
                        );
                    }}
                  />
                  I need websites for this task
                </label>
                {web && (
                  <div className="setup-guide">
                    <label>
                      Which browser?
                      <select
                        value={browserId}
                        onChange={(e) => {
                          const picked = e.target.value;
                          const other = workApps.find(
                            (id) =>
                              id !== picked &&
                              browsers.some((b) => b.id === id),
                          );
                          if (picked && other) {
                            const otherName =
                              browsers.find((b) => b.id === other)?.name ??
                              'another browser';
                            const pickedName =
                              browsers.find((b) => b.id === picked)?.name ??
                              'this browser';
                            setBrowserWarn(
                              `Only one browser per session. Uncheck ${otherName} to use ${pickedName} instead.`,
                            );
                            return;
                          }
                          setBrowserWarn('');
                          update({ browser_app: picked });
                        }}
                      >
                        <option value="">Choose a browser</option>
                        {browsers.map((b) => (
                          <option key={b.id} value={b.id}>
                            {b.name}
                          </option>
                        ))}
                      </select>
                    </label>
                    {browserWarn && (
                      <p role="alert" className="camera-warn">
                        {browserWarn}
                      </p>
                    )}
                    <div className="setting-row divider-top">
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
                            placeholder="youtube.com"
                          />
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
                            placeholder="github.com"
                          />
                          {blockSitesError && (
                            <p className="error small" role="alert">
                              {blockSitesError}
                            </p>
                          )}
                        </>
                      )}
                    </div>
                  </div>
                )}
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
                                      'Turn on VYSTRIQ in System Settings → Privacy & Security → Camera, then come back here.',
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
                    <select
                      value={attention.grace_seconds}
                      onChange={(e) =>
                        updateAttention({
                          grace_seconds: Number(e.target.value),
                        })
                      }
                    >
                      <option value={3}>3 seconds</option>
                      <option value={10}>10 seconds (recommended)</option>
                      <option value={20}>20 seconds</option>
                    </select>
                  </label>
                </div>
              </>
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
                {step === 2
                  ? editingId
                    ? 'Save changes'
                    : 'Create session'
                  : 'Continue'}
              </button>
            </div>
          </form>
        </>
      ) : (
        <div className="panel new-session-card">
          <span className="focus-orbit">
            <Timer size={30} />
          </span>
          <h2>Start a new focus session</h2>
          <p className="muted">
            Choose what matters, set your time,
            <br />
            and give it your full attention.
          </p>
          <button
            type="button"
            onClick={() => {
              setProblem('');
              setEditingId(null);
              setMinutes(data.preferences.default_minutes);
              setStep(0);
              setMaxStep(0);
              setCreating(true);
            }}
          >
            <Plus size={16} />
            New session
          </button>
        </div>
      )}
      <div className="section-heading">
        <h2>Your sessions</h2>
      </div>
      <div className="session-list">
        {sessions.length ? (
          sessions.map((s) => (
            <SessionCard
              session={s}
              key={s.id}
              onEdit={startEdit}
              appNames={Object.fromEntries(
                (setup?.applications ?? []).map((a) => [a.id, a.name]),
              )}
              setup={setup}
              onSetupRefresh={refresh}
            />
          ))
        ) : (
          <div className="panel">
            <Empty>No session yet.</Empty>
          </div>
        )}
      </div>
    </>
  );
}
