import { describe, expect, it } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { SessionReview } from '../components/SessionReview';
import type { SessionReviewProps } from '../components/SessionReview';
import { defaultFocus, type FocusConfig } from '../types';

const NOW = new Date('2026-11-11T10:00:00').getTime();

function makeFocus(over: Partial<FocusConfig> = {}): FocusConfig {
  return {
    ...defaultFocus,
    ...over,
    attention: {
      ...defaultFocus.attention!,
      ...(over.attention ?? {}),
    },
    pomodoro: over.pomodoro ?? defaultFocus.pomodoro,
    reward: over.reward ?? defaultFocus.reward,
    frozen: over.frozen ?? defaultFocus.frozen,
  };
}

function renderReview(
  over: Partial<SessionReviewProps> = {},
  focusOver: Partial<FocusConfig> = {},
) {
  return render(
    <SessionReview
      focus={makeFocus(focusOver)}
      title="Deep work"
      timing={{ kind: 'immediate', minutes: 25 }}
      nowMs={NOW}
      {...over}
    />,
  );
}

describe('SessionReview', () => {
  it('shows the overview, apps, websites, YouTube and browser for a strict session', () => {
    renderReview(
      {
        appNames: { 'com.example.focus-app': 'Focus App' },
        browserName: 'Chrome',
        browserConnected: true,
        cameraPermission: 'granted',
        platform: 'darwin',
      },
      {
        mode: 'strict',
        apps: ['com.example.focus-app'],
        domains: ['youtube.com', 'instagram.com'],
        website_blocking: 'blocklist',
        youtube_blocked_channels: ['@mkbhd'],
        youtube_allow_watch_later: true,
        browser_app: 'com.google.Chrome',
      },
    );
    expect(screen.getByText('Deep work')).toBeInTheDocument();
    expect(screen.getByText('Focus session')).toBeInTheDocument();
    expect(screen.getByText('25 minutes')).toBeInTheDocument();
    expect(screen.getByText('Immediately')).toBeInTheDocument();
    // End = 10:00 + 25 min, in the local format.
    expect(screen.getByText(/10:25/)).toBeInTheDocument();
    expect(screen.getByText('Strict')).toBeInTheDocument();
    expect(screen.getByText('Focus App')).toBeInTheDocument();
    expect(screen.getByText('Allowed')).toBeInTheDocument();
    expect(screen.getByText('youtube.com')).toBeInTheDocument();
    expect(screen.getByText('instagram.com')).toBeInTheDocument();
    expect(screen.getByText('@mkbhd')).toBeInTheDocument();
    expect(screen.getByText('Watch Later: Allowed')).toBeInTheDocument();
    expect(screen.getByText('Chrome')).toBeInTheDocument();
    expect(screen.getByText('Connected')).toBeInTheDocument();
    expect(screen.getByText('Ready for this session.')).toBeInTheDocument();
    expect(
      screen.queryByText(/Camera access is not granted/),
    ).not.toBeInTheDocument();
    expect(screen.queryByText(/Safari/)).not.toBeInTheDocument();
  });

  it('shows allowlist mode and that every other website is blocked', () => {
    renderReview(
      {},
      { domains: ['github.com'], website_blocking: 'allowlist' },
    );
    expect(
      screen.getByText('Block all websites except these'),
    ).toBeInTheDocument();
    expect(screen.getByText('github.com')).toBeInTheDocument();
    expect(
      screen.getByText('Every other website is blocked.'),
    ).toBeInTheDocument();
  });

  it('shows a compact Website blocking Off status when blocking is off', () => {
    renderReview({}, { website_blocking: 'none' });
    expect(screen.getByText('Website blocking: Off')).toBeInTheDocument();
    expect(
      screen.queryByText('Block distracting websites'),
    ).not.toBeInTheDocument();
  });

  it('shows study mode with allowed channels and Watch Later off', () => {
    renderReview(
      {},
      {
        youtube_study_mode: true,
        youtube_allowed_channels: ['@khanacademy'],
        youtube_allow_watch_later: false,
      },
    );
    expect(
      screen.getByText('Only the listed channels are allowed.'),
    ).toBeInTheDocument();
    expect(screen.getByText('@khanacademy')).toBeInTheDocument();
    expect(screen.getByText('Watch Later: Off')).toBeInTheDocument();
  });

  it('omits the YouTube section entirely when nothing is configured', () => {
    renderReview();
    expect(screen.queryByText('YouTube restrictions')).not.toBeInTheDocument();
  });

  it('shows entertainment blocking summary when youtube_block_entertainment is on', () => {
    renderReview({}, { youtube_block_entertainment: true });
    expect(
      screen.getByText(
        'Entertainment content (comedy, pranks, vlogs, funny/animal clips) is blocked.',
      ),
    ).toBeInTheDocument();
  });

  it('warns when camera is required but permission is not granted', () => {
    renderReview(
      { cameraPermission: 'denied', platform: 'darwin' },
      { camera: true },
    );
    expect(
      screen.getByText(/Camera access is not granted/),
    ).toBeInTheDocument();
  });

  it('does not warn when camera is required and permission is granted', () => {
    renderReview(
      { cameraPermission: 'granted', platform: 'darwin' },
      { camera: true },
    );
    expect(screen.getByText(/checks you're at your desk/)).toBeInTheDocument();
    expect(
      screen.queryByText(/Camera access is not granted/),
    ).not.toBeInTheDocument();
  });

  it('shows Camera Off and timer-only when monitoring is off', () => {
    renderReview(
      { platform: 'darwin' },
      {
        camera: false,
        attention: { ...defaultFocus.attention!, enabled: false },
      },
    );
    expect(screen.getByText('Off')).toBeInTheDocument();
    expect(
      screen.getByText('Off — timer only, no focus checks.'),
    ).toBeInTheDocument();
  });

  it('shows focus checks and type of work when attention is on', () => {
    renderReview(
      { platform: 'darwin' },
      {
        attention: {
          ...defaultFocus.attention!,
          enabled: true,
          verification: 'camera_and_activity',
          task: 'lecture',
          grace_seconds: 10,
        },
        camera: true,
      },
    );
    expect(
      screen.getByText(/Checks your apps and that you're at your desk/),
    ).toBeInTheDocument();
    expect(
      screen.getByText(/Type of work: Watching a lesson/),
    ).toBeInTheDocument();
    expect(
      screen.queryByText(/Wait before labeling a distraction/),
    ).not.toBeInTheDocument();
  });

  it('shows Pomodoro only when enabled', () => {
    const { unmount } = renderReview(
      {},
      {
        pomodoro: {
          enabled: true,
          focus_minutes: 25,
          short_break_minutes: 5,
          long_break_minutes: 15,
          cycles_before_long_break: 4,
        },
      },
    );
    expect(screen.getByText(/25m focus · 5m short break/)).toBeInTheDocument();
    unmount();
    renderReview();
    expect(screen.queryByText('Pomodoro')).not.toBeInTheDocument();
  });

  it('shows locks only when a lock summary is passed', () => {
    const { unmount } = renderReview({ lockText: 'password · delay 5 min' });
    expect(screen.getByText('password · delay 5 min')).toBeInTheDocument();
    unmount();
    renderReview({ lockText: null });
    expect(screen.queryByText('Locks')).not.toBeInTheDocument();
  });

  it('collapses long website lists behind a Show all toggle', async () => {
    const user = userEvent.setup();
    const domains = Array.from({ length: 12 }, (_, i) => `site${i}.com`);
    renderReview({}, { domains, website_blocking: 'blocklist' });
    expect(screen.getByText('site0.com')).toBeInTheDocument();
    expect(screen.getByText('site7.com')).toBeInTheDocument();
    expect(screen.queryByText('site8.com')).not.toBeInTheDocument();
    await user.click(
      screen.getByRole('button', { name: 'Show all 12 websites' }),
    );
    expect(screen.getByText('site11.com')).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Show less' }));
    expect(screen.queryByText('site8.com')).not.toBeInTheDocument();
  });

  it('shows schedule days, window and overnight end', () => {
    render(
      <SessionReview
        focus={makeFocus({})}
        title="Morning"
        timing={{
          kind: 'schedule',
          minutes: 60,
          days: [1, 2, 3, 4, 5],
          startTime: '09:00',
          endTime: '10:00',
        }}
        nowMs={NOW}
      />,
    );
    expect(screen.getByText('Scheduled session')).toBeInTheDocument();
    expect(screen.getByText('Weekdays · 09:00')).toBeInTheDocument();
    expect(screen.getByText('10:00')).toBeInTheDocument();
    expect(screen.getByText('60 minutes')).toBeInTheDocument();
  });

  it('shows an overnight schedule window and next-day end', () => {
    render(
      <SessionReview
        focus={makeFocus({})}
        title="Night"
        timing={{
          kind: 'schedule',
          minutes: 60,
          days: [1, 2, 3, 4, 5],
          startTime: '22:00',
          endTime: '06:00',
        }}
        nowMs={NOW}
      />,
    );
    expect(screen.getByText('480 minutes')).toBeInTheDocument();
    expect(screen.getByText('06:00 (next day)')).toBeInTheDocument();
  });

  it('shows one-shot start and end, or that no time is picked yet', () => {
    const fireAtMs = new Date('2099-01-01T09:00').getTime();
    const { unmount } = render(
      <SessionReview
        focus={makeFocus({})}
        title="Evening"
        timing={{ kind: 'oneshot', minutes: 30, fireAtMs }}
        nowMs={NOW}
      />,
    );
    expect(screen.getByText('One-shot session')).toBeInTheDocument();
    // Start and end are both in 2099.
    expect(screen.getAllByText(/2099/)).toHaveLength(2);
    expect(screen.getByText(/9:30/)).toBeInTheDocument();
    unmount();
    render(
      <SessionReview
        focus={makeFocus({})}
        title="Evening"
        timing={{ kind: 'oneshot', minutes: 30, fireAtMs: null }}
        nowMs={NOW}
      />,
    );
    expect(screen.getByText('Pick a date and time')).toBeInTheDocument();
  });

  it('shows the offline note when all internet is blocked', () => {
    renderReview({}, { general_internet: false });
    expect(
      screen.getByText('All internet access is blocked during this session.'),
    ).toBeInTheDocument();
  });

  it('omits the browser section when no browser is needed or selected', () => {
    renderReview({}, { browser_app: '' });
    expect(screen.queryByText('Browser')).not.toBeInTheDocument();
  });

  describe('Windows', () => {
    it('never shows the Safari warning on Windows', () => {
      renderReview(
        {
          platform: 'windows',
          safariWarning: true,
          browserName: 'Chrome',
          browserConnected: true,
        },
        { browser_app: 'com.google.Chrome' },
      );
      expect(screen.queryByText(/Safari/)).not.toBeInTheDocument();
    });

    it('never warns about camera permission on Windows', () => {
      renderReview(
        { platform: 'windows', cameraPermission: 'denied' },
        { camera: true },
      );
      expect(
        screen.queryByText(/Camera access is not granted/),
      ).not.toBeInTheDocument();
      // Camera checks do not exist on Windows: no Camera row at all.
      expect(screen.queryByText('Camera')).not.toBeInTheDocument();
    });

    it('evaluates browser readiness on Windows like any other platform', () => {
      const { unmount } = render(
        <SessionReview
          focus={makeFocus({
            domains: ['youtube.com'],
            website_blocking: 'blocklist',
            browser_app: 'chrome.exe',
          })}
          title="Deep work"
          timing={{ kind: 'immediate', minutes: 25 }}
          browserName="Chrome"
          browserConnected={false}
          platform="windows"
          nowMs={NOW}
        />,
      );
      expect(screen.getByText('Not connected')).toBeInTheDocument();
      expect(
        screen.getByText(/Browser extension is not connected/),
      ).toBeInTheDocument();
      unmount();
      render(
        <SessionReview
          focus={makeFocus({
            domains: ['youtube.com'],
            website_blocking: 'blocklist',
            browser_app: 'chrome.exe',
          })}
          title="Deep work"
          timing={{ kind: 'immediate', minutes: 25 }}
          browserName="Chrome"
          browserConnected
          platform="windows"
          nowMs={NOW}
        />,
      );
      expect(screen.getByText('Connected')).toBeInTheDocument();
      expect(screen.getByText('Ready for this session.')).toBeInTheDocument();
    });

    it('shows the Safari warning on macOS when Safari is selected', () => {
      renderReview(
        {
          platform: 'darwin',
          safariWarning: true,
          browserName: 'Safari',
          browserConnected: false,
        },
        {
          domains: ['youtube.com'],
          website_blocking: 'blocklist',
          browser_app: 'com.apple.Safari',
        },
      );
      expect(
        screen.getByText(/Safari has no Preelock extension/),
      ).toBeInTheDocument();
    });
  });
});
