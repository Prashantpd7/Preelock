import { beforeEach, describe, expect, it } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { Analytics } from '../pages/Analytics';
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
function todayAt(hour: number): number {
  const date = new Date();
  date.setHours(hour, 0, 0, 0);
  return date.getTime();
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

function cardValue(label: string): string | null {
  const article = screen.getByText(label).closest('article');
  return article?.querySelector('strong')?.textContent ?? null;
}

describe('Analytics page', () => {
  it('renders six cards and follows the date filter', async () => {
    const eightDaysAgo = todayAt(9) - 8 * 86_400_000;
    useAppStore.setState({
      data: {
        ...structuredClone(emptySnapshot),
        sessions: [
          makeSession({
            title: 'Deep work',
            status: 'completed',
            started_at: todayAt(9),
            focused_ms: 25 * 60_000,
          }),
          makeSession({
            title: 'Email',
            status: 'ended',
            started_at: todayAt(10),
            focused_ms: 5 * 60_000,
          }),
          makeSession({
            title: 'Old session',
            status: 'completed',
            started_at: eightDaysAgo,
            focused_ms: 10 * 60_000,
          }),
        ],
      },
    });
    const user = userEvent.setup();
    render(<Analytics />);
    expect(screen.getByRole('button', { name: 'Last 7 days' })).toHaveAttribute(
      'aria-pressed',
      'true',
    );
    expect(cardValue('Focus time')).toBe('30m');
    expect(cardValue('Sessions completed')).toBe('1/2');
    expect(cardValue('Days focused')).toBe('1/7');
    expect(cardValue('Best session')).toBe('25m');
    expect(screen.getByText('Deep work')).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Today' }));
    expect(cardValue('Sessions completed')).toBe('1/2');
    expect(cardValue('Days focused')).toBe('1/1');
    await user.click(screen.getByRole('button', { name: 'All time' }));
    expect(cardValue('Sessions completed')).toBe('2/3');
    expect(cardValue('Focus time')).toBe('40m');
  });

  it('shows honest empty states with no sessions', () => {
    render(<Analytics />);
    expect(screen.getByText(/No sessions yet/)).toBeInTheDocument();
    expect(screen.getByText('0/0')).toBeInTheDocument();
    expect(screen.getAllByText('—')).toHaveLength(2);
    expect(screen.getByText('0 days')).toBeInTheDocument();
  });
});
