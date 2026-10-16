import { useCallback, useRef, useState } from 'react';
import { desktopApi } from '../api';
import { errorMessage, type DesktopStatus, type Settings } from '../types';
import { useToast } from './useToasts';

/**
 * Runs backend commands with per-action busy flags, refreshes the status after
 * each one and reports the outcome through toasts instead of inline banners.
 */
export function useActions(refresh: () => Promise<void>) {
  const toast = useToast();
  const [busy, setBusy] = useState<ReadonlySet<string>>(new Set());

  const run = useCallback(
    async (key: string, action: () => Promise<unknown>, success?: string): Promise<boolean> => {
      setBusy(current => new Set(current).add(key));
      try {
        await action();
        await refresh();
        if (success) toast.success(success);
        return true;
      } catch (reason) {
        toast.error(errorMessage(reason));
        return false;
      } finally {
        setBusy(current => {
          const next = new Set(current);
          next.delete(key);
          return next;
        });
      }
    },
    [refresh, toast],
  );

  const isBusy = useCallback((key: string) => busy.has(key), [busy]);
  return { run, isBusy, anyBusy: busy.size > 0 };
}

/**
 * The backend replaces the whole settings object on each update. Changes are
 * merged onto the newest local intent and sent one at a time, so two quick
 * toggles cannot overwrite each other with a stale snapshot.
 */
export function useSettings(status: DesktopStatus | null, refresh: () => Promise<void>) {
  const toast = useToast();
  const [optimistic, setOptimistic] = useState<Settings | null>(null);
  const intent = useRef<Settings | null>(null);
  const queue = useRef<Promise<void>>(Promise.resolve());
  const pending = useRef(0);
  const statusRef = useRef(status);
  statusRef.current = status;

  const change = useCallback(
    (patch: Partial<Settings>, success?: string) => {
      const base = intent.current ?? statusRef.current?.settings;
      if (!base) return;
      const next = { ...base, ...patch };
      intent.current = next;
      setOptimistic(next);
      pending.current += 1;
      queue.current = queue.current.then(async () => {
        try {
          await desktopApi.updateSettings(next);
          if (success) toast.success(success);
        } catch (reason) {
          toast.error(errorMessage(reason));
          // Later queued intents were built on top of this one; drop back to
          // the backend's truth once the queue drains.
          intent.current = null;
        } finally {
          pending.current -= 1;
          if (pending.current === 0) {
            await refresh();
            intent.current = null;
            setOptimistic(null);
          }
        }
      });
    },
    [refresh, toast],
  );

  return { settings: optimistic ?? status?.settings ?? null, change, saving: optimistic !== null };
}
