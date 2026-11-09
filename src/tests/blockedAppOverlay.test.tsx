import { beforeEach, expect, it, vi } from 'vitest';
import { fireEvent, render, screen } from '@testing-library/react';
import { BlockedAppOverlay } from '../components/BlockedAppOverlay';
import { api, emptySnapshot } from '../services/api';
import { useAppStore } from '../state/store';

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
    dismissBlockedAppPage: vi.fn().mockResolvedValue(undefined),
  },
}));

function seedNotice(
  notice: NonNullable<typeof emptySnapshot.blocked_app_notice>,
) {
  useAppStore.setState({
    data: { ...structuredClone(emptySnapshot), blocked_app_notice: notice },
    loading: false,
    error: null,
    busy: false,
  });
  render(<BlockedAppOverlay />);
}

beforeEach(() => {
  vi.clearAllMocks();
  useAppStore.setState({
    data: structuredClone(emptySnapshot),
    loading: false,
    error: null,
    busy: false,
  });
});

it('shows the blocked app reason like the website blocked page', () => {
  seedNotice({
    app_name: 'Safari',
    reason: 'Safari is blocked — your “Deep work” session is on.',
    ends_at_ms: null,
    at_ms: Date.now(),
  });
  expect(screen.getByText('Stay with your task')).toBeTruthy();
  expect(
    screen.getByText('Safari is blocked — your “Deep work” session is on.'),
  ).toBeTruthy();
  expect(screen.getByText('Back to work')).toBeTruthy();
});

it('shows a live countdown for timed locks', () => {
  seedNotice({
    app_name: 'Game',
    reason: 'Game is blocked for 2h 13m more',
    ends_at_ms: Date.now() + 2 * 3_600_000 + 13 * 60_000,
    at_ms: Date.now(),
  });
  expect(screen.getByText('Game is blocked for 2h 13m more')).toBeTruthy();
  // countdown() renders MM:SS — about 133 minutes left.
  expect(screen.getByLabelText('Time remaining').textContent).toMatch(
    /^133:/,
  );
});

it('dismisses the page from the Back to work button', () => {
  seedNotice({
    app_name: 'Safari',
    reason: 'Safari is blocked — your “Deep work” session is on.',
    ends_at_ms: null,
    at_ms: Date.now(),
  });
  fireEvent.click(screen.getByText('Back to work'));
  expect(api.dismissBlockedAppPage).toHaveBeenCalledTimes(1);
});

it('dismisses the page with the Escape key', () => {
  seedNotice({
    app_name: 'Safari',
    reason: 'Safari is blocked — your “Deep work” session is on.',
    ends_at_ms: null,
    at_ms: Date.now(),
  });
  fireEvent.keyDown(window, { key: 'Escape' });
  expect(api.dismissBlockedAppPage).toHaveBeenCalledTimes(1);
});

it('shows a quiet fallback when there is no notice', () => {
  render(<BlockedAppOverlay />);
  expect(screen.getByText('Nothing blocked')).toBeTruthy();
});
