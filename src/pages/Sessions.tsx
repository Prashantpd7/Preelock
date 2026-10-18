import { useState } from 'react';
import { Link } from 'react-router-dom';
import { useAppStore } from '../state/store';
import { desktop } from '../services/api';
import { PageHeading, Empty } from '../components/ui';
import { SessionCard } from '../components/SessionCard';
import { useFocusSetup } from '../hooks/useFocusSetup';
import { defaultFocus, type AttentionPolicy, type FocusConfig } from '../types';
export function Sessions() {
  const { data, execute, busy } = useAppStore();
  const { setup, error, loading, refresh } = useFocusSetup();
  const [step, setStep] = useState(0);
  const [title, setTitle] = useState('');
  const [minutes, setMinutes] = useState(data.preferences.default_minutes);
  const [pause, setPause] = useState(true);
  const [focus, setFocus] = useState<FocusConfig>({ ...defaultFocus });
  const [domains, setDomains] = useState('');
  const [workDomains, setWorkDomains] = useState('');
  const attention = focus.attention!;
  const updateAttention = (patch: Partial<AttentionPolicy>) =>
    setFocus((c) => ({ ...c, attention: { ...c.attention!, ...patch } }));
  const update = (patch: Partial<FocusConfig>) =>
    setFocus((c) => ({ ...c, ...patch }));
  const sessions = data.sessions
    .filter((s) =>
      ['ready', 'preparing', 'running', 'paused'].includes(s.status),
    )
    .slice()
    .reverse();
  const browsers =
    setup?.applications.filter((a) =>
      /chrome|chromium|edge|brave|vivaldi/i.test(a.id),
    ) ?? [];
  return (
    <>
      <PageHeading
        eyebrow="PROTECT YOUR ATTENTION"
        title="Focus Sessions"
        description="Choose your intention, your boundaries and what you consent to share."
      />
      <form
        className="panel form-panel"
        onSubmit={async (e) => {
          e.preventDefault();
          if (step < 2) {
            setStep(step + 1);
            return;
          }
          if (
            await execute({
              type: 'create_session',
              title,
              minutes,
              allow_pause: focus.mode === 'standard' && pause,
              focus: {
                ...focus,
                attention: {
                  ...attention,
                  work_domains: workDomains
                    .split(/[\n,]/)
                    .map((s) => s.trim())
                    .filter(Boolean),
                },
                domains: domains
                  .split(/[\n,]/)
                  .map((s) => s.trim())
                  .filter(Boolean),
              },
            })
          ) {
            setTitle('');
            setStep(0);
          }
        }}
      >
        <div className="wizard-steps" aria-label="Setup progress">
          {['Intention', 'Boundaries', 'Review & consent'].map((label, i) => (
            <span key={label} className={i === step ? 'current' : ''}>
              {i + 1}. {label}
            </span>
          ))}
        </div>
        {step === 0 && (
          <>
            <h2>Your next meaningful task</h2>
            <div className="form-grid">
              <label className="grow">
                Your intention
                <input
                  placeholder="What will you focus on?"
                  required
                  maxLength={160}
                  value={title}
                  onChange={(e) => setTitle(e.target.value)}
                />
              </label>
              <label>
                Productive target (minutes)
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
            <div className="form-grid">
              <label>
                Task context
                <select
                  value={attention.task}
                  onChange={(e) => updateAttention({ task: e.target.value })}
                >
                  <option value="other">Other focused work</option>
                  <option value="coding">Coding / debugging</option>
                  <option value="reading">Reading / studying</option>
                  <option value="lecture">Watching lectures</option>
                  <option value="writing">Writing</option>
                </select>
              </label>
              <label>
                Time acceptance policy
                <select
                  value={attention.verification}
                  onChange={(e) =>
                    updateAttention({
                      verification: e.target
                        .value as AttentionPolicy['verification'],
                    })
                  }
                >
                  <option value="activity_only">
                    Approved app and domain activity
                  </option>
                  <option value="camera_and_activity">
                    Camera presence plus approved activity
                  </option>
                  <option value="manual_checkpoints">
                    Self-reported 60-second work checkpoints
                  </option>
                </select>
              </label>
              <label>
                Distraction / absence grace
                <select
                  value={attention.grace_seconds}
                  onChange={(e) =>
                    updateAttention({ grace_seconds: Number(e.target.value) })
                  }
                >
                  <option value={3}>Sensitive · 3 seconds</option>
                  <option value={10}>Balanced · 10 seconds</option>
                  <option value={20}>Gentle · 20 seconds</option>
                </select>
              </label>
            </div>
            <p className="muted small">
              Only accepted work reduces the target. Uncertain observations
              pause credit immediately; sustained evidence determines
              distraction or absence. Quiet reading has a longer idle allowance.
              Activity and camera presence cannot prove comprehension.
            </p>
            <label>
              Restriction mode
              <select
                value={focus.mode}
                onChange={(e) =>
                  update({
                    mode: e.target.value as FocusConfig['mode'],
                    apps: [],
                  })
                }
              >
                <option value="standard">
                  Standard · selected distractions
                </option>
                <option value="strict">
                  Strict · approved apps and browser domains
                </option>
                <option value="deep" disabled>
                  Deep Focus · managed-device setup unavailable
                </option>
              </select>
            </label>
            <p className="muted small mode-note">
              {focus.mode === 'standard'
                ? 'Selected apps are hidden/minimized. Selected domains are blocked in the paired browser.'
                : 'Unapproved user apps are hidden/minimized. The paired browser uses an allowlist. Ordinary pause/end controls are removed; emergency exit stays accessible.'}{' '}
              These are voluntary restrictions, not tamper-proof device locks.
            </p>
          </>
        )}
        {step === 1 && (
          <>
            <div className="section-heading">
              <h2>
                {focus.mode === 'standard'
                  ? 'Distracting applications'
                  : 'Approved applications'}
              </h2>
              <button
                type="button"
                className="secondary"
                disabled={loading || !desktop}
                onClick={() => void refresh()}
              >
                Refresh running apps
              </button>
            </div>
            {error && <p role="alert">{error}</p>}
            <div className="application-picker">
              {setup?.applications.length ? (
                setup.applications
                  .filter(
                    (a) =>
                      !/vystriq|finder|activitymonitor|systempreferences|explorer\.exe|taskmgr\.exe|systemsettings\.exe/i.test(
                        a.id,
                      ),
                  )
                  .map((app) => (
                    <label className="check-label" key={app.id}>
                      <input
                        type="checkbox"
                        checked={focus.apps.includes(app.id)}
                        onChange={(e) =>
                          update({
                            apps: e.target.checked
                              ? [...focus.apps, app.id]
                              : focus.apps.filter((id) => id !== app.id),
                          })
                        }
                      />
                      <span>
                        {app.name}
                        <small className="app-id">{app.id}</small>
                      </span>
                    </label>
                  ))
              ) : (
                <p className="muted small">
                  Open applications you want to select, then refresh in the
                  desktop app.
                </p>
              )}
            </div>
            {focus.mode === 'standard' &&
              attention.verification !== 'manual_checkpoints' && (
                <>
                  <h3>Productive applications</h3>
                  <p className="muted small">
                    Only these apps can earn time; keep them separate from
                    distracting apps above.
                  </p>
                  <div className="application-picker">
                    {setup?.applications
                      .filter((a) => !focus.apps.includes(a.id))
                      .map((app) => (
                        <label className="check-label" key={app.id}>
                          <input
                            type="checkbox"
                            checked={attention.work_apps.includes(app.id)}
                            onChange={(e) =>
                              updateAttention({
                                work_apps: e.target.checked
                                  ? [...attention.work_apps, app.id]
                                  : attention.work_apps.filter(
                                      (id) => id !== app.id,
                                    ),
                              })
                            }
                          />
                          {app.name}
                        </label>
                      ))}
                  </div>
                  <label>
                    Productive browser domains
                    <textarea
                      rows={3}
                      value={workDomains}
                      onChange={(e) => setWorkDomains(e.target.value)}
                      placeholder="docs.rs, github.com"
                    />
                  </label>
                </>
              )}
            <div className="form-grid">
              <label className="grow">
                Paired Chromium browser
                <select
                  value={focus.browser_app}
                  onChange={(e) => update({ browser_app: e.target.value })}
                >
                  <option value="">No browser filter selected</option>
                  {browsers.map((app) => (
                    <option key={app.id} value={app.id}>
                      {app.name}
                    </option>
                  ))}
                </select>
              </label>
            </div>
            <label className="check-label">
              <input
                type="checkbox"
                checked={focus.general_internet}
                onChange={(e) => update({ general_internet: e.target.checked })}
              />
              {focus.mode === 'standard'
                ? 'Permit browser internet except blocked domains'
                : 'Permit approved browser domains only'}
            </label>
            <p className="muted small">
              Unchecked = offline in the paired browser; localhost remains
              available. Other applications retain network access.
            </p>
            <label className="domain-label">
              {focus.mode === 'standard'
                ? 'Blocked domains'
                : 'Approved domains'}
              <textarea
                rows={4}
                value={domains}
                onChange={(e) => setDomains(e.target.value)}
                placeholder="One domain per line, e.g. docs.rs"
              />
            </label>
            <div className="row controls">
              <button
                type="button"
                className="secondary"
                onClick={() =>
                  setDomains(
                    focus.mode === 'standard'
                      ? 'reddit.com\nx.com\ninstagram.com\ntiktok.com'
                      : 'developer.mozilla.org\ndocs.rs\ngithub.com\nstackoverflow.com',
                  )
                }
              >
                {focus.mode === 'standard'
                  ? 'Use distraction preset'
                  : 'Use coding preset'}
              </button>
              {focus.mode !== 'standard' && (
                <button
                  type="button"
                  className="secondary"
                  onClick={() => setDomains('coursera.org\nedx.org')}
                >
                  Use lecture preset
                </button>
              )}
            </div>
            <p className="muted small">
              Domains include subdomains. Add necessary media/CDN/login domains
              yourself; presets cannot guarantee a platform works.
            </p>
          </>
        )}
        {step === 2 && (
          <>
            <h2>
              {title} · {minutes} minutes
            </h2>
            <p className="muted mode-note">
              {focus.mode === 'strict'
                ? 'Strict commitment: ordinary pause and end are disabled.'
                : 'Standard session with reversible restrictions.'}{' '}
              App selection: {focus.apps.length}. Browser filter:{' '}
              {focus.mode === 'strict' ||
              !focus.general_internet ||
              domains.trim()
                ? 'requires paired extension acknowledgement before the timer starts'
                : 'not requested'}
              .
            </p>
            <label className="check-label consent">
              <input
                type="checkbox"
                checked={focus.activity_consent}
                onChange={(e) => update({ activity_consent: e.target.checked })}
              />
              I consent to foreground-app and idle-time sampling during this
              session, and hostname-only browser sampling when paired. Required
              for app restrictions and activity verification.
            </label>
            <label className="check-label consent">
              <input
                type="checkbox"
                checked={focus.camera}
                disabled={setup?.camera_permission !== 'granted'}
                onChange={(e) => update({ camera: e.target.checked })}
              />
              Enable local, probabilistic camera presence detection (
              {setup?.camera_permission ?? 'permission not checked'})
            </label>
            {attention.verification === 'camera_and_activity' && (
              <label>
                When camera evidence is unavailable
                <select
                  value={attention.camera_fallback}
                  onChange={(e) =>
                    updateAttention({
                      camera_fallback: e.target
                        .value as AttentionPolicy['camera_fallback'],
                    })
                  }
                >
                  <option value="unknown">Pause credit and show UNKNOWN</option>
                  <option value="activity_only">
                    Explicitly accept approved activity without camera
                  </option>
                </select>
              </label>
            )}
            <p className="notice">
              Readiness: native activity{' '}
              {setup?.native_available ? 'available' : 'unavailable'} · camera{' '}
              {setup?.camera_permission ?? 'unchecked'} · browser{' '}
              {setup?.browser.connected ? 'connected' : 'not paired'}. Closing
              the window keeps sessions running; reopen from the menu bar /
              system tray.
            </p>
            {focus.mode === 'standard' && (
              <label className="check-label consent">
                <input
                  type="checkbox"
                  checked={pause}
                  onChange={(e) => setPause(e.target.checked)}
                />
                Allow pausing
              </label>
            )}
            <p className="notice">
              Activity stays local. No page content, keystrokes or raw camera
              footage is retained. No system-wide internet blocking, VPN
              resistance or learning verification is claimed. Emergency recovery
              is always accessible.
            </p>
            <Link to="/settings">Open permissions & browser setup →</Link>
          </>
        )}
        <div className="row controls spread">
          <button
            type="button"
            className="secondary"
            disabled={step === 0}
            onClick={() => setStep(step - 1)}
          >
            Back
          </button>
          <button disabled={busy || !desktop}>
            {step === 2 ? 'Create session' : 'Continue'}
          </button>
        </div>
      </form>
      <div className="section-heading">
        <h2>Planned & active</h2>
        <span className="muted small">One active session at a time</span>
      </div>
      <div className="session-list">
        {sessions.length ? (
          sessions.map((s) => <SessionCard key={s.id} session={s} />)
        ) : (
          <div className="panel">
            <Empty>Set an intention above to prepare your next session.</Empty>
          </div>
        )}
      </div>
    </>
  );
}
