import { useState } from 'react';
import { useAppStore } from '../state/store';
import { api, desktop } from '../services/api';
import { setupProblem, splitDomains } from '../services/focusSetup';
import { PageHeading, Empty } from '../components/ui';
import { SessionCard } from '../components/SessionCard';
import { BrowserSetup } from '../components/BrowserSetup';
import { useFocusSetup } from '../hooks/useFocusSetup';
import { defaultFocus, type AttentionPolicy, type FocusConfig } from '../types';
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
    [problem, setProblem] = useState(''),
    [pause, setPause] = useState(true);
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
    camera: attention.verification === 'camera_and_activity' && focus.camera,
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
      await refresh();
      setProblem('Choose Allow in the macOS prompt, then click Check camera.');
    } catch (e) {
      setProblem(String(e));
    }
  }
  return (
    <>
      <PageHeading
        eyebrow=""
        title="Focus Sessions"
        description="Choose a task. Pick your work apps. Start focusing."
      />
      <form
        className="panel form-panel simple-session"
        onSubmit={async (e) => {
          e.preventDefault();
          setProblem('');
          if (step === 0) {
            setStep(1);
            return;
          }
          if (
            web &&
            (!manual || strict) &&
            selected.general_internet &&
            !sites.trim()
          ) {
            setProblem('Add at least one work website, or turn off websites.');
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
            return;
          }
          if (
            await execute({
              type: 'create_session',
              title,
              minutes,
              allow_pause: !strict && pause,
              focus: selected,
            })
          ) {
            setTitle('');
            setStep(0);
          }
        }}
      >
        <div className="wizard-steps" aria-label="Setup progress">
          {['Task', 'Apps & websites', 'Ready'].map((label, i) => (
            <span key={label} className={step === i ? 'current' : ''}>
              {i + 1}. {label}
            </span>
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
                  ['standard', 'Flexible', 'Pause or stop whenever you need.'],
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
                        update({ mode: value as FocusConfig['mode'], apps: [] })
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
                Emergency exit is always available. App blocking hides windows;
                it is not a device lock.
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
                    onChange={(e) => update({ browser_app: e.target.value })}
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
                  <span>
                    Browser extension{' '}
                    <span
                      className={`badge ${setup?.browser.connected ? 'green' : ''}`}
                    >
                      {setup?.browser.connected ? 'Connected' : 'Setup needed'}
                    </span>
                  </span>
                  <button
                    type="button"
                    className="secondary"
                    onClick={() => setGuide(!guide)}
                  >
                    {guide ? 'Hide setup' : 'Connect browser'}
                  </button>
                </div>
                {guide && (
                  <BrowserSetup
                    setup={setup}
                    refresh={refresh}
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
                <details>
                  <summary>Website options</summary>
                  <label>
                    Internet in this browser
                    <select
                      value={focus.general_internet ? 'allowed' : 'offline'}
                      onChange={(e) =>
                        update({
                          general_internet: e.target.value === 'allowed',
                        })
                      }
                    >
                      <option value="allowed">Use my website choices</option>
                      <option value="offline">Block all websites</option>
                    </select>
                  </label>
                  <p className="small muted">
                    Other apps keep their internet access.
                  </p>
                </details>
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
            {!manual || strict || focus.apps.length > 0 ? (
              <>
                <label className="check-label consent">
                  <input
                    type="checkbox"
                    checked={focus.activity_consent}
                    onChange={(e) =>
                      update({ activity_consent: e.target.checked })
                    }
                  />
                  Allow app activity for this session
                </label>
                <p className="small muted">
                  Checks your current app{web ? ' and website name' : ''}. No
                  typing or page content is recorded.
                </p>
              </>
            ) : null}
            {attention.verification === 'camera_and_activity' && (
              <div className="setup-guide">
                <div className="setting-row">
                  <strong>Camera</strong>
                  <span>
                    {setup?.camera_permission === 'granted'
                      ? 'Ready'
                      : 'Permission needed'}
                  </span>
                </div>
                {setup?.camera_permission !== 'granted' && (
                  <div className="row controls">
                    <button
                      type="button"
                      onClick={() => void cameraPermission()}
                    >
                      Allow camera
                    </button>
                    <button
                      type="button"
                      className="secondary"
                      onClick={() => void refresh()}
                    >
                      Check camera
                    </button>
                  </div>
                )}
                <label className="check-label consent">
                  <input
                    type="checkbox"
                    checked={focus.camera}
                    disabled={setup?.camera_permission !== 'granted'}
                    onChange={(e) => update({ camera: e.target.checked })}
                  />
                  Use camera during this session
                </label>
                <small>Processed on your Mac. No video is saved.</small>
              </div>
            )}
            {strict && (
              <p className="small muted">
                No Pause or End button. Use Emergency exit if needed.
              </p>
            )}
            <details>
              <summary>More options</summary>
              <label>
                Wait before labeling a distraction
                <select
                  value={attention.grace_seconds}
                  onChange={(e) =>
                    updateAttention({ grace_seconds: Number(e.target.value) })
                  }
                >
                  <option value={3}>3 seconds</option>
                  <option value={10}>10 seconds (recommended)</option>
                  <option value={20}>20 seconds</option>
                </select>
              </label>
              <p className="small muted">
                Uncertain time does not count. This delay avoids quick
                distraction labels.
              </p>
              {!strict && (
                <label className="check-label consent">
                  <input
                    type="checkbox"
                    checked={pause}
                    onChange={(e) => setPause(e.target.checked)}
                  />
                  Allow pausing
                </label>
              )}
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
            </details>
            <p className="small muted">
              Closing the window keeps the session running. Reopen from the VY
              menu.
            </p>
          </>
        )}
        {(problem || error) && (
          <p className="error notice" role="alert">
            {problem || error}
          </p>
        )}
        <div className="row controls spread">
          <button
            type="button"
            className="secondary"
            disabled={step === 0}
            onClick={() => {
              setProblem('');
              setStep(step - 1);
            }}
          >
            Back
          </button>
          <button disabled={busy || !desktop}>
            {step === 2 ? 'Create session' : 'Continue'}
          </button>
        </div>
      </form>
      <div className="section-heading">
        <h2>Your sessions</h2>
      </div>
      <div className="session-list">
        {sessions.length ? (
          sessions.map((s) => <SessionCard session={s} key={s.id} />)
        ) : (
          <div className="panel">
            <Empty>No session yet.</Empty>
          </div>
        )}
      </div>
    </>
  );
}
