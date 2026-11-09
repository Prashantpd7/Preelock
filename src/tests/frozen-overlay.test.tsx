import { act, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { FrozenOverlay } from '../components/FrozenOverlay';
import { emptySnapshot } from '../services/api';
import { useAppStore } from '../state/store';
import { defaultFocus, type Session } from '../types';

function frozenSession(overrides: Partial<Session> = {}): Session {
  return {
    id: 'f1',
    title: 'Deep work',
    duration_minutes: 25,
    remaining_ms: 25 * 60_000,
    focused_ms: 0,
    allow_pause: false,
    status: 'running',
    started_at: Date.now() - 59_000,
    ended_at: null,
    deadline_ms: null,
    focus: {
      ...defaultFocus,
      frozen: { enabled: true, allowed_apps: [], allowed_websites: [] },
    },
    report: {
      events: [],
      enforcement: '',
      active_app: '',
      camera_status: 'off',
      browser_blocks: 0,
      interruptions: 0,
      approved_ms: 0,
      distracting_ms: 0,
      idle_ms: 0,
    },
    attention: {
      state: 'FOCUSED',
      reason: '',
      elapsed_ms: 0,
      distracted_ms: 0,
      absent_ms: 0,
      unknown_ms: 0,
      corrected_ms: 0,
      intervals: [],
    },
    ...overrides,
  } as Session;
}

describe('FrozenOverlay', () => {
  afterEach(() => {
    vi.useRealTimers();
  });
  it('shows the frozen timer and no unlock button', () => {
    vi.useFakeTimers();
    useAppStore.setState({
      data: { ...emptySnapshot, sessions: [frozenSession()] },
    });
    render(<FrozenOverlay />);
    expect(screen.getByText('Deep work')).toBeInTheDocument();
    // Wall-clock time left: 25 min duration, started 59 s ago —
    // shown as 24:00 once the 1-second clock tick fires.
    act(() => {
      vi.advanceTimersByTime(1000);
    });
    expect(screen.getByLabelText('Time remaining')).toHaveTextContent('24:00');
    expect(screen.queryByRole('button')).toBeNull();
    expect(screen.getByText(/no way to end it early/i)).toBeInTheDocument();
  });
  it('shows the ended state when no frozen session is running', () => {
    useAppStore.setState({
      data: {
        ...emptySnapshot,
        sessions: [frozenSession({ status: 'completed' })],
      },
    });
    render(<FrozenOverlay />);
    expect(screen.getByText('Frozen block ended')).toBeInTheDocument();
    expect(screen.queryByRole('button')).toBeNull();
  });
});
