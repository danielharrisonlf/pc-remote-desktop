import { useEffect, useState } from 'react';

/** Current time, re-rendering every `interval` ms while `active`. */
export function useNow(interval = 1000, active = true): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!active) return;
    setNow(Date.now());
    const timer = window.setInterval(() => setNow(Date.now()), interval);
    return () => window.clearInterval(timer);
  }, [interval, active]);
  return now;
}
