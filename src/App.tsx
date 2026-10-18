import { HashRouter, Navigate, Route, Routes } from 'react-router-dom';
import { AppLayout } from './layouts/AppLayout';
import { Dashboard } from './pages/Dashboard';
import { Sessions } from './pages/Sessions';
import { Tasks } from './pages/Tasks';
import { Activity } from './pages/Activity';
import { Analytics } from './pages/Analytics';
import { Settings } from './pages/Settings';
export function App() {
  return (
    <HashRouter>
      <Routes>
        <Route element={<AppLayout />}>
          <Route index element={<Dashboard />} />
          <Route path="sessions" element={<Sessions />} />
          <Route path="tasks" element={<Tasks />} />
          <Route path="activity" element={<Activity />} />
          <Route path="analytics" element={<Analytics />} />
          <Route path="settings" element={<Settings />} />
          <Route path="*" element={<Navigate to="/" replace />} />
        </Route>
      </Routes>
    </HashRouter>
  );
}
