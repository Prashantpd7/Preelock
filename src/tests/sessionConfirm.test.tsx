import { describe, expect, it, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { SessionConfirmDialog } from '../components/SessionConfirmDialog';

function renderDialog(
  over: Partial<Parameters<typeof SessionConfirmDialog>[0]> = {},
) {
  const onStart = vi.fn();
  const onCancel = vi.fn();
  render(
    <SessionConfirmDialog
      title="Deep work"
      minutes={25}
      strict={false}
      busy={false}
      onStart={onStart}
      onCancel={onCancel}
      {...over}
    />,
  );
  return { onStart, onCancel };
}

describe('SessionConfirmDialog', () => {
  it('renders the session summary with Start and Cancel buttons', () => {
    renderDialog();
    expect(screen.getByText('Deep work')).toBeInTheDocument();
    expect(screen.getByText('25 min · Standard mode')).toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: 'Start session' }),
    ).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Cancel' })).toBeInTheDocument();
  });

  it('shows the strict warning only for strict sessions', () => {
    const { unmount } = render(
      <SessionConfirmDialog
        title="Deep work"
        minutes={60}
        strict
        busy={false}
        onStart={vi.fn()}
        onCancel={vi.fn()}
      />,
    );
    expect(screen.getByText('60 min · Strict mode')).toBeInTheDocument();
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

  it('starts the session when Start is clicked', async () => {
    const user = userEvent.setup();
    const { onStart, onCancel } = renderDialog();
    await user.click(screen.getByRole('button', { name: 'Start session' }));
    expect(onStart).toHaveBeenCalledTimes(1);
    expect(onCancel).not.toHaveBeenCalled();
  });

  it('discards without starting when Cancel is clicked', async () => {
    const user = userEvent.setup();
    const { onStart, onCancel } = renderDialog({ strict: true });
    await user.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(onCancel).toHaveBeenCalledTimes(1);
    expect(onStart).not.toHaveBeenCalled();
  });

  it('disables both buttons while busy', () => {
    renderDialog({ busy: true });
    expect(
      screen.getByRole('button', { name: 'Start session' }),
    ).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Cancel' })).toBeDisabled();
  });
});
