import { beforeEach, describe, expect, it } from 'vitest';
import { render, screen } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { AppLayout } from '../layouts/AppLayout';
import { useAppStore } from '../state/store';
import { emptySnapshot } from '../services/api';

beforeEach(() => {
  useAppStore.setState({
    data: structuredClone(emptySnapshot),
    loading: false,
    error: null,
    busy: false,
  });
});

describe('AppLayout navigation', () => {
  it('has no Activity nav item', () => {
    const { unmount } = render(
      <MemoryRouter initialEntries={['/']}>
        <AppLayout />
      </MemoryRouter>,
    );
    const nav = screen.getByRole('navigation', { name: 'Main navigation' });
    expect(nav).toBeInTheDocument();
    expect(
      screen.queryByRole('link', { name: 'Activity' }),
    ).not.toBeInTheDocument();
    // The remaining items are untouched.
    for (const label of [
      'Dashboard',
      'Focus Sessions',
      'Tasks',
      'Analytics',
      'Screen Time',
      'Settings',
    ]) {
      expect(screen.getByRole('link', { name: label })).toBeInTheDocument();
    }
    unmount();
  });

  it('renders the power button in the sidebar and toggles power on click', async () => {
    let executedAction: unknown = null;
    useAppStore.setState({
      execute: async (action) => {
        executedAction = action;
        return true;
      },
    });

    const { unmount } = render(
      <MemoryRouter initialEntries={['/']}>
        <AppLayout />
      </MemoryRouter>,
    );

    const button = screen.getByRole('button', { name: 'Turn Preelock Off' });
    expect(button).toBeInTheDocument();
    expect(button).toHaveClass('is-on');
    expect(screen.getByText('Preelock On')).toBeInTheDocument();

    button.click();
    expect(executedAction).toEqual({ type: 'toggle_power' });
    unmount();
  });

  it('applies dimmed and inert state when preelock is powered off', () => {
    useAppStore.setState({
      data: {
        ...structuredClone(emptySnapshot),
        preferences: {
          theme: 'dark',
          default_minutes: 25,
          paused: true,
        },
      },
    });

    const { container, unmount } = render(
      <MemoryRouter initialEntries={['/']}>
        <AppLayout />
      </MemoryRouter>,
    );

    const shell = container.querySelector('.app-shell');
    expect(shell).toHaveClass('preelock-powered-off');

    const button = screen.getByRole('button', { name: 'Turn Preelock On' });
    expect(button).toBeInTheDocument();
    expect(button).toHaveClass('is-off');
    expect(screen.getByText('Preelock Off')).toBeInTheDocument();

    const sidebarTop = container.querySelector('.sidebar-top');
    const mainShell = container.querySelector('.main-shell');
    expect(sidebarTop).toHaveAttribute('inert');
    expect(mainShell).toHaveAttribute('inert');

    unmount();
  });
});
