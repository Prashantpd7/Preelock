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
});
