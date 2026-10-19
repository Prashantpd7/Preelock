import { BreakBar } from '../components/BreakBar';
import { NavLink, useLocation } from 'react-router-dom';
import {
  LayoutDashboard,
  Timer,
  ListTodo,
  Activity,
  ChartNoAxesColumn,
  Settings,
  X,
} from 'lucide-react';
import { useAppStore } from '../state/store';
import { desktop } from '../services/api';
import { useApplication } from '../hooks/useApplication';
import { Dashboard } from '../pages/Dashboard';
import { Sessions } from '../pages/Sessions';
import { Tasks } from '../pages/Tasks';
import { Activity as ActivityPage } from '../pages/Activity';
import { Analytics } from '../pages/Analytics';
import { Settings as SettingsPage } from '../pages/Settings';
const navigation = [
  ['/', 'Dashboard', LayoutDashboard],
  ['/sessions', 'Focus Sessions', Timer],
  ['/tasks', 'Tasks', ListTodo],
  ['/activity', 'Activity', Activity],
  ['/analytics', 'Analytics', ChartNoAxesColumn],
  ['/settings', 'Settings', Settings],
] as const;
const pagePaths = [
  '/',
  '/sessions',
  '/tasks',
  '/activity',
  '/analytics',
  '/settings',
];
export function AppLayout() {
  useApplication();
  const { pathname } = useLocation();
  const activePath = pagePaths.includes(pathname) ? pathname : '/';
  const error = useAppStore((s) => s.error);
  const loading = useAppStore((s) => s.loading);
  return (
    <div className="app-shell">
      <aside className="sidebar">
        <NavLink to="/" className="brand">
          <span className="brand-icon">V</span>VYSTRIQ
          <span className="brand-dot">.</span>
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
              <div hidden={activePath !== '/'}>
                <Dashboard />
              </div>
              <div hidden={activePath !== '/sessions'}>
                <Sessions />
              </div>
              <div hidden={activePath !== '/tasks'}>
                <Tasks />
              </div>
              <div hidden={activePath !== '/activity'}>
                <ActivityPage />
              </div>
              <div hidden={activePath !== '/analytics'}>
                <Analytics />
              </div>
              {activePath === '/settings' && <SettingsPage />}
            </>
          )}
        </main>
      </div>
    </div>
  );
}
