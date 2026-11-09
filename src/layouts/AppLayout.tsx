import { useEffect } from 'react';
import { FirstRunPermissions } from '../components/PermissionsGuide';
import { BreakBar } from '../components/BreakBar';
import { NavLink, useLocation } from 'react-router-dom';
import {
  LayoutDashboard,
  Timer,
  ListTodo,
  ChartNoAxesColumn,
  Clock3,
  Settings,
  X,
} from 'lucide-react';
import { useAppStore } from '../state/store';
import { desktop } from '../services/api';
import { useApplication } from '../hooks/useApplication';
import { Dashboard } from '../pages/Dashboard';
import { Sessions } from '../pages/Sessions';
import { Tasks } from '../pages/Tasks';
import { Analytics } from '../pages/Analytics';
import { ScreenTime } from '../pages/ScreenTime';
import { Settings as SettingsPage } from '../pages/Settings';
import logo from '../assets/preelock-logo.png';
const navigation = [
  ['/', 'Dashboard', LayoutDashboard],
  ['/sessions', 'Focus Sessions', Timer],
  ['/tasks', 'Tasks', ListTodo],
  ['/analytics', 'Analytics', ChartNoAxesColumn],
  ['/screen-time', 'Screen Time', Clock3],
  ['/settings', 'Settings', Settings],
] as const;
const pagePaths = [
  '/',
  '/sessions',
  '/tasks',
  '/analytics',
  '/screen-time',
  '/settings',
];
export function AppLayout() {
  useApplication();
  const { pathname } = useLocation();
  const activePath = pagePaths.includes(pathname) ? pathname : '/';
  const error = useAppStore((s) => s.error);
  const loading = useAppStore((s) => s.loading);
  // Pages stay mounted in hidden divs, so the window scroll position
  // otherwise persists across navigation and a newly opened page can
  // land mid-scroll with a heading half-cut at the viewport top.
  useEffect(() => {
    window.scrollTo(0, 0);
  }, [activePath]);
  return (
    <div className="app-shell">
      <aside className="sidebar">
        <NavLink to="/" className="brand">
          <img className="brand-icon" src={logo} alt="" />
          Preelock
        </NavLink>
        <nav aria-label="Main navigation">
          {navigation.map(([to, label, Icon]) => (
            <NavLink key={to} to={to} end={to === '/'}>
              <Icon size={18} />
              <span>{label}</span>
            </NavLink>
          ))}
        </nav>
      </aside>
      <div className="main-shell">
        <main>
          <BreakBar />
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
          {loading ? (
            <p role="status">Opening your workspace…</p>
          ) : (
            <>
              <FirstRunPermissions />
              <div hidden={activePath !== '/'}>
                <Dashboard />
              </div>
              <div hidden={activePath !== '/sessions'}>
                <Sessions />
              </div>
              <div hidden={activePath !== '/tasks'}>
                <Tasks />
              </div>
              <div hidden={activePath !== '/analytics'}>
                <Analytics />
              </div>
              <div hidden={activePath !== '/screen-time'}>
                <ScreenTime />
              </div>
              {activePath === '/settings' && <SettingsPage />}
            </>
          )}
        </main>
      </div>
    </div>
  );
}
