import { beforeEach, expect, it, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import { App } from '../App';
import { api, emptySnapshot } from '../services/api';
import { useAppStore } from '../state/store';
import type { ScreenTimeBucket } from '../types';

vi.mock('../services/settings', () => ({
  settingsAction: vi.fn().mockResolvedValue({}),
  preferenceDefaults: {
    theme: 'dark',
    default_minutes: 25,
    saved_setups: [],
    reminder_days: [],
  },
}));

vi.mock('../services/api', () => ({
  desktop: true,
  emptySnapshot: {
    sessions: [],
    tasks: [],
    schedules: [],
    preferences: { theme: 'dark', default_minutes: 25 },
  },
  api: {
    snapshot: vi.fn(),
    execute: vi.fn(),
    focusSetup: vi.fn(),
    capabilities: vi.fn(),
    blockAttempts: vi.fn().mockResolvedValue([]),
    appIcon: vi.fn().mockResolvedValue(null),
  },
}));

function bucket(
  name: string,
  minutes: number,
  dayOffset = 0,
): ScreenTimeBucket {
  const day = new Date();
  day.setDate(day.getDate() + dayOffset);
  day.setHours(0, 0, 0, 0);
  return {
    day_ms: day.getTime(),
    kind: 'app',
    target: `com.example.${name.split(' ')[0].toLowerCase()}`,
    name,
    ms: minutes * 60_000,
  };
}

// 90 + 60 minutes today (2h 30m total); yesterday's 60 minutes must be
// excluded from the dashboard card.
function seedDashboard() {
  window.location.hash = '#/';
  useAppStore.setState({
    data: {
      ...structuredClone(emptySnapshot),
      screen_time_usage: [
        bucket('Alpha', 90),
        bucket('Beta', 60),
        bucket('Yesterday', 60, -1),
      ],
    },
    loading: false,
    error: null,
    busy: false,
  });
  render(<App />);
}

// AppLayout keeps every page mounted in hidden divs, and other pages
// (Screen Time, Analytics) render their own stat cards and totals —
// scope all card queries to the visible dashboard page only.
function dashboardGrids(): Element[] {
  return Array.from(document.querySelectorAll('.stats-grid')).filter(
    (grid) => !grid.closest('div[hidden]'),
  );
}

function screenTimeCard(): Element | undefined {
  return Array.from(dashboardGrids()[0].querySelectorAll('article')).find(
    (article) => article.textContent?.includes('Screen Time'),
  );
}

beforeEach(() => {
  window.location.hash = '#/';
  localStorage.setItem('preelock-permissions-v2', 'seen');
  vi.clearAllMocks();
  vi.mocked(api.snapshot).mockImplementation(async () =>
    structuredClone(useAppStore.getState().data),
  );
  vi.mocked(api.capabilities).mockResolvedValue([]);
  vi.mocked(api.focusSetup).mockResolvedValue({
    platform: 'macos',
    native_available: true,
    applications: [],
    camera_permission: 'not_requested',
    pairing_token: 'test-token',
    browser: {
      available: true,
      connected: false,
      applied_id: '',
      error: '',
      blocks: 0,
      cleanup_pending: false,
      extension_stale: false,
    },
  });
});

it('shows the Screen Time card as the first dashboard card', () => {
  seedDashboard();
  const grids = dashboardGrids();
  expect(grids.length).toBe(2);
  const firstGridCards = grids[0].querySelectorAll('article');
  expect(firstGridCards.length).toBe(3);
  expect(firstGridCards[0].textContent).toContain('Screen Time');
});

it('removes the Task progress card from the dashboard', () => {
  seedDashboard();
  const labels = Array.from(
    dashboardGrids()[1].querySelectorAll('article'),
  ).map((article) => article.querySelector('span')?.textContent);
  expect(labels).not.toContain('Task progress');
  expect(screen.queryByText('Task progress')).toBeNull();
});

it("shows today's total screen time, ignoring other days", () => {
  seedDashboard();
  const card = screenTimeCard();
  expect(card).toBeTruthy();
  // 90 + 60 minutes today; yesterday's 60 minutes excluded.
  expect(card?.querySelector('strong')?.textContent).toBe('2h 30m');
  expect(card?.textContent).toContain("Today's total screen time");
});
