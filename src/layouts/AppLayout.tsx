import { NavLink, Outlet, useLocation } from 'react-router-dom';
import {
  LayoutDashboard,
  Timer,
  ListTodo,
  Activity,
  ChartNoAxesColumn,
  Settings,
  ArrowUpRight,
  ShieldCheck,
  X,
} from 'lucide-react';
import { useAppStore } from '../state/store';
import { desktop } from '../services/api';
import { useApplication } from '../hooks/useApplication';
import { appConfig } from '../config';
const navigation = [
  ['/', 'Dashboard', LayoutDashboard],
  ['/sessions', 'Focus Sessions', Timer],
  ['/tasks', 'Tasks', ListTodo],
  ['/activity', 'Activity', Activity],
  ['/analytics', 'Analytics', ChartNoAxesColumn],
  ['/settings', 'Settings', Settings],
] as const;
export function AppLayout() {
  useApplication();
  const location = useLocation();
  const pageName =
    navigation.find(([path]) => path === location.pathname)?.[1] ?? 'Overview';
  const monitoring = useAppStore((s) =>
    s.data.sessions.some(
      (session) =>
        session.status === 'running' && session.focus?.activity_consent,
    ),
  );
  const camera = useAppStore((s) =>
    s.data.sessions.some(
      (session) =>
        session.status === 'running' &&
        session.focus?.camera &&
        ['person_detected', 'person_not_detected', 'extended_absence'].includes(
          session.report?.camera_status ?? '',
        ),
    ),
  );
  const error = useAppStore((s) => s.error);
  const loading = useAppStore((s) => s.loading);
  return (
    <div className="app-shell">
      <aside className="sidebar">
        <NavLink to="/" className="brand">
          <span className="brand-icon">V</span>VYSTRIQ
          <span className="brand-dot">.</span>
        </NavLink>
        <p className="nav-label">WORKSPACE</p>
        <nav aria-label="Main navigation">
          {navigation.map(([to, label, Icon]) => (
            <NavLink key={to} to={to} end={to === '/'}>
              <Icon size={18} />
              <span>{label}</span>
            </NavLink>
          ))}
        </nav>
        <div className="sidebar-note">
          <ShieldCheck size={20} />
          <h3>Your focus. Your space.</h3>
          <p>
            Stored on this device.
            <br />
            {camera
              ? 'Camera monitoring active'
              : monitoring
                ? 'Activity monitoring active'
                : 'No monitoring active.'}
          </p>
          <NavLink to="/settings">
            Privacy & capabilities <ArrowUpRight size={14} />
          </NavLink>
        </div>
        <div className="sidebar-footer">
          <span className="status-dot" />
          Local workspace<span className="version">v{appConfig.version}</span>
        </div>
      </aside>
      <div className="main-shell">
        <div className="topbar">
          <span>
            Personal workspace <span className="slash">/</span>{' '}
            <b>{pageName}</b>
          </span>
          <span className="small muted">
            {new Date().toLocaleDateString(undefined, {
              weekday: 'short',
              month: 'short',
              day: 'numeric',
            })}
          </span>
        </div>
        <main>
          {!desktop && (
            <div className="notice">
              Browser preview · Launch with <code>npm run desktop</code> to use
              SQLite-backed sessions and tasks.
            </div>
          )}
          {error && (
            <div className="error" role="alert">
              <span>{error}</span>
              <button
                className="icon-button"
                aria-label="Dismiss error"
                onClick={() => useAppStore.getState().clearError()}
              >
                <X size={16} />
              </button>
            </div>
          )}
          {loading ? <p role="status">Opening your workspace…</p> : <Outlet />}
        </main>
        <footer className="main-footer">
          Less noise. More meaningful work.
          <span>LOCAL FIRST · PRODUCTIVE TIME</span>
        </footer>
      </div>
    </div>
  );
}
