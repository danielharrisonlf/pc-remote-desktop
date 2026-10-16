import { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';
import { oneOf, readStored, writeStored } from '../lib/storage';

export type ThemePreference = 'system' | 'light' | 'dark';
export type ResolvedTheme = 'light' | 'dark';

const THEMES = ['system', 'light', 'dark'] as const;
const darkQuery = '(prefers-color-scheme: dark)';
const reducedMotionQuery = '(prefers-reduced-motion: reduce)';

export function readThemePreference(): ThemePreference {
  return readStored<ThemePreference>('theme', 'system', oneOf(THEMES));
}

/** Applies the theme attribute before React renders to avoid a mismatched first frame. */
export function applyInitialTheme(): void {
  const preference = readThemePreference();
  const dark = preference === 'dark' || (preference === 'system' && window.matchMedia?.(darkQuery).matches);
  document.documentElement.dataset.theme = dark ? 'dark' : 'light';
}

export function useTheme() {
  const [preference, setPreferenceState] = useState<ThemePreference>(readThemePreference);
  const [systemDark, setSystemDark] = useState(() => window.matchMedia?.(darkQuery).matches ?? false);
  const animate = useRef(false);
  const resolved: ResolvedTheme = preference === 'system' ? (systemDark ? 'dark' : 'light') : preference;

  useEffect(() => {
    const query = window.matchMedia?.(darkQuery);
    if (!query) return;
    const onChange = (event: MediaQueryListEvent) => setSystemDark(event.matches);
    query.addEventListener('change', onChange);
    return () => query.removeEventListener('change', onChange);
  }, []);

  useLayoutEffect(() => {
    const root = document.documentElement;
    // A short, uniform cross-fade on color properties only. Hover transitions
    // are suspended so individual controls do not flicker during the switch.
    const fade = animate.current && !window.matchMedia?.(reducedMotionQuery).matches;
    animate.current = false;
    if (fade) root.classList.add('theme-transition');
    root.dataset.theme = resolved;
    if (!fade) return;
    const timer = window.setTimeout(() => root.classList.remove('theme-transition'), 260);
    return () => {
      window.clearTimeout(timer);
      root.classList.remove('theme-transition');
    };
  }, [resolved]);

  const setPreference = useCallback((next: ThemePreference) => {
    animate.current = true;
    writeStored('theme', next);
    setPreferenceState(next);
  }, []);

  const toggle = useCallback(() => {
    setPreference(resolved === 'dark' ? 'light' : 'dark');
  }, [resolved, setPreference]);

  return { preference, resolved, setPreference, toggle };
}
