import { beforeEach, describe, expect, it } from 'vitest';
import { render, screen } from '@testing-library/react';
import { Sessions } from '../pages/Sessions';
import { useAppStore } from '../state/store';
import { emptySnapshot } from '../services/api';
import { defaultFocus, type Session } from '../types';

let seq = 0;
function makeSession(over: Partial<Session> = {}): Session {
  seq += 1;
  return {
    id: `s${seq}`,
    title: `Session ${seq}`,
    duration_minutes: 25,
    remaining_ms: 0,
    focused_ms: 0,
    allow_pause: true,
    status: 'completed',
    started_at: null,
    ended_at: null,
    deadline_ms: null,
    focus: defaultFocus,
    ...over,
  };
}

beforeEach(() => {
  seq = 0;
  useAppStore.setState({
    data: structuredClone(emptySnapshot),
    loading: false,
    error: null,
    busy: false,
  });
});

describe('Focus Sessions page', () => {
  it('shows the Activity content where the saved-sessions list was', () => {
    render(<Sessions />);
    expect(screen.queryByText('Your sessions')).not.toBeInTheDocument();
    expect(screen.queryByText('No session yet.')).not.toBeInTheDocument();
    expect(
      screen.getByRole('heading', { name: 'Activity' }),
    ).toBeInTheDocument();
    // The new-session entry point is untouched.
    expect(
      screen.getByRole('heading', { name: 'Focus Sessions' }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: /New session/ }),
    ).toBeInTheDocument();
  });

  it('lists a past session inside the embedded Activity section', () => {
    useAppStore.setState({
      data: {
        ...structuredClone(emptySnapshot),
        sessions: [
          makeSession({
            title: 'Deep work',
            status: 'completed',
            started_at: Date.now() - 3_600_000,
          }),
        ],
      },
    });
    render(<Sessions />);
    expect(screen.getByText('Deep work')).toBeInTheDocument();
    // No reuse affordance: the old per-card edit path is gone.
    expect(screen.queryByText('Your sessions')).not.toBeInTheDocument();
  });
});
