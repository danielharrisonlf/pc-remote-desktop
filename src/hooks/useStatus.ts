import { useCallback, useEffect, useRef, useState } from 'react';
import { desktopApi } from '../api';
import { errorMessage, type DesktopStatus } from '../types';

export type ServiceState = 'starting' | 'connected' | 'active' | 'unavailable';

const NORMAL_INTERVAL = 2000;
const FAST_INTERVAL = 1000;
const HIDDEN_INTERVAL = 10000;
/** One failed poll can be a busy backend; two in a row means the service is gone. */
const STALE_AFTER_FAILURES = 2;

/**
 * Polls the Rust service with a single chained timer (never overlapping calls).
 * The last good status is kept on failure so the UI does not blank or flicker;
 * consumers read `serviceState` to show that it may be outdated.
 */
export function useStatus(fast: boolean) {
  const [status, setStatus] = useState<DesktopStatus | null>(null);
  const [failures, setFailures] = useState(0);
  const [lastError, setLastError] = useState<string | null>(null);
  const sequence = useRef(0);
  const fastRef = useRef(fast);
  const wakeRef = useRef<() => void>(() => undefined);
  fastRef.current = fast;

  const refresh = useCallback(async () => {
    const id = ++sequence.current;
    try {
      const next = await desktopApi.getStatus();
      if (id !== sequence.current) return;
      setStatus(next);
      setFailures(0);
      setLastError(null);
    } catch (reason) {
      if (id !== sequence.current) return;
      setFailures(count => count + 1);
      setLastError(errorMessage(reason));
    }
  }, []);

  useEffect(() => {
    let disposed = false;
    let running = false;
    let timer = 0;
    const tick = async () => {
      if (disposed || running) return;
      running = true;
      window.clearTimeout(timer);
      await refresh();
      running = false;
      if (disposed) return;
      const delay = document.hidden ? HIDDEN_INTERVAL : fastRef.current ? FAST_INTERVAL : NORMAL_INTERVAL;
      timer = window.setTimeout(() => void tick(), delay);
    };
    const wake = () => {
      if (document.hidden) return;
      window.clearTimeout(timer);
      void tick();
    };
    wakeRef.current = wake;
    void tick();
    document.addEventListener('visibilitychange', wake);
    window.addEventListener('focus', wake);
    return () => {
      disposed = true;
      window.clearTimeout(timer);
      document.removeEventListener('visibilitychange', wake);
      window.removeEventListener('focus', wake);
    };
  }, [refresh]);

  useEffect(() => {
    if (fast) wakeRef.current();
  }, [fast]);

  const stale = failures >= STALE_AFTER_FAILURES;
  const serviceState: ServiceState =
    stale || status?.serviceRunning === false || (!status && failures > 0)
      ? 'unavailable'
      : !status
        ? 'starting'
        : status.devices.some(device => device.connected)
          ? 'connected'
          : 'active';

  return { status, serviceState, lastError: stale || !status ? lastError : null, refresh };
}
