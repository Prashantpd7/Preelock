import { create } from 'zustand';
import { api, emptySnapshot } from '../services/api';
import type { Action, Capability, Snapshot } from '../types';
interface Store {
  data: Snapshot;
  capabilities: Capability[];
  loading: boolean;
  busy: boolean;
  error: string | null;
  refresh: () => Promise<void>;
  execute: (action: Action) => Promise<boolean>;
  initialize: () => Promise<void>;
  clearError: () => void;
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
export const useAppStore = create<Store>((set, get) => ({
  data: emptySnapshot,
  capabilities: [],
  loading: true,
  busy: false,
  error: null,
  clearError: () => set({ error: null }),
  initialize: async () => {
    await get().refresh();
    try {
      set({ capabilities: await api.capabilities() });
    } catch (error) {
      set({ error: message(error) });
    }
  },
  refresh: () =>
    queue(async () => {
      try {
        set({ data: await api.snapshot(), loading: false });
      } catch (error) {
        set({ error: message(error), loading: false });
      }
    }),
  execute: async (action) => {
    if (get().busy) return false;
    set({ busy: true, error: null });
    return queue(async () => {
      try {
        set({ data: await api.execute(action) });
        return true;
      } catch (error) {
        set({ error: message(error) });
        return false;
      } finally {
        set({ busy: false });
      }
    });
  },
}));
