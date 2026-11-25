import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { api } from '../services/api';
import { useAppStore } from '../state/store';

// Reproduces the 2026-11-24 banner storm: every poll timed out and every
// timeout created a fresh red toast, forever.
describe('engine-down banner', () => {
  beforeEach(() => {
    useAppStore.setState({
      error: null,
      dismissedError: null,
      isEngineDown: false,
      loading: false,
    });
  });
  afterEach(() => vi.restoreAllMocks());

  it('turns a snapshot timeout into the banner, not a toast', async () => {
    vi.spyOn(api, 'snapshot').mockRejectedValue(
      new Error('Snapshot timed out after 10s'),
    );
    await useAppStore.getState().refresh();
    const state = useAppStore.getState();
    expect(state.isEngineDown).toBe(true);
    expect(state.error).toBeNull();
  });

  it('suppresses other failures while the banner is showing', async () => {
    useAppStore.setState({ isEngineDown: true });
    vi.spyOn(api, 'snapshot').mockRejectedValue(new Error('Disk is full.'));
    await useAppStore.getState().refresh();
    expect(useAppStore.getState().error).toBeNull();
  });

  it('shows an ordinary failure once and keeps it dismissed', async () => {
    vi.spyOn(api, 'snapshot').mockRejectedValue(new Error('Disk is full.'));
    await useAppStore.getState().refresh();
    expect(useAppStore.getState().error).toBe('Disk is full.');
    useAppStore.getState().clearError();
    await useAppStore.getState().refresh();
    const state = useAppStore.getState();
    expect(state.error).toBeNull();
    expect(state.dismissedError).toBe('Disk is full.');
  });

  it('clears the banner when a snapshot succeeds again', async () => {
    useAppStore.setState({ isEngineDown: true, error: null });
    vi.spyOn(api, 'snapshot').mockResolvedValue(useAppStore.getState().data);
    await useAppStore.getState().refresh();
    expect(useAppStore.getState().isEngineDown).toBe(false);
  });
});
