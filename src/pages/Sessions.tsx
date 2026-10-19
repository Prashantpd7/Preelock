import { useState } from 'react';
import { ArrowLeft, Plus, Timer } from 'lucide-react';
import { useAppStore } from '../state/store';
import { api, desktop } from '../services/api';
import { setupProblem, splitDomains } from '../services/focusSetup';
import { PageHeading, Empty } from '../components/ui';
import { SessionCard } from '../components/SessionCard';
import { BrowserSetup } from '../components/BrowserSetup';
import { useFocusSetup } from '../hooks/useFocusSetup';
import {
  defaultFocus,
  type AttentionPolicy,
  type FocusConfig,
  type Session,
  type SavedSetup,
} from '../types';
const CAMERA_GUIDE =
  'Choose Allow in the macOS prompt, then click Check camera.';
export function Sessions() {
  const { data, execute, busy } = useAppStore();
  const { setup, error, loading, refresh } = useFocusSetup();
  const [step, setStep] = useState(0),
    [title, setTitle] = useState(''),
    [minutes, setMinutes] = useState(data.preferences.default_minutes);
  const [focus, setFocus] = useState<FocusConfig>(
    structuredClone(defaultFocus),
  );
  const [workApps, setWorkApps] = useState<string[]>([]),
    [sites, setSites] = useState(''),
    [blockedSites, setBlockedSites] = useState('');
  const [web, setWeb] = useState(false),
    [guide, setGuide] = useState(false),
    [problem, setProblem] = useState('');
  const [maxStep, setMaxStep] = useState(0);
  const [creating, setCreating] = useState(false);
  const [editingId, setEditingId] = useState<string | null>(null);
  function applySavedSetup(preset: SavedSetup) {
    const f = structuredClone(preset.focus);
    f.camera = false;
    f.activity_consent = false;
    setTitle(preset.name);
    setMinutes(preset.minutes);
    setFocus(f);
    setWorkApps(f.mode === 'strict' ? f.apps : (f.attention?.work_apps ?? []));
    setSites(
      (f.mode === 'strict'
        ? f.domains
        : (f.attention?.work_domains ?? [])
      ).join('\n'),
    );
    setBlockedSites((f.mode === 'strict' ? [] : f.domains).join('\n'));
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
      },
    });
    setWorkApps(f?.attention?.work_apps ? [...f.attention.work_apps] : []);
    if (strictMode) {
      setSites((f?.domains ?? []).join('\n'));
      setBlockedSites('');
    } else {
      setSites((f?.attention?.work_domains ?? []).join('\n'));
      setBlockedSites((f?.domains ?? []).join('\n'));
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
  const manual = attention.verification === 'manual_checkpoints',
    strict = focus.mode === 'strict';
  const browsers = setup?.browsers ?? [];
  const browserId = web ? focus.browser_app || browsers[0]?.id || '' : '';
  const selected: FocusConfig = {
    ...focus,
    apps: strict ? workApps : focus.apps,
    browser_app: browserId,
    general_internet: web ? focus.general_internet : true,
    domains: web ? splitDomains(strict ? sites : blockedSites) : [],
    camera: attention.verification === 'camera_and_activity',
    attention: {
      ...attention,
      work_apps: workApps,
      work_domains: web && !strict ? splitDomains(sites) : [],
    },
  };
  // App-only Strict sessions do not require an unused browser connection.
  const sessions = data.sessions
    .filter((s) =>
      ['ready', 'preparing', 'running', 'paused'].includes(s.status),
    )
    .slice()
    .reverse();
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
  async function checkCamera() {
    const s = await refresh();
    if (s?.camera_permission !== 'granted') {
      setProblem(
        s?.camera_permission === 'denied'
          ? 'Camera access is blocked. Turn on VYSTRIQ in System Settings → Privacy & Security → Camera, then click Check camera.'
          : CAMERA_GUIDE,
      );
    }
  }
  // The camera guide clears itself once permission is granted.
  const visibleProblem =
    problem === CAMERA_GUIDE && setup?.camera_permission === 'granted'
      ? ''
      : problem;
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
                (!manual || strict) &&
                selected.general_internet &&
                !sites.trim()
              ) {
                setProblem(
                  'Add at least one work website, or turn off websites.',
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
                  <div className="choice-grid">
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
                      [
                        'manual_checkpoints',
                        'Manual check-ins',
                        'You confirm your work once a minute.',
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
                          checked={attention.verification === value}
                          onChange={() =>
                            updateAttention({
                              verification:
                                value as AttentionPolicy['verification'],
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
                  {strict
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
                          if (
                            e.target.checked &&
                            browsers.some((b) => b.id === app.id)
                          ) {
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
                {manual && (
                  <p className="small muted">
                    App choices are optional for Flexible manual sessions.
                  </p>
                )}
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
                        onChange={(e) =>
                          update({ browser_app: e.target.value })
                        }
                      >
                        <option value="">Choose a browser</option>
                        {browsers.map((b) => (
                          <option key={b.id} value={b.id}>
                            {b.name}
                          </option>
                        ))}
                      </select>
                    </label>
                    <div className="setting-row">
                      <span>Browser extension</span>
                      {setup?.browser.connected ? (
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
                    <label>
                      Websites for this task
                      <textarea
                        rows={3}
                        value={sites}
                        onChange={(e) => setSites(e.target.value)}
                        placeholder="github.com\ndocs.rs"
                      />
                    </label>
                    <p className="small muted">
                      One website per line.
                      {strict
                        ? ' Other websites will be blocked.'
                        : ' Only these websites earn focus time.'}
                    </p>
                    <div className="row controls">
                      <button
                        type="button"
                        className="secondary"
                        onClick={() =>
                          setSites('github.com\ndocs.rs\ndeveloper.mozilla.org')
                        }
                      >
                        Coding examples
                      </button>
                      <button
                        type="button"
                        className="secondary"
                        onClick={() => setSites('coursera.org\nedx.org')}
                      >
                        Study examples
                      </button>
                    </div>
                    {!strict && (
                      <details>
                        <summary>Block distracting websites</summary>
                        <label>
                          Websites to block
                          <textarea
                            rows={3}
                            value={blockedSites}
                            onChange={(e) => setBlockedSites(e.target.value)}
                            placeholder="reddit.com\ninstagram.com"
                          />
                        </label>
                      </details>
                    )}
                  </div>
                )}
                {!strict && (
                  <details>
                    <summary>Block distracting apps (optional)</summary>
                    <div className="application-picker">
                      {apps
                        .filter(
                          (a) => !workApps.includes(a.id) && a.id !== browserId,
                        )
                        .map((app) => (
                          <label className="check-label" key={app.id}>
                            <input
                              type="checkbox"
                              aria-label={`Block ${app.name}`}
                              checked={focus.apps.includes(app.id)}
                              onChange={(e) =>
                                update({
                                  apps: e.target.checked
                                    ? [...focus.apps, app.id]
                                    : focus.apps.filter((id) => id !== app.id),
                                })
                              }
                            />
                            {app.name}
                          </label>
                        ))}
                    </div>
                  </details>
                )}
              </>
            )}
            {step === 2 && (
              <>
                <h2>Ready to focus?</h2>
                <p className="session-summary">
                  {title} · {minutes} min · {strict ? 'Strict' : 'Flexible'}
                </p>
                {attention.verification === 'camera_and_activity' && (
                  <div className="setup-guide camera-status">
                    <div className="setting-row">
                      <strong>Camera</strong>
                      {setup?.camera_permission === 'granted' ? (
                        <span className="status-pill">Ready</span>
                      ) : (
                        <span>Permission needed</span>
                      )}
                    </div>
                    {setup?.camera_permission !== 'granted' && (
                      <div className="row controls">
                        {setup?.camera_permission === 'denied' ? (
                          <button
                            type="button"
                            onClick={() =>
                              void (async () => {
                                try {
                                  await api.openSetup('camera_settings');
                                  setProblem(
                                    'Turn on VYSTRIQ in System Settings → Privacy & Security → Camera, then click Check camera.',
                                  );
                                } catch (e) {
                                  setProblem(String(e));
                                }
                              })()
                            }
                          >
                            Open camera settings
                          </button>
                        ) : (
                          <button
                            type="button"
                            onClick={() => void cameraPermission()}
                          >
                            Allow camera
                          </button>
                        )}
                        <button
                          type="button"
                          className="secondary"
                          onClick={() => void checkCamera()}
                        >
                          Check camera
                        </button>
                      </div>
                    )}
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
                  {attention.verification === 'camera_and_activity' && (
                    <label>
                      If the camera stops working
                      <select
                        value={attention.camera_fallback}
                        onChange={(e) =>
                          updateAttention({
                            camera_fallback: e.target
                              .value as AttentionPolicy['camera_fallback'],
                          })
                        }
                      >
                        <option value="unknown">Pause the timer</option>
                        <option value="activity_only">
                          Continue using app activity
                        </option>
                      </select>
                    </label>
                  )}
                </div>
              </>
            )}
            {step === 2 && (
              <label className="check-line">
                <input
                  type="checkbox"
                  checked={focus.activity_consent}
                  onChange={(e) =>
                    update({ activity_consent: e.target.checked })
                  }
                />
                Allow app activity for this session
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
