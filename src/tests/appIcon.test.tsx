import { describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';

const invokeMock = vi.hoisted(() => vi.fn());
vi.mock('@tauri-apps/api/core', () => ({
  invoke: invokeMock,
  isTauri: () => true,
}));

import { RowIcon } from '../components/AppIcon';

describe('RowIcon', () => {
  it('shows the site favicon and falls back to a letter tile on error', () => {
    const { container } = render(
      <RowIcon kind="site" target="example.com" name="Example" />,
    );
    const img = container.querySelector('img.screen-time-icon');
    expect(img?.getAttribute('src')).toBe('https://example.com/favicon.ico');
    fireEvent.error(img!);
    expect(screen.getByText('E')).toBeInTheDocument();
    expect(container.querySelector('img')).toBeNull();
  });

  it('shows the letter tile when the backend returns no app icon', async () => {
    invokeMock.mockResolvedValue(null);
    const { container } = render(
      <RowIcon kind="app" target="com.unknown" name="Unknown" />,
    );
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('get_app_icon', {
        identity: 'com.unknown',
      }),
    );
    expect(container.querySelector('img')).toBeNull();
    expect(screen.getByText('U')).toBeInTheDocument();
  });

  it('shows the backend icon when one resolves', async () => {
    invokeMock.mockResolvedValue('AAA');
    const { container } = render(
      <RowIcon kind="app" target="com.brave.Browser" name="Brave" />,
    );
    await waitFor(() =>
      expect(
        container.querySelector('img.screen-time-icon')?.getAttribute('src'),
      ).toBe('data:image/png;base64,AAA'),
    );
  });
});
