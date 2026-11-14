import { beforeEach, describe, expect, it, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { UnconnectedBrowserBanner } from '../components/UnconnectedBrowserBanner';
import { AppLayout } from '../layouts/AppLayout';
import { useAppStore } from '../state/store';
import { MemoryRouter } from 'react-router-dom';
import { emptySnapshot } from '../services/api';

vi.mock('../services/api', () => ({
  desktop: true,
  emptySnapshot: {
    sessions: [],
    tasks: [],
    preferences: { theme: 'dark', default_minutes: 25 },
    unconnected_browser: null,
  },
  api: {
    snapshot: vi.fn().mockResolvedValue({
      sessions: [],
      tasks: [],
      preferences: { theme: 'dark', default_minutes: 25 },
      unconnected_browser: null,
    }),
    focusSetup: vi.fn().mockResolvedValue({
      platform: 'macos',
      native_available: true,
      applications: [],
      camera_permission: 'granted',
      pairing_token:
        'PAIR1234567890ABCDEF1234567890ABCDEF1234567890ABCDEF1234567890ABCDEF',
      browser: {
        available: true,
        connected: false,
        applied_id: '',
        error: '',
        blocks: 0,
        cleanup_pending: false,
        extension_stale: false,
      },
      connected_browsers: [],
      browsers: [
        { id: 'com.brave.Browser', name: 'Brave' },
        { id: 'com.google.Chrome', name: 'Google Chrome' },
        { id: 'com.microsoft.edgemac', name: 'Microsoft Edge' },
      ],
    }),
    openSetup: vi.fn(),
    resetPairing: vi.fn(),
    capabilities: vi.fn().mockResolvedValue([]),
    blockAttempts: vi.fn().mockResolvedValue([]),
    screenTimeSummary: vi.fn().mockResolvedValue([]),
    desktopSettings: vi.fn().mockResolvedValue({
      login_launch_supported: false,
      login_launch_enabled: false,
      login_user_pref: false,
      battery_saver_active: false,
      notifications: 2,
    }),
  },
}));

vi.mock('../services/settings', () => ({
  settingsAction: vi.fn().mockResolvedValue({}),
}));

describe('UnconnectedBrowserBanner Component', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useAppStore.setState({
      data: {
        ...emptySnapshot,
        unconnected_browser: null,
      },
      loading: false,
      error: null,
    });
  });

  it('renders nothing when there is no unconnected browser warning', () => {
    render(<UnconnectedBrowserBanner />);
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
  });

  it('renders a red warning banner with correct text when Brave is unconnected', () => {
    useAppStore.setState({
      data: {
        ...emptySnapshot,
        unconnected_browser: {
          browser_id: 'com.brave.Browser',
          browser_name: 'Brave',
        },
      },
    });

    render(<UnconnectedBrowserBanner />);
    expect(screen.getByRole('alert')).toBeInTheDocument();
    expect(
      screen.getByText(/Brave is not connected to Preelock/i),
    ).toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: /Connect/i }),
    ).toBeInTheDocument();
  });

  it('renders a red warning banner with correct text when Chrome is unconnected', () => {
    useAppStore.setState({
      data: {
        ...emptySnapshot,
        unconnected_browser: {
          browser_id: 'com.google.Chrome',
          browser_name: 'Google Chrome',
        },
      },
    });

    render(<UnconnectedBrowserBanner />);
    expect(screen.getByRole('alert')).toBeInTheDocument();
    expect(
      screen.getByText(/Google Chrome is not connected to Preelock/i),
    ).toBeInTheDocument();
  });

  it('renders a red warning banner with correct text when Edge is unconnected', () => {
    useAppStore.setState({
      data: {
        ...emptySnapshot,
        unconnected_browser: {
          browser_id: 'com.microsoft.edgemac',
          browser_name: 'Microsoft Edge',
        },
      },
    });

    render(<UnconnectedBrowserBanner />);
    expect(screen.getByRole('alert')).toBeInTheDocument();
    expect(
      screen.getByText(/Microsoft Edge is not connected to Preelock/i),
    ).toBeInTheDocument();
  });

  it('clicking Connect button opens the modal preselected for that browser', async () => {
    useAppStore.setState({
      data: {
        ...emptySnapshot,
        unconnected_browser: {
          browser_id: 'com.brave.Browser',
          browser_name: 'Brave',
        },
      },
    });

    render(<UnconnectedBrowserBanner />);
    const user = userEvent.setup();
    const connectButton = screen.getByRole('button', { name: /Connect/i });
    await user.click(connectButton);

    expect(screen.getByText('Connect Brave')).toBeInTheDocument();
  });

  it('banner disappears when unconnected_browser is cleared after successful connection', () => {
    const { rerender } = render(<UnconnectedBrowserBanner />);
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();

    useAppStore.setState({
      data: {
        ...emptySnapshot,
        unconnected_browser: {
          browser_id: 'com.brave.Browser',
          browser_name: 'Brave',
        },
      },
    });
    rerender(<UnconnectedBrowserBanner />);
    expect(screen.getByRole('alert')).toBeInTheDocument();

    useAppStore.setState({
      data: {
        ...emptySnapshot,
        unconnected_browser: null,
      },
    });
    rerender(<UnconnectedBrowserBanner />);
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
  });

  it('banner updates dynamically when switching from one unconnected browser to another', () => {
    useAppStore.setState({
      data: {
        ...emptySnapshot,
        unconnected_browser: {
          browser_id: 'com.brave.Browser',
          browser_name: 'Brave',
        },
      },
    });
    const { rerender } = render(<UnconnectedBrowserBanner />);
    expect(screen.getByText(/Brave is not connected/i)).toBeInTheDocument();

    useAppStore.setState({
      data: {
        ...emptySnapshot,
        unconnected_browser: {
          browser_id: 'com.microsoft.edgemac',
          browser_name: 'Microsoft Edge',
        },
      },
    });
    rerender(<UnconnectedBrowserBanner />);
    expect(
      screen.queryByText(/Brave is not connected/i),
    ).not.toBeInTheDocument();
    expect(
      screen.getByText(/Microsoft Edge is not connected/i),
    ).toBeInTheDocument();
  });
});

describe('AppLayout Global Banner Integration', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useAppStore.setState({
      data: {
        ...emptySnapshot,
        unconnected_browser: null,
      },
      loading: false,
      error: null,
    });
  });

  it('renders the unconnected browser banner at the top of AppLayout across routes', () => {
    useAppStore.setState({
      data: {
        ...emptySnapshot,
        unconnected_browser: {
          browser_id: 'com.brave.Browser',
          browser_name: 'Brave',
        },
      },
    });

    render(
      <MemoryRouter initialEntries={['/']}>
        <AppLayout />
      </MemoryRouter>,
    );

    expect(screen.getByRole('alert')).toBeInTheDocument();
    expect(screen.getByText(/Brave is not connected/i)).toBeInTheDocument();
  });

  it('stacks cleanly with active session banner without collision', () => {
    useAppStore.setState({
      data: {
        ...emptySnapshot,
        sessions: [
          {
            id: 'sess-1',
            title: 'Deep Work Test',
            started_at: Date.now() - 60000,
            duration_minutes: 25,
            status: 'running',
            remaining_ms: 24 * 60 * 1000,
            focused_ms: 0,
            allow_pause: true,
            ended_at: null,
            deadline_ms: null,
            focus: {
              mode: 'standard',
              apps: [],
              domains: [],
              exceptions: [],
              blocked_folders: [],
              blocked_titles: [],
              youtube_blocked_channels: [],
              youtube_allowed_channels: [],
              youtube_study_mode: false,
              youtube_block_entertainment: false,
              youtube_allow_watch_later: false,
              block_adult_content: false,
              website_blocking: 'auto',
              general_internet: true,
              browser_app: '',
              activity_consent: true,
              camera: false,
              frozen: { enabled: false },
            },
          },
        ],
        unconnected_browser: {
          browser_id: 'com.brave.Browser',
          browser_name: 'Brave',
        },
      },
    });

    render(
      <MemoryRouter initialEntries={['/sessions']}>
        <AppLayout />
      </MemoryRouter>,
    );

    expect(screen.getByText(/Brave is not connected/i)).toBeInTheDocument();
    expect(screen.getByText(/Deep Work Test/i)).toBeInTheDocument();
  });
});
