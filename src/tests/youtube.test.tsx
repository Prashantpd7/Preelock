import { beforeEach, expect, it, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { App } from '../App';
import { api, emptySnapshot } from '../services/api';
import { useAppStore } from '../state/store';
import { defaultFocus, type Snapshot } from '../types';
vi.mock('../services/settings', () => ({
  settingsAction: vi.fn().mockResolvedValue({}),
  preferenceDefaults: {
    theme: 'dark',
    default_minutes: 25,
    saved_setups: [],
    reminder_days: [],
  },
}));
vi.mock('../services/api', () => ({
  desktop: true,
  emptySnapshot: {
    sessions: [],
    tasks: [],
    preferences: { theme: 'dark', default_minutes: 25 },
  },
  api: {
    snapshot: vi.fn(),
    execute: vi.fn(),
    focusSetup: vi.fn(),
    capabilities: vi.fn(),
    blockAttempts: vi.fn().mockResolvedValue([]),
  },
}));
const connectedSetup = {
  platform: 'macos',
  native_available: true,
  applications: [{ id: 'com.microsoft.VSCode', name: 'VS Code' }],
  browsers: [{ id: 'ai.perplexity.comet', name: 'Comet' }],
  camera_permission: 'not_requested',
  pairing_token: 'x'.repeat(64),
  browser: {
    available: true,
    connected: true,
    client_browser: 'ai.perplexity.comet',
    applied_id: '',
    error: '',
    blocks: 0,
    cleanup_pending: false,
    extension_stale: false,
  },
};
beforeEach(() => {
  localStorage.setItem('preelock-permissions-v2', 'seen');
  window.location.hash = '#/sessions';
  vi.clearAllMocks();
  useAppStore.setState({
    data: structuredClone(emptySnapshot),
    loading: false,
    error: null,
    busy: false,
  });
  vi.mocked(api.snapshot).mockResolvedValue(structuredClone(emptySnapshot));
  vi.mocked(api.capabilities).mockResolvedValue([]);
  vi.mocked(api.focusSetup).mockResolvedValue(connectedSetup);
});
async function openBlocklistWizard(user: ReturnType<typeof userEvent.setup>) {
  render(<App />);
  await screen.findByRole('heading', { name: 'Focus Sessions' });
  await user.click(screen.getByRole('button', { name: 'New session' }));
  await user.type(screen.getByLabelText('Task name'), 'Study physics');
  await user.click(screen.getByRole('button', { name: 'Continue' }));
  await user.click(screen.getByRole('checkbox', { name: 'VS Code' }));
  await user.click(
    screen.getByRole('checkbox', { name: 'Block distracting websites' }),
  );
}
it('saves study mode, allowed channels and Watch Later with the session', async () => {
  const user = userEvent.setup();
  await openBlocklistWizard(user);
  await user.click(
    screen.getByRole('checkbox', {
      name: 'Block all except these',
    }),
  );
  await user.type(screen.getByLabelText('Allowed channels'), '@physics');
  await user.click(
    screen.getByRole('checkbox', {
      name: 'Allow Watch Later playlist even when YouTube is blocked',
    }),
  );
  await user.click(screen.getByRole('button', { name: 'Continue' }));
  vi.mocked(api.execute).mockResolvedValue(emptySnapshot);
  await user.click(screen.getByRole('button', { name: 'Create session' }));
  // Confirmation popup: the session is only created once started here.
  await user.click(screen.getByRole('button', { name: 'Start session' }));
  expect(api.execute).toHaveBeenCalledWith(
    expect.objectContaining({
      type: 'create_session',
      focus: expect.objectContaining({
        website_blocking: 'blocklist',
        youtube_blocked_channels: [],
        youtube_allowed_channels: ['@physics'],
        youtube_study_mode: true,
        youtube_allow_watch_later: true,
      }),
    }),
  );
});
it('saves blocked channels in block mode, even with no domain list', async () => {
  const user = userEvent.setup();
  await openBlocklistWizard(user);
  await user.click(
    screen.getByRole('checkbox', { name: 'Block these channels' }),
  );
  await user.type(screen.getByLabelText('Channels to block'), '@distracting');
  await user.click(screen.getByRole('button', { name: 'Continue' }));
  vi.mocked(api.execute).mockResolvedValue(emptySnapshot);
  await user.click(screen.getByRole('button', { name: 'Create session' }));
  // Confirmation popup: the session is only created once started here.
  await user.click(screen.getByRole('button', { name: 'Start session' }));
  expect(api.execute).toHaveBeenCalledWith(
    expect.objectContaining({
      type: 'create_session',
      focus: expect.objectContaining({
        youtube_blocked_channels: ['@distracting'],
        youtube_study_mode: false,
        youtube_allow_watch_later: false,
      }),
    }),
  );
});
it('keeps the two YouTube options mutually exclusive and rejects bad channels', async () => {
  const user = userEvent.setup();
  await openBlocklistWizard(user);
  // Ticking an option reveals its textarea; the other stays hidden.
  await user.click(
    screen.getByRole('checkbox', { name: 'Block these channels' }),
  );
  expect(screen.getByLabelText('Channels to block')).toBeInTheDocument();
  expect(screen.queryByLabelText('Allowed channels')).not.toBeInTheDocument();
  // Ticking study mode unticks the block option and swaps the textarea.
  await user.click(
    screen.getByRole('checkbox', {
      name: 'Block all except these',
    }),
  );
  expect(
    screen.getByRole('checkbox', { name: 'Block these channels' }),
  ).not.toBeChecked();
  expect(screen.queryByLabelText('Channels to block')).not.toBeInTheDocument();
  await user.type(screen.getByLabelText('Allowed channels'), 'not a channel');
  await user.click(screen.getByRole('button', { name: 'Continue' }));
  expect(
    await screen.findByText('Invalid YouTube channel: not a channel'),
  ).toBeInTheDocument();
  expect(api.execute).not.toHaveBeenCalled();
});
it('restores saved YouTube rules when a saved setup is applied', async () => {
  const savedData: Snapshot = {
    ...structuredClone(emptySnapshot),
    preferences: {
      theme: 'dark',
      default_minutes: 25,
      saved_setups: [
        {
          id: 'p1',
          name: 'Study',
          minutes: 25,
          focus: {
            ...defaultFocus,
            browser_app: 'ai.perplexity.comet',
            website_blocking: 'blocklist',
            domains: [],
            youtube_blocked_channels: ['@distracting'],
            youtube_allowed_channels: ['@physics'],
            youtube_study_mode: true,
            youtube_allow_watch_later: true,
          },
        },
      ],
    },
  };
  vi.mocked(api.snapshot).mockResolvedValue(structuredClone(savedData));
  useAppStore.setState({ data: structuredClone(savedData) });
  const user = userEvent.setup();
  render(<App />);
  await screen.findByRole('heading', { name: 'Focus Sessions' });
  await user.click(screen.getByRole('button', { name: /Study/ }));
  await user.click(screen.getByRole('button', { name: 'Continue' }));
  // Study mode wins: the block option is unticked and its textarea hidden,
  // but the saved block text is kept and restored when re-ticked.
  expect(
    screen.getByRole('checkbox', { name: 'Block these channels' }),
  ).not.toBeChecked();
  expect(screen.queryByLabelText('Channels to block')).not.toBeInTheDocument();
  await user.click(
    screen.getByRole('checkbox', { name: 'Block these channels' }),
  );
  expect(screen.getByLabelText('Channels to block')).toHaveValue(
    '@distracting',
  );
  await user.click(
    screen.getByRole('checkbox', {
      name: 'Block all except these',
    }),
  );
  expect(screen.getByLabelText('Allowed channels')).toHaveValue('@physics');
  expect(
    screen.getByRole('checkbox', {
      name: 'Block all except these',
    }),
  ).toBeChecked();
  expect(
    screen.getByRole('checkbox', {
      name: 'Allow Watch Later playlist even when YouTube is blocked',
    }),
  ).toBeChecked();
});
it('hides the YouTube subsection when website blocking is off', async () => {
  const user = userEvent.setup();
  render(<App />);
  await screen.findByRole('heading', { name: 'Focus Sessions' });
  await user.click(screen.getByRole('button', { name: 'New session' }));
  await user.type(screen.getByLabelText('Task name'), 'Write notes');
  await user.click(screen.getByRole('button', { name: 'Continue' }));
  expect(screen.queryByLabelText('Channels to block')).not.toBeInTheDocument();
  expect(screen.queryByLabelText('Allowed channels')).not.toBeInTheDocument();
});
