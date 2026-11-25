import { create } from 'zustand';
import { api, emptySnapshot } from '../services/api';
import type { Action, Capability, Snapshot } from '../types';
interface Store {
  data: Snapshot;
  capabilities: Capability[];
  loading: boolean;
  busy: boolean;
  error: string | null;
  /** The message the user dismissed. A repeat of the SAME failure is
   *  not shown again; only a different failure may create a new toast. */
  dismissedError: string | null;
  isEngineDown: boolean;
  /** A session the user just started, shown instantly in the banner
   *  while the backend finishes starting it (DNS proxy setup can take
   *  seconds). Cleared once the backend snapshot reports the live
   *  session, or when starting fails. Without this the banner only
   *  appears 2–5s after the user clicks Start. */
  startingSession: { title: string; minutes: number } | null;
  setStartingSession: (s: { title: string; minutes: number } | null) => void;
  refresh: () => Promise<void>;
  execute: (action: Action) => Promise<boolean>;
  initialize: () => Promise<void>;
  clearError: () => void;
  restartEngine: () => Promise<boolean>;
}
function message(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}
// Serialize reads and writes so a late polling response cannot overwrite a mutation.
let pending: Promise<unknown> = Promise.resolve();
function queue<T>(work: () => Promise<T>): Promise<T> {
  const result = pending.then(work, work);
  pending = result.catch(() => undefined);
  return result;
}
/** Whether a backend failure means the engine is unreachable. A
 *  snapshot timeout belongs here too: the engine being too slow to
 *  answer is the same outage as a refused connection, and it must
 *  produce the one persistent banner instead of a new toast every
 *  10 seconds. */
function engineDownMessage(msg: string) {
  return (
    msg.includes('Background engine unavailable') ||
    msg.includes('Connection refused') ||
    msg.includes('No such file or directory') ||
    msg.includes('broken pipe') ||
    msg.includes('IPC') ||
    msg.includes('spawn cooldown') ||
    msg.includes('timed out')
  );
}
// One poll at a time. While the engine is down each snapshot takes the
// full 10 s timeout; a 1 s poll interval would otherwise queue dozens of
// doomed requests and delay recovery.
let pollInFlight = false;
export const useAppStore = create<Store>((set, get) => ({
  data: emptySnapshot,
  capabilities: [],
  loading: true,
  busy: false,
  error: null,
  dismissedError: null,
  isEngineDown: false,
  startingSession: null,
  setStartingSession: (s) => set({ startingSession: s }),
  clearError: () =>
    set((state) => ({ error: null, dismissedError: state.error })),
  initialize: async () => {
    await get().refresh();
    try {
      set({ capabilities: await api.capabilities() });
    } catch (error) {
      set({ error: message(error) });
    }
  },
  refresh: () => {
    if (pollInFlight) return Promise.resolve();
    pollInFlight = true;
    return queue(async () => {
      try {
        const snap = await api.snapshot();
        if (snap) {
          set({
            data: snap,
            loading: false,
            isEngineDown: false,
            dismissedError: null,
          });
        } else {
          set({ loading: false, isEngineDown: false });
        }
      } catch (error) {
        const msg = message(error);
        const engineDown = engineDownMessage(msg);
        set((state) => ({
          loading: false,
          isEngineDown: engineDown || state.isEngineDown,
          // While the engine-down banner is showing there is exactly one
          // status message; no per-poll error toast. A dismissed error
          // stays dismissed until a different failure occurs.
          // While the engine-down banner is showing, no other toast may
          // appear either: one persistent status is the whole point.
          error:
            engineDown || state.isEngineDown
              ? null
              : state.dismissedError === msg
                ? null
                : msg,
        }));
      }
    }).finally(() => {
      pollInFlight = false;
    });
  },
  restartEngine: async () => {
    try {
      await api.restartBackgroundEngine();
      await get().refresh();
      return true;
    } catch (error) {
      set({ error: message(error) });
      return false;
    }
  },
  execute: async (action) => {
    if (get().busy) return false;
    set({ busy: true, error: null });
    return queue(async () => {
      try {
        const snap = await api.execute(action);
        if (snap) {
          set({ data: snap });
        }
        return true;
      } catch (error) {
        const msg = message(error);
        const engineDown = engineDownMessage(msg);
        set((state) => ({
          isEngineDown: engineDown || state.isEngineDown,
          error:
            engineDown || state.isEngineDown
              ? null
              : state.dismissedError === msg
                ? null
                : msg,
        }));
        return false;
      } finally {
        set({ busy: false });
      }
    });
  },
}));
