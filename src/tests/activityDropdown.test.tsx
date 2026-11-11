import { beforeEach, describe, expect, it } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { Sessions } from '../pages/Sessions';
import { useAppStore } from '../state/store';
import { emptySnapshot } from '../services/api';
import { defaultActivitySection } from '../services/activitySection';
import {
  defaultFocus,
  type Allowance,
  type OneShot,
  type Schedule,
  type Session,
  type Snapshot,
} from '../types';

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
function makeSchedule(over: Partial<Schedule> = {}): Schedule {
  return {
    id: 'sched-1',
    title: 'Morning block',
    days: [1, 2, 3, 4, 5],
    start_time: '09:00',
    end_time: '10:00',
    focus: structuredClone(defaultFocus),
    enabled: true,
    lock_changes: false,
    created_at: 1,
    ...over,
  };
}
function makeOneShot(over: Partial<OneShot> = {}): OneShot {
  return {
    id: 'one-1',
    title: 'Evening timer',
    minutes: 25,
    allow_pause: true,
    focus: structuredClone(defaultFocus),
    fire_at_ms: Date.now() + 3_600_000,
    created_at: 1,
    ...over,
  };
}
function makeAllowance(over: Partial<Allowance> = {}): Allowance {
  return {
    id: 'allow-1',
    title: 'YouTube budget',
    kind: 'site',
    target: 'youtube.com',
    budget_minutes: 30,
    period: 'day',
    mode: 'rolling',
    enabled: true,
    created_at: 1,
    ...over,
  };
}
function setData(data: Partial<Snapshot>) {
  useAppStore.setState({
    data: { ...structuredClone(emptySnapshot), ...data },
    loading: false,
    error: null,
    busy: false,
  });
}

beforeEach(() => {
  seq = 0;
  setData({});
});

describe('defaultActivitySection', () => {
  it('prefers Focus Sessions when a session is live', () => {
    const data = {
      ...structuredClone(emptySnapshot),
      sessions: [makeSession({ status: 'running' })],
      schedules: [makeSchedule()],
    };
    expect(defaultActivitySection(data)).toBe('sessions');
  });
  it('prefers Schedules over one-shots and allowances', () => {
    const data = {
      ...structuredClone(emptySnapshot),
      schedules: [makeSchedule()],
      one_shots: [makeOneShot()],
      allowances: [makeAllowance()],
    };
    expect(defaultActivitySection(data)).toBe('schedules');
  });
  it('prefers upcoming one-shots over allowances, and ignores past ones', () => {
    const now = Date.now();
    const data = {
      ...structuredClone(emptySnapshot),
      one_shots: [makeOneShot({ fire_at_ms: now - 1_000 })],
      allowances: [makeAllowance()],
    };
    expect(defaultActivitySection(data, now)).toBe('allowances');
    expect(
      defaultActivitySection(
        {
          ...structuredClone(emptySnapshot),
          one_shots: [makeOneShot({ fire_at_ms: now + 60_000 })],
        },
        now,
      ),
    ).toBe('oneshots');
  });
  it('falls back to Focus Sessions history when nothing exists', () => {
    expect(defaultActivitySection(structuredClone(emptySnapshot))).toBe(
      'sessions',
    );
  });
});

describe('Focus Sessions cards', () => {
  it('has no large empty-state boxes and keeps all four action buttons', () => {
    render(<Sessions />);
    expect(screen.queryByText('No active session')).not.toBeInTheDocument();
    expect(screen.queryByText('No schedules yet.')).not.toBeInTheDocument();
    expect(
      screen.queryByText('No one-shot timers yet.'),
    ).not.toBeInTheDocument();
    expect(screen.queryByText('No allowances yet.')).not.toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: 'New session' }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: 'New schedule' }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: 'New one-shot' }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: 'New allowance' }),
    ).toBeInTheDocument();
  });
});

describe('Activity dropdown', () => {
  it('auto-selects Schedules when schedules exist, and lists them', () => {
    setData({ schedules: [makeSchedule()] });
    render(<Sessions />);
    expect(
      screen.getByRole('button', { name: 'Schedules' }),
    ).toBeInTheDocument();
    expect(screen.getByText('Morning block')).toBeInTheDocument();
  });
  it('auto-selects Start once at for an upcoming one-shot', () => {
    setData({ one_shots: [makeOneShot()] });
    render(<Sessions />);
    expect(
      screen.getByRole('button', { name: 'Start once at' }),
    ).toBeInTheDocument();
    expect(screen.getByText('Evening timer')).toBeInTheDocument();
  });
  it('auto-selects Allowances when only allowances exist', () => {
    setData({ allowances: [makeAllowance()] });
    render(<Sessions />);
    expect(
      screen.getByRole('button', { name: 'Allowances' }),
    ).toBeInTheDocument();
    expect(screen.getByText('YouTube budget')).toBeInTheDocument();
  });
  it('shows only the selected section, and the options in order', async () => {
    setData({
      schedules: [makeSchedule()],
      one_shots: [makeOneShot()],
      allowances: [makeAllowance()],
    });
    const user = userEvent.setup();
    render(<Sessions />);
    // Schedules is the default (priority over one-shots/allowances).
    expect(screen.getByText('Morning block')).toBeInTheDocument();
    expect(screen.queryByText('Evening timer')).not.toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Schedules' }));
    expect(screen.getAllByRole('option').map((o) => o.textContent)).toEqual([
      'Focus Sessions',
      'Schedules',
      'Start once at',
      'Allowances',
    ]);
    await user.click(screen.getByRole('option', { name: 'Start once at' }));
    expect(screen.getByText('Evening timer')).toBeInTheDocument();
    expect(screen.queryByText('Morning block')).not.toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: 'Start once at' }),
    ).toBeInTheDocument();
  });
  it('keeps a manual selection when the data changes', async () => {
    setData({ schedules: [makeSchedule()] });
    const user = userEvent.setup();
    const { rerender } = render(<Sessions />);
    expect(
      screen.getByRole('button', { name: 'Schedules' }),
    ).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Schedules' }));
    await user.click(screen.getByRole('option', { name: 'Focus Sessions' }));
    expect(
      screen.getByRole('button', { name: 'Focus Sessions' }),
    ).toBeInTheDocument();
    // New data arrives that would have defaulted to Allowances — the
    // user's choice must stick.
    setData({ schedules: [makeSchedule()], allowances: [makeAllowance()] });
    rerender(<Sessions />);
    expect(
      screen.getByRole('button', { name: 'Focus Sessions' }),
    ).toBeInTheDocument();
    expect(screen.queryByText('YouTube budget')).not.toBeInTheDocument();
  });
  it('excludes a live session from history but keeps completed ones', () => {
    setData({
      sessions: [
        makeSession({
          title: 'Running work',
          status: 'running',
          started_at: Date.now() - 60_000,
        }),
        makeSession({
          title: 'Done work',
          status: 'completed',
          started_at: Date.now() - 3_600_000,
        }),
      ],
    });
    render(<Sessions />);
    // A live session forces the Focus Sessions default.
    expect(
      screen.getByRole('button', { name: 'Focus Sessions' }),
    ).toBeInTheDocument();
    expect(screen.getByText('Done work')).toBeInTheDocument();
    expect(screen.queryByText('Running work')).not.toBeInTheDocument();
  });
  it('opens the New allowance form from the Allowances card', async () => {
    const user = userEvent.setup();
    render(<Sessions />);
    await user.click(screen.getByRole('button', { name: 'New allowance' }));
    expect(
      screen.getByRole('button', { name: 'Allowances' }),
    ).toBeInTheDocument();
    expect(screen.getByLabelText('Allowance name')).toBeInTheDocument();
  });
  it('shows a compact empty state without a large box when a section is empty', async () => {
    setData({ schedules: [makeSchedule()] });
    const user = userEvent.setup();
    render(<Sessions />);
    await user.click(screen.getByRole('button', { name: 'Schedules' }));
    await user.click(screen.getByRole('option', { name: 'Start once at' }));
    expect(screen.getByText('No one-shot timers yet.')).toBeInTheDocument();
    // Two "New one-shot" buttons by design: the card's and the
    // section's. Both open the same wizard.
    expect(
      screen.getAllByRole('button', { name: 'New one-shot' }),
    ).toHaveLength(2);
  });
});

describe('Activity date filter', () => {
  const tenDaysAgo = () => Date.now() - 10 * 86_400_000;

  it('is visible in all four sections, beside the dropdown', async () => {
    setData({
      sessions: [
        makeSession({
          title: 'Done work',
          status: 'completed',
          started_at: Date.now() - 3_600_000,
        }),
      ],
      schedules: [makeSchedule({ created_at: Date.now() })],
      one_shots: [makeOneShot()],
      allowances: [makeAllowance({ created_at: Date.now() })],
    });
    const user = userEvent.setup();
    render(<Sessions />);
    // The dropdown trigger and the date filters share one toolbar.
    const toolbar = document.querySelector('.activity-toolbar');
    expect(toolbar).not.toBeNull();
    const expectFilterBesideDropdown = (triggerName: string) => {
      const trigger = screen.getByRole('button', { name: triggerName });
      const group = screen.getByRole('group', { name: 'Date range' });
      expect(trigger).toBeInTheDocument();
      expect(group).toBeInTheDocument();
      expect(toolbar?.contains(trigger)).toBe(true);
      expect(toolbar?.contains(group)).toBe(true);
    };
    const goTo = async (from: string, to: string) => {
      await user.click(screen.getByRole('button', { name: from }));
      await user.click(screen.getByRole('option', { name: to }));
    };
    // Default is Schedules (a schedule exists, no live session).
    expectFilterBesideDropdown('Schedules');
    await goTo('Schedules', 'Focus Sessions');
    expectFilterBesideDropdown('Focus Sessions');
    await goTo('Focus Sessions', 'Start once at');
    expectFilterBesideDropdown('Start once at');
    await goTo('Start once at', 'Allowances');
    expectFilterBesideDropdown('Allowances');
  });

  it('filters session history by started_at', async () => {
    setData({
      sessions: [
        makeSession({
          title: 'Old work',
          status: 'completed',
          started_at: tenDaysAgo(),
        }),
      ],
    });
    const user = userEvent.setup();
    render(<Sessions />);
    expect(screen.getByText('Old work')).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Today' }));
    expect(screen.queryByText('Old work')).not.toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'All time' }));
    expect(screen.getByText('Old work')).toBeInTheDocument();
  });

  it('filters one-shots by their scheduled start', async () => {
    setData({
      one_shots: [
        makeOneShot({
          title: 'Old timer',
          fire_at_ms: tenDaysAgo(),
        }),
        makeOneShot({
          id: 'one-2',
          title: 'Later today',
          fire_at_ms: Date.now() + 3_600_000,
        }),
      ],
    });
    const user = userEvent.setup();
    render(<Sessions />);
    // 'Later today' is upcoming, so Start once at is the default.
    expect(screen.getByText('Old timer')).toBeInTheDocument();
    expect(screen.getByText('Later today')).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Today' }));
    expect(screen.queryByText('Old timer')).not.toBeInTheDocument();
    expect(screen.getByText('Later today')).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'All time' }));
    expect(screen.getByText('Old timer')).toBeInTheDocument();
  });

  it('filters schedules by created_at and shows a period empty state', async () => {
    setData({
      schedules: [makeSchedule({ created_at: tenDaysAgo() })],
    });
    const user = userEvent.setup();
    render(<Sessions />);
    expect(screen.getByText('Morning block')).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Today' }));
    expect(screen.queryByText('Morning block')).not.toBeInTheDocument();
    expect(
      screen.getByText('No schedules in this period.'),
    ).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'All time' }));
    expect(screen.getByText('Morning block')).toBeInTheDocument();
  });

  it('filters allowances by created_at', async () => {
    setData({
      allowances: [makeAllowance({ created_at: tenDaysAgo() })],
    });
    const user = userEvent.setup();
    render(<Sessions />);
    expect(screen.getByText('YouTube budget')).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Today' }));
    expect(screen.queryByText('YouTube budget')).not.toBeInTheDocument();
    expect(
      screen.getByText('No allowances in this period.'),
    ).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'All time' }));
    expect(screen.getByText('YouTube budget')).toBeInTheDocument();
  });
});
