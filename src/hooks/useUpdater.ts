import { useCallback, useEffect, useRef, useState } from 'react';
import { getVersion } from '@tauri-apps/api/app';
import { version as packageVersion } from '../../package.json';
import { desktopApi } from '../api';
import { fetchLatestRelease, pickInstaller, type ReleaseInfo, type ReleaseLookup } from '../lib/releases';
import { isNewerVersion } from '../lib/semver';
import { isString, oneOf, readStored, writeStored } from '../lib/storage';

export const UPDATE_REPOSITORY = 'danielharrisonlf/pc-remote-desktop';
export const UPDATE_INTERVALS = [0, 1, 6, 24] as const;
export type UpdateInterval = (typeof UPDATE_INTERVALS)[number];

interface CheckResult {
  checkedAt: number;
  lookup: ReleaseLookup;
}

const HOUR = 3_600_000;
/** A restart within this window reuses the cached answer (GitHub allows 60 calls/h per IP). */
const STARTUP_MIN_AGE = 30 * 60_000;
const EVALUATE_EVERY = 5 * 60_000;

function isCheckResult(value: unknown): value is CheckResult {
  if (!value || typeof value !== 'object') return false;
  const { checkedAt, lookup } = value as Partial<CheckResult>;
  if (typeof checkedAt !== 'number' || !lookup || typeof lookup !== 'object') return false;
  if (lookup.kind === 'none') return true;
  if (lookup.kind === 'unavailable') return typeof lookup.message === 'string';
  if (lookup.kind !== 'found') return false;
  const release = lookup.release as Partial<ReleaseInfo> | undefined;
  return (
    !!release &&
    typeof release.version === 'string' &&
    typeof release.tag === 'string' &&
    typeof release.notes === 'string' &&
    typeof release.pageUrl === 'string' &&
    release.pageUrl.startsWith('https://github.com/') &&
    Array.isArray(release.assets)
  );
}

export function useUpdater() {
  const [currentVersion, setCurrentVersion] = useState(packageVersion);
  const [interval, setIntervalState] = useState<UpdateInterval>(() =>
    readStored<UpdateInterval>('update.interval', 6, oneOf(UPDATE_INTERVALS)),
  );
  const [skippedVersion, setSkippedVersion] = useState(() => readStored('update.skipped', '', isString));
  const [result, setResult] = useState<CheckResult | null>(() => readStored<CheckResult | null>('update.last', null, isCheckResult));
  const [checking, setChecking] = useState(false);
  const resultRef = useRef(result);
  const controller = useRef<AbortController | null>(null);
  const inFlight = useRef<Promise<ReleaseLookup | null> | null>(null);
  const startupPending = useRef(true);
  resultRef.current = result;

  useEffect(() => {
    getVersion()
      .then(setCurrentVersion)
      .catch(() => undefined);
    return () => controller.current?.abort();
  }, []);

  // A manual check during an automatic one shares its answer instead of
  // sending a second request.
  const check = useCallback((): Promise<ReleaseLookup | null> => {
    if (inFlight.current) return inFlight.current;
    const active = new AbortController();
    controller.current = active;
    setChecking(true);
    inFlight.current = fetchLatestRelease(UPDATE_REPOSITORY, { signal: active.signal })
      .then(lookup => {
        const next = { checkedAt: Date.now(), lookup };
        writeStored('update.last', next);
        setResult(next);
        return lookup;
      })
      .catch(() => null)
      .finally(() => {
        controller.current = null;
        inFlight.current = null;
        if (!active.signal.aborted) setChecking(false);
      });
    return inFlight.current;
  }, []);

  useEffect(() => {
    if (interval === 0) return;
    const evaluate = () => {
      const now = Date.now();
      const last = resultRef.current;
      if (last?.lookup.kind === 'unavailable' && last.lookup.retryAt && now < last.lookup.retryAt) return;
      const threshold = startupPending.current ? STARTUP_MIN_AGE : interval * HOUR;
      startupPending.current = false;
      if (now - (last?.checkedAt ?? 0) >= threshold) void check();
    };
    evaluate();
    const timer = window.setInterval(evaluate, EVALUATE_EVERY);
    return () => window.clearInterval(timer);
  }, [interval, check]);

  const setInterval = useCallback((next: UpdateInterval) => {
    writeStored('update.interval', next);
    setIntervalState(next);
  }, []);

  const skip = useCallback((version: string) => {
    writeStored('update.skipped', version);
    setSkippedVersion(version);
  }, []);

  const release = result?.lookup.kind === 'found' ? result.lookup.release : null;
  const available = Boolean(release && isNewerVersion(release.version, currentVersion));
  const installer = release ? pickInstaller(release.assets) : undefined;

  const download = useCallback(() => {
    if (!release) return Promise.resolve();
    return desktopApi.openUrl(installer?.downloadUrl ?? release.pageUrl);
  }, [installer, release]);

  const openReleasePage = useCallback(() => {
    return release ? desktopApi.openUrl(release.pageUrl) : Promise.resolve();
  }, [release]);

  return {
    currentVersion,
    interval,
    setInterval,
    checking,
    checkedAt: result?.checkedAt ?? null,
    lookup: result?.lookup ?? null,
    release,
    installer,
    available,
    /** Available and not dismissed by "Ignorar esta versão". */
    notify: available && release?.version !== skippedVersion,
    skip,
    check,
    download,
    openReleasePage,
  };
}

export type Updater = ReturnType<typeof useUpdater>;
