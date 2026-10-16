import { useCallback, useEffect, useReducer, useRef, useState } from 'react';
import { getVersion } from '@tauri-apps/api/app';
import { Channel } from '@tauri-apps/api/core';
import { version as packageVersion } from '../../package.json';
import { desktopApi } from '../api';
import { isBoolean, oneOf, readStored, writeStored } from '../lib/storage';
import {
  canInstall,
  initialUpdaterState,
  isBusy,
  isCheckDue,
  releasePageUrl,
  retryStage,
  shouldAutoDownload,
  updaterReducer,
  type UpdaterState,
} from '../lib/updates';
import { errorMessage, type DownloadEvent, type UpdateInfo } from '../types';

export const UPDATE_INTERVALS = [0, 1, 6, 24] as const;
export type UpdateInterval = (typeof UPDATE_INTERVALS)[number];

export type CheckOutcome =
  | { kind: 'found'; update: UpdateInfo }
  | { kind: 'current' }
  | { kind: 'failed'; message: string }
  | { kind: 'busy' };

/** Lets the network come up when Windows starts the app at login. */
const STARTUP_DELAY = 10_000;
const EVALUATE_EVERY = 5 * 60_000;

const isTimestamp = (value: unknown): value is number | null => value === null || (typeof value === 'number' && Number.isFinite(value));

/**
 * Drives the Rust updater: check, background download with progress and verified install.
 * Automatic checks follow the chosen interval; with "Baixar atualizações automaticamente" on,
 * a found update downloads right away and waits for "Reiniciar para atualizar".
 */
export function useUpdater() {
  const [currentVersion, setCurrentVersion] = useState(packageVersion);
  const [interval, setIntervalState] = useState<UpdateInterval>(() =>
    readStored<UpdateInterval>('update.interval', 6, oneOf(UPDATE_INTERVALS)),
  );
  const [autoDownload, setAutoDownloadState] = useState(() => readStored('update.autoDownload', true, isBoolean));
  const [state, dispatch] = useReducer(updaterReducer, undefined, () =>
    initialUpdaterState(readStored<number | null>('update.checkedAt', null, isTimestamp)),
  );
  const stateRef = useRef<UpdaterState>(state);
  const autoDownloadRef = useRef(autoDownload);
  const checking = useRef<Promise<CheckOutcome> | null>(null);
  const downloading = useRef<Promise<boolean> | null>(null);
  const installing = useRef(false);
  const checkedThisSession = useRef(false);
  stateRef.current = state;
  autoDownloadRef.current = autoDownload;

  useEffect(() => {
    getVersion()
      .then(setCurrentVersion)
      .catch(() => undefined);
  }, []);

  /** Resolves true once the installer is downloaded and its signature verified. */
  const download = useCallback((): Promise<boolean> => {
    if (downloading.current) return downloading.current;
    if (!stateRef.current.update) return Promise.resolve(false);
    if (stateRef.current.update.ready) return Promise.resolve(true);
    dispatch({ type: 'downloadStarted' });
    const channel = new Channel<DownloadEvent>();
    channel.onmessage = event => dispatch({ type: 'downloadEvent', event });
    const pending = desktopApi
      .updateDownload(channel)
      .then(() => {
        dispatch({ type: 'downloaded' });
        return true;
      })
      .catch(reason => {
        dispatch({ type: 'downloadFailed', message: errorMessage(reason) });
        return false;
      })
      .finally(() => {
        downloading.current = null;
      });
    downloading.current = pending;
    return pending;
  }, []);

  // A manual check during an automatic one shares its answer.
  const check = useCallback((): Promise<CheckOutcome> => {
    if (checking.current) return checking.current;
    if (isBusy(stateRef.current)) return Promise.resolve({ kind: 'busy' });
    checkedThisSession.current = true;
    dispatch({ type: 'check' });
    const pending = desktopApi
      .updateCheck()
      .then(
        (update): CheckOutcome => {
          const at = Date.now();
          writeStored('update.checkedAt', at);
          dispatch({ type: 'checked', update, at });
          if (!update) return { kind: 'current' };
          if (shouldAutoDownload(update, autoDownloadRef.current)) {
            // The reducer applies "checked" before the download reads the state.
            stateRef.current = updaterReducer(stateRef.current, { type: 'checked', update, at });
            void download();
          }
          return { kind: 'found', update };
        },
        (reason): CheckOutcome => {
          const at = Date.now();
          const message = errorMessage(reason);
          writeStored('update.checkedAt', at);
          dispatch({ type: 'checkFailed', message, at });
          return { kind: 'failed', message };
        },
      )
      .finally(() => {
        checking.current = null;
      });
    checking.current = pending;
    return pending;
  }, [download]);

  /** Starts the verified installer. On success the app exits; the installer reopens it. */
  const install = useCallback(async () => {
    if (installing.current || !canInstall(stateRef.current)) return;
    installing.current = true;
    dispatch({ type: 'install' });
    try {
      await desktopApi.updateInstall();
    } catch (reason) {
      dispatch({ type: 'installFailed', message: errorMessage(reason) });
    } finally {
      installing.current = false;
    }
  }, []);

  /** "Baixar e instalar": downloads when needed, then restarts into the new version. */
  const downloadAndInstall = useCallback(async () => {
    if (await download()) {
      stateRef.current = updaterReducer(stateRef.current, { type: 'downloaded' });
      await install();
    }
  }, [download, install]);

  const retry = useCallback(() => {
    const stage = retryStage(stateRef.current);
    if (stage === 'install') return install();
    if (stage === 'download') return download().then(() => undefined);
    return check().then(() => undefined);
  }, [check, download, install]);

  // Automatic checks: once shortly after start, then on the chosen interval.
  useEffect(() => {
    if (interval === 0) return;
    const evaluate = () => {
      if (isCheckDue(stateRef.current, interval, Date.now(), !checkedThisSession.current)) void check();
    };
    const first = window.setTimeout(evaluate, checkedThisSession.current ? 0 : STARTUP_DELAY);
    const timer = window.setInterval(evaluate, EVALUATE_EVERY);
    return () => {
      window.clearTimeout(first);
      window.clearInterval(timer);
    };
  }, [interval, check]);

  const setInterval = useCallback((next: UpdateInterval) => {
    writeStored('update.interval', next);
    setIntervalState(next);
  }, []);

  const setAutoDownload = useCallback(
    (next: boolean) => {
      writeStored('update.autoDownload', next);
      setAutoDownloadState(next);
      autoDownloadRef.current = next;
      if (next && stateRef.current.phase === 'available') void download();
    },
    [download],
  );

  const releasePage = releasePageUrl(state.update?.version);
  const openReleasePage = useCallback(() => desktopApi.openUrl(releasePage), [releasePage]);

  return {
    currentVersion,
    interval,
    setInterval,
    autoDownload,
    setAutoDownload,
    state,
    check,
    download,
    install,
    downloadAndInstall,
    retry,
    releasePage,
    openReleasePage,
  };
}

export type Updater = ReturnType<typeof useUpdater>;
