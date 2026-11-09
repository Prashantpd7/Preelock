import { useEffect, useState } from 'react';
import { Link } from 'react-router-dom';
import { Plus } from 'lucide-react';
import { useAppStore } from '../state/store';
import {
  countdown,
  formatDuration,
  formatShortDuration,
  productiveStatistics,
  statistics,
} from '../services/analytics';
import { PageHeading, Stat } from '../components/ui';
import { dayCategoryTotals, localDayStart } from '../services/screenTimeWeek';
export function Dashboard() {
  const { sessions, tasks, preferences, browser, screen_time_usage } =
    useAppStore((s) => s.data);
  const stats = statistics(sessions, tasks);
  // Today's total screen time, from the always-on usage buckets — the
  // same source the Screen Time page uses. The total is
  // category-independent, so no category overrides are needed.
  const screenTimeTodayMs = dayCategoryTotals(
    screen_time_usage ?? [],
    localDayStart(new Date()),
    'all',
  ).total;
  const productive = productiveStatistics(sessions);
  const isLiveStatus = (s: (typeof sessions)[number]) =>
    ['running', 'paused', 'preparing'].includes(s.status);
  const liveSession = sessions.find((s) => isLiveStatus(s));
  // Does this session use browser website blocking? Mirrors the Rust
  // FocusConfig::network_required() logic.
  const sessionNeedsBrowser = (s: (typeof sessions)[number]) => {
    const f = s.focus;
    if (!f) return false;
    return (
      !f.general_internet ||
      (f.domains?.length ?? 0) > 0 ||
      (f.mode !== 'standard' && !!f.browser_app) ||
      (f.attention?.work_domains?.length ?? 0) > 0
    );
  };
  // Step 14 grace: a running session with a future
  // grace_until_ms has not started enforcing yet. The clock
  // lives in an effect (never in render, per the purity rule)
  // and ticks every second while grace is set, so the countdown
  // below stays live even between snapshot refreshes.
  const graceUntilMs = liveSession?.grace_until_ms ?? null;
  // The clock ticks from an effect, never from render (purity
  // rule): an immediate timeout syncs it the moment grace starts
  // (a mount-time initializer would go stale if the banner mounted
  // long before grace began), and the interval keeps it ticking.
  const [nowMs, setNowMs] = useState(() => Date.now());
  useEffect(() => {
    if (!graceUntilMs) return;
    const immediate = window.setTimeout(() => setNowMs(Date.now()), 0);
    const timer = window.setInterval(() => setNowMs(Date.now()), 1000);
    return () => {
      window.clearTimeout(immediate);
      window.clearInterval(timer);
    };
  }, [graceUntilMs]);
  const graceRemainingSecs =
    liveSession?.status === 'running' && graceUntilMs
      ? Math.max(0, Math.ceil((graceUntilMs - nowMs) / 1000))
      : 0;
  const inGrace = graceRemainingSecs > 0;
  // Show the reconnect button when the live session needs the browser
  // but the extension is not connected. The session keeps running and
  // apps stay blocked; only website blocking is paused.
  // Step 11 fail-closed: the live session's own flag is authoritative
  // (the shared browser status can show another browser connected).
  // A fail-closed session gets its own banner below, so the generic
  // transient-disconnect banner is suppressed for it.
  const showFailClosed =
    !!liveSession &&
    liveSession.status === 'running' &&
    !inGrace &&
    !!liveSession.browser_fail_closed;
  const showReconnectBrowser =
    !!liveSession &&
    liveSession.status === 'running' &&
    !inGrace &&
    sessionNeedsBrowser(liveSession) &&
    !browser?.connected &&
    !showFailClosed;
  const cameraBlocking =
    !!liveSession?.focus?.camera &&
    liveSession.status === 'running' &&
    !inGrace &&
    (liveSession.attention?.state === 'unknown' ||
      liveSession.attention?.state === 'absent');
  return (
    <>
      {liveSession && (
        <section
          className="panel active-session-banner"
          aria-live="polite"
          aria-label="Active session"
        >
          <p className="active-session-kicker">
            <span className="live-dot" aria-hidden="true" />
            Active session · {liveSession.title}
            {liveSession.status === 'preparing' && ' · waiting for browser'}
            {liveSession.status === 'paused' && ' · paused'}
            {inGrace && ' · starting soon'}
          </p>
          <p className="active-session-timer">
            {inGrace
              ? `Starts in ${graceRemainingSecs}s`
              : countdown(liveSession.remaining_ms)}
          </p>
          <p className="small muted">
            {inGrace
              ? 'Blocking begins when the countdown ends. Finish up — nothing is blocked yet.'
              : liveSession.status === 'preparing'
                ? 'Waiting for the browser extension to connect…'
                : 'Distracting apps and websites stay blocked until this session ends.'}
          </p>
          {showFailClosed && (
            <p>
              <Link
                className="button secondary"
                to="/settings?tab=Connections"
                aria-label="Connect browser from Settings Connections"
              >
                Connect browser
              </Link>
              <span className="small muted" style={{ marginLeft: '0.75rem' }}>
                Browser blocked until the extension is connected — this
                session&apos;s website rules were never applied, so the
                browser app itself stays blocked. Killing the app or
                removing the extension cannot end this block early; it
                resumes when Preelock restarts. This does not prevent
                uninstall or force-quit itself.
              </span>
            </p>
          )}
          {showReconnectBrowser && (
            <p>
              <Link
                className="button secondary"
                to="/settings?tab=Connections"
                aria-label="Connect browser from Settings Connections"
              >
                Connect browser
              </Link>
              <span className="small muted" style={{ marginLeft: '0.75rem' }}>
                Browser disconnected — apps stay blocked. Reconnect to resume
                website blocking.
              </span>
            </p>
          )}
          {cameraBlocking && (
            <p className="camera-warn" role="alert">
              Camera isn&apos;t detecting you — timer is paused. Sit in front of
              the camera to resume.
            </p>
          )}
        </section>
      )}
      <PageHeading
        eyebrow=""
        title="Make room for deep work."
        description="One intention. One focused session. A little more progress."
        action={
          <Link className="button" to="/sessions">
            <Plus size={17} />
            New session
          </Link>
        }
      />
      {!!preferences.daily_goal && (
        <section className="panel daily-goal">
          <div className="section-heading">
            <h2>Daily goal</h2>
            <strong>
              {formatShortDuration(
                statistics(
                  sessions.filter((s) => s.focus?.attention?.enabled),
                  tasks,
                ).today,
              )}{' '}
              / {formatShortDuration(preferences.daily_goal * 60000)}
            </strong>
          </div>
          <progress
            aria-label="Daily focus goal"
            max={preferences.daily_goal * 60000}
            value={Math.min(
              preferences.daily_goal * 60000,
              statistics(
                sessions.filter((s) => s.focus?.attention?.enabled),
                tasks,
              ).today,
            )}
          />
          <p className="small muted">Based on sessions started today.</p>
        </section>
      )}
      <div className="stats-grid">
        <Stat
          label="Screen Time"
          value={formatDuration(screenTimeTodayMs)}
          detail="Today's total screen time"
        />
        <Stat
          label="Focus time"
          value={formatDuration(productive.accepted)}
          detail="Based on your focus checks"
        />
        <Stat
          label="Total session time"
          value={formatDuration(productive.elapsed)}
          detail="Includes time the timer was paused"
        />
      </div>
      <div className="stats-grid">
        <Stat
          label="Distracted time"
          value={formatDuration(productive.distracted)}
          detail={`${productive.percentage}% focus · ${formatDuration(productive.unknown)} unverified`}
        />
        <Stat
          label="Focus today"
          value={formatDuration(
            statistics(
              sessions.filter((s) => s.focus?.attention?.enabled),
              tasks,
            ).today,
          )}
          detail="Time from sessions started today"
        />
        <Stat
          label="Sessions completed"
          value={String(stats.completed)}
          detail="All-time completed sessions"
        />
      </div>
    </>
  );
}
