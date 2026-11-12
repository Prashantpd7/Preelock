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
async function openAppsStep(
  user: ReturnType<typeof userEvent.setup>,
  strict = false,
) {
  render(<App />);
  await screen.findByRole('heading', { name: 'Focus Sessions' });
  await user.click(screen.getByRole('button', { name: 'New session' }));
  await user.type(screen.getByLabelText('Task name'), 'Study physics');
  if (strict) {
    await user.click(screen.getByRole('radio', { name: /Strict/i }));
  }
  await user.click(screen.getByRole('button', { name: 'Continue' }));
  await user.click(screen.getByRole('checkbox', { name: 'VS Code' }));
}

it('keeps Website blocking and YouTube blocking unchecked by default in Standard and Strict modes', async () => {
  const user = userEvent.setup();
  await openAppsStep(user, false);
  expect(
    screen.getByRole('checkbox', { name: 'Website blocking' }),
  ).not.toBeChecked();
  expect(
    screen.getByRole('checkbox', { name: 'YouTube blocking' }),
  ).not.toBeChecked();
  expect(
    screen.queryByRole('checkbox', { name: 'Block distracting websites' }),
  ).not.toBeInTheDocument();
  expect(
    screen.queryByRole('checkbox', { name: 'Block all websites except these' }),
  ).not.toBeInTheDocument();
  expect(
    screen.queryByRole('checkbox', { name: 'Block these channels' }),
  ).not.toBeInTheDocument();
  expect(
    screen.queryByRole('checkbox', { name: 'Block all except these' }),
  ).not.toBeInTheDocument();
  expect(
    screen.queryByRole('checkbox', { name: 'Block all entertainment content' }),
  ).not.toBeInTheDocument();
  expect(
    screen.queryByRole('checkbox', {
      name: /Allow Watch Later playlist/i,
    }),
  ).not.toBeInTheDocument();

  await user.click(screen.getByRole('button', { name: 'Back' }));
  await user.click(screen.getByRole('radio', { name: /Strict/i }));
  await user.click(screen.getByRole('button', { name: 'Continue' }));
  expect(
    screen.getByRole('checkbox', { name: 'Website blocking' }),
  ).not.toBeChecked();
  expect(
    screen.getByRole('checkbox', { name: 'YouTube blocking' }),
  ).not.toBeChecked();
});

it('shows Block these channels and Block all entertainment content under YouTube blocking independently of Website blocking', async () => {
  const user = userEvent.setup();
  await openAppsStep(user);
  await user.click(screen.getByRole('checkbox', { name: 'YouTube blocking' }));
  expect(
    screen.getByRole('checkbox', { name: 'Block these channels' }),
  ).toBeInTheDocument();
  expect(
    screen.getByRole('checkbox', { name: 'Block all entertainment content' }),
  ).toBeInTheDocument();
  expect(
    screen.queryByRole('checkbox', { name: 'Block all except these' }),
  ).not.toBeInTheDocument();
  expect(
    screen.queryByRole('checkbox', {
      name: 'Allow Watch Later playlist when YouTube is blocked',
    }),
  ).not.toBeInTheDocument();

  await user.click(
    screen.getByRole('checkbox', { name: 'Block these channels' }),
  );
  await user.type(screen.getByLabelText('Channels to block'), '@distracting');
  await user.click(
    screen.getByRole('checkbox', { name: 'Block all entertainment content' }),
  );
  await user.click(screen.getByRole('button', { name: 'Continue' }));
  vi.mocked(api.execute).mockResolvedValue(emptySnapshot);
  await user.click(screen.getByRole('button', { name: 'Create session' }));
  await user.click(screen.getByRole('button', { name: 'Start session' }));
  expect(api.execute).toHaveBeenCalledWith(
    expect.objectContaining({
      type: 'create_session',
      focus: expect.objectContaining({
        website_blocking: 'none',
        youtube_blocked_channels: ['@distracting'],
        youtube_allowed_channels: [],
        youtube_study_mode: false,
        youtube_block_entertainment: true,
        youtube_allow_watch_later: false,
      }),
    }),
  );
});

it('shows Allow Watch Later playlist dynamically at the bottom of Website blocking when typing rules block YouTube', async () => {
  const user = userEvent.setup();
  await openAppsStep(user);
  await user.click(screen.getByRole('checkbox', { name: 'Website blocking' }));
  expect(
    screen.getByRole('checkbox', { name: 'Block distracting websites' }),
  ).toBeInTheDocument();
  expect(
    screen.getByRole('checkbox', { name: 'Block all websites except these' }),
  ).toBeInTheDocument();

  // Blocklist: Watch Later is hidden until user enters YouTube.
  await user.click(
    screen.getByRole('checkbox', { name: 'Block distracting websites' }),
  );
  expect(
    screen.queryByRole('checkbox', {
      name: 'Allow Watch Later playlist when YouTube is blocked',
    }),
  ).not.toBeInTheDocument();

  const blockInput = screen.getByLabelText('Websites to block');
  await user.type(blockInput, 'instagram.com');
  expect(
    screen.queryByRole('checkbox', {
      name: 'Allow Watch Later playlist when YouTube is blocked',
    }),
  ).not.toBeInTheDocument();

  await user.type(blockInput, '{enter}youtube.com');
  const watchLater = screen.getByRole('checkbox', {
    name: 'Allow Watch Later playlist when YouTube is blocked',
  });
  expect(watchLater).toBeInTheDocument();
  await user.click(watchLater);
  expect(watchLater).toBeChecked();
  await user.click(watchLater);
  expect(watchLater).not.toBeChecked();

  // Removing youtube.com hides Watch Later immediately; switch to allowlist shows it
  // when YouTube is omitted, and hides it as soon as youtube.com is kept open.
  await user.clear(blockInput);
  await user.type(blockInput, 'reddit.com');
  expect(
    screen.queryByRole('checkbox', {
      name: 'Allow Watch Later playlist when YouTube is blocked',
    }),
  ).not.toBeInTheDocument();

  await user.click(
    screen.getByRole('checkbox', { name: 'Block all websites except these' }),
  );
  expect(
    screen.getByRole('checkbox', {
      name: 'Allow Watch Later playlist when YouTube is blocked',
    }),
  ).toBeInTheDocument();

  const allowInput = screen.getByLabelText('Websites to keep open');
  await user.type(allowInput, '{enter}youtube.com');
  expect(
    screen.queryByRole('checkbox', {
      name: 'Allow Watch Later playlist when YouTube is blocked',
    }),
  ).not.toBeInTheDocument();

  await user.clear(allowInput);
  await user.type(allowInput, 'docs.python.org');
  const watchLaterAllowlist = screen.getByRole('checkbox', {
    name: 'Allow Watch Later playlist when YouTube is blocked',
  });
  await user.click(watchLaterAllowlist);
  await user.click(screen.getByRole('button', { name: 'Continue' }));
  vi.mocked(api.execute).mockResolvedValue(emptySnapshot);
  await user.click(screen.getByRole('button', { name: 'Create session' }));
  await user.click(screen.getByRole('button', { name: 'Start session' }));
  expect(api.execute).toHaveBeenCalledWith(
    expect.objectContaining({
      type: 'create_session',
      focus: expect.objectContaining({
        website_blocking: 'allowlist',
        domains: ['docs.python.org'],
        youtube_allow_watch_later: true,
      }),
    }),
  );
});

it('rejects bad channels under Block these channels', async () => {
  const user = userEvent.setup();
  await openAppsStep(user);
  await user.click(screen.getByRole('checkbox', { name: 'YouTube blocking' }));
  await user.click(
    screen.getByRole('checkbox', { name: 'Block these channels' }),
  );
  await user.type(screen.getByLabelText('Channels to block'), 'not a channel');
  await user.click(screen.getByRole('button', { name: 'Continue' }));
  expect(
    await screen.findByText('Invalid YouTube channel: not a channel'),
  ).toBeInTheDocument();
  expect(api.execute).not.toHaveBeenCalled();
});

it('restores saved YouTube and Website blocking rules when a saved setup is applied', async () => {
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
            domains: ['youtube.com'],
            youtube_blocked_channels: ['@distracting'],
            youtube_block_entertainment: true,
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
  expect(
    screen.getByRole('checkbox', { name: 'Website blocking' }),
  ).toBeChecked();
  expect(
    screen.getByRole('checkbox', { name: 'Block distracting websites' }),
  ).toBeChecked();
  expect(screen.getByLabelText('Websites to block')).toHaveValue('youtube.com');
  expect(
    screen.getByRole('checkbox', {
      name: 'Allow Watch Later playlist when YouTube is blocked',
    }),
  ).toBeChecked();
  expect(
    screen.getByRole('checkbox', { name: 'YouTube blocking' }),
  ).toBeChecked();
  expect(
    screen.getByRole('checkbox', { name: 'Block these channels' }),
  ).toBeChecked();
  expect(screen.getByLabelText('Channels to block')).toHaveValue(
    '@distracting',
  );
  expect(
    screen.getByRole('checkbox', { name: 'Block all entertainment content' }),
  ).toBeChecked();
});

it('ignores inactive channel input and hidden YouTube/Website controls when turned off', async () => {
  const user = userEvent.setup();
  await openAppsStep(user);
  await user.click(screen.getByRole('checkbox', { name: 'Website blocking' }));
  await user.click(
    screen.getByRole('checkbox', { name: 'Block distracting websites' }),
  );
  await user.type(screen.getByLabelText('Websites to block'), 'youtube.com');
  await user.click(
    screen.getByRole('checkbox', {
      name: 'Allow Watch Later playlist when YouTube is blocked',
    }),
  );
  await user.click(screen.getByRole('checkbox', { name: 'YouTube blocking' }));
  await user.click(
    screen.getByRole('checkbox', { name: 'Block these channels' }),
  );
  await user.type(
    screen.getByLabelText('Channels to block'),
    'invalid entry!!',
  );
  await user.click(
    screen.getByRole('checkbox', { name: 'Block all entertainment content' }),
  );

  // Untick both Website blocking and YouTube blocking -> hidden invalid
  // channel input must not block Continue and hidden rules must not submit.
  await user.click(screen.getByRole('checkbox', { name: 'Website blocking' }));
  await user.click(screen.getByRole('checkbox', { name: 'YouTube blocking' }));
  await user.click(screen.getByRole('button', { name: 'Continue' }));
  expect(
    screen.getByRole('heading', { name: 'Ready to focus?' }),
  ).toBeInTheDocument();

  vi.mocked(api.execute).mockResolvedValue(emptySnapshot);
  await user.click(screen.getByRole('button', { name: 'Create session' }));
  await user.click(screen.getByRole('button', { name: 'Start session' }));
  expect(api.execute).toHaveBeenCalledWith(
    expect.objectContaining({
      type: 'create_session',
      focus: expect.objectContaining({
        website_blocking: 'none',
        domains: [],
        youtube_blocked_channels: [],
        youtube_allowed_channels: [],
        youtube_study_mode: false,
        youtube_block_entertainment: false,
        youtube_allow_watch_later: false,
      }),
    }),
  );
});
