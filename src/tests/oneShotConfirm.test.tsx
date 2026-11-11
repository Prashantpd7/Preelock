import { describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { OneShotConfirmDialog } from '../components/OneShotConfirmDialog';

function renderDialog(
  over: Partial<Parameters<typeof OneShotConfirmDialog>[0]> = {},
) {
  const onCreate = vi.fn();
  const onCancel = vi.fn();
  const { unmount } = render(
    <OneShotConfirmDialog
      title="Deep work"
      minutes={25}
      strict={false}
      busy={false}
      onCreate={onCreate}
      onCancel={onCancel}
      {...over}
    />,
  );
  return { onCreate, onCancel, unmount };
}

describe('OneShotConfirmDialog', () => {
  it('renders the summary, prefilled name and start time field', () => {
    renderDialog();
    expect(screen.getByText('25 min · Standard mode')).toBeInTheDocument();
    expect(screen.getByLabelText('One-shot name')).toHaveValue('Deep work');
    expect(screen.getByLabelText('Start at')).toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: 'Create one-shot' }),
    ).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Cancel' })).toBeInTheDocument();
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

  it('rejects a past time without creating', async () => {
    const user = userEvent.setup();
    const { onCreate } = renderDialog();
    fireEvent.change(screen.getByLabelText('Start at'), {
      target: { value: '2020-01-01T09:00' },
    });
    await user.click(screen.getByRole('button', { name: 'Create one-shot' }));
    expect(
      await screen.findByText('Pick a future date and time.'),
    ).toBeInTheDocument();
    expect(onCreate).not.toHaveBeenCalled();
  });

  it('creates the one-shot at a future time', async () => {
    const user = userEvent.setup();
    const { onCreate } = renderDialog();
    fireEvent.change(screen.getByLabelText('Start at'), {
      target: { value: '2099-01-01T09:00' },
    });
    await user.click(screen.getByRole('button', { name: 'Create one-shot' }));
    expect(onCreate).toHaveBeenCalledTimes(1);
    expect(onCreate).toHaveBeenCalledWith({
      title: 'Deep work',
      fire_at_ms: new Date('2099-01-01T09:00').getTime(),
    });
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
      screen.getByRole('button', { name: 'Create one-shot' }),
    ).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Cancel' })).toBeDisabled();
  });
});
