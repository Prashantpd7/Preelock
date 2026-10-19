import { HashRouter, Route, Routes } from 'react-router-dom';
import { AppLayout } from './layouts/AppLayout';
export function App() {
  return (
    <HashRouter>
      <Routes>
        <Route path="*" element={<AppLayout />} />
      </Routes>
    </HashRouter>
  );
}
