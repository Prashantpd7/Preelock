import { describe, expect, it, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { ScheduleConfirmDialog } from '../components/ScheduleConfirmDialog';
import { defaultFocus } from '../types';

const reviewFocus = {
  ...defaultFocus,
  apps: ['com.example.focus-app'],
  domains: ['youtube.com'],
  website_blocking: 'blocklist' as const,
  browser_app: 'com.google.Chrome',
};

function renderDialog(
  over: Partial<Parameters<typeof ScheduleConfirmDialog>[0]> = {},
) {
  const onCreate = vi.fn();
  const onCancel = vi.fn();
  const { unmount } = render(
    <ScheduleConfirmDialog
      title="Deep work"
      minutes={25}
      strict={false}
      busy={false}
      focus={reviewFocus}
      appNames={{ 'com.example.focus-app': 'Focus App' }}
      browserName="Chrome"
      browserConnected
      cameraPermission="granted"
      platform="darwin"
      onCreate={onCreate}
      onCancel={onCancel}
      {...over}
    />,
  );
  return { onCreate, onCancel, unmount };
}

describe('ScheduleConfirmDialog', () => {
  it('renders the summary, prefilled name, weekdays and default times', () => {
    renderDialog();
    expect(screen.getByText('25 min · Standard mode')).toBeInTheDocument();
    expect(screen.getByLabelText('Schedule name')).toHaveValue('Deep work');
    for (const day of ['Mon', 'Tue', 'Wed', 'Thu', 'Fri']) {
      expect(screen.getByRole('checkbox', { name: day })).toBeChecked();
    }
    expect(screen.getByRole('checkbox', { name: 'Sun' })).not.toBeChecked();
    expect(screen.getByRole('checkbox', { name: 'Sat' })).not.toBeChecked();
    expect(screen.getByLabelText('Start time')).toHaveValue('09:00');
    expect(screen.getByLabelText('End time')).toHaveValue('10:00');
    expect(
      screen.getByRole('button', { name: 'Create schedule' }),
    ).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Cancel' })).toBeInTheDocument();
  });

  it('shows the session review with the picked days and times', () => {
    renderDialog();
    expect(screen.getByText('Scheduled session')).toBeInTheDocument();
    expect(screen.getByText('Weekdays · 09:00')).toBeInTheDocument();
    expect(screen.getByText('Focus App')).toBeInTheDocument();
    expect(screen.getByText('youtube.com')).toBeInTheDocument();
  });

  it('updates the review days live when a day is unchecked', async () => {
    const user = userEvent.setup();
    renderDialog();
    expect(screen.getByText('Weekdays · 09:00')).toBeInTheDocument();
    await user.click(screen.getByRole('checkbox', { name: 'Tue' }));
    expect(screen.getByText('Mon, Wed, Thu, Fri · 09:00')).toBeInTheDocument();
    expect(screen.queryByText('Weekdays · 09:00')).not.toBeInTheDocument();
  });

  it('shows the picked lock in the review', async () => {
    const user = userEvent.setup();
    renderDialog();
    expect(screen.queryByText('Locks')).not.toBeInTheDocument();
    await user.click(
      screen.getByRole('checkbox', { name: 'Lock with a password' }),
    );
    await user.type(screen.getByLabelText('Lock password'), 'secret');
    expect(screen.getByText('password')).toBeInTheDocument();
  });

  it('shows the strict warning only for strict sessions', () => {
    const { unmount } = renderDialog({ strict: true });
    expect(
      screen.getByText(
        /Strict mode will block everything except your work apps/,
      ),
    ).toBeInTheDocument();
    unmount();
    renderDialog({ strict: false });
    expect(
      screen.queryByText(
        /Strict mode will block everything except your work apps/,
      ),
    ).not.toBeInTheDocument();
  });

  it('creates the schedule with the picked days and times', async () => {
    const user = userEvent.setup();
    const { onCreate } = renderDialog();
    await user.click(screen.getByRole('checkbox', { name: 'Tue' }));
    await user.click(screen.getByRole('button', { name: 'Create schedule' }));
    expect(onCreate).toHaveBeenCalledTimes(1);
    expect(onCreate).toHaveBeenCalledWith({
      title: 'Deep work',
      days: [1, 3, 4, 5],
      start_time: '09:00',
      end_time: '10:00',
      enabled: true,
      lock_changes: false,
    });
  });

  it('rejects an empty day selection without creating', async () => {
    const user = userEvent.setup();
    const { onCreate } = renderDialog();
    for (const day of ['Mon', 'Tue', 'Wed', 'Thu', 'Fri']) {
      await user.click(screen.getByRole('checkbox', { name: day }));
    }
    await user.click(screen.getByRole('button', { name: 'Create schedule' }));
    expect(
      await screen.findByText('Pick at least one day.'),
    ).toBeInTheDocument();
    expect(onCreate).not.toHaveBeenCalled();
  });

  it('rejects a password lock with no password', async () => {
    const user = userEvent.setup();
    const { onCreate } = renderDialog();
    await user.click(
      screen.getByRole('checkbox', { name: 'Lock with a password' }),
    );
    await user.click(screen.getByRole('button', { name: 'Create schedule' }));
    expect(
      await screen.findByText('Enter a password for the password lock.'),
    ).toBeInTheDocument();
    expect(onCreate).not.toHaveBeenCalled();
  });

  it('includes the lock when one is set', async () => {
    const user = userEvent.setup();
    const { onCreate } = renderDialog();
    await user.click(
      screen.getByRole('checkbox', { name: 'Lock with a password' }),
    );
    await user.type(screen.getByLabelText('Lock password'), 'secret');
    await user.click(screen.getByRole('button', { name: 'Create schedule' }));
    expect(onCreate).toHaveBeenCalledWith(
      expect.objectContaining({
        lock: expect.objectContaining({ challenge_length: 0 }),
        lock_password: 'secret',
      }),
    );
  });

  it('cancels without creating', async () => {
    const user = userEvent.setup();
    const { onCreate, onCancel } = renderDialog();
    await user.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(onCancel).toHaveBeenCalledTimes(1);
    expect(onCreate).not.toHaveBeenCalled();
  });

  it('disables both buttons while busy', () => {
    renderDialog({ busy: true });
    expect(
      screen.getByRole('button', { name: 'Create schedule' }),
    ).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Cancel' })).toBeDisabled();
  });
});
