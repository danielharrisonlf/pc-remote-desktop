import type { DownloadEvent, UpdateInfo } from '../types';
import { formatRelative } from './format';

// Update flow as seen by the panel. Rust owns the manifest, the download and the
// signature check; this module only tracks what to show and which action comes next.

export const RELEASES_URL = 'https://github.com/danielharrisonlf/pc-remote-desktop/releases';

export type UpdatePhase =
  | 'idle'
  | 'checking'
  | 'current'
  | 'available'
  | 'downloading'
  | 'verifying'
  | 'ready'
  | 'installing'
  | 'error';

export type UpdateStage = 'check' | 'download' | 'install';

export interface UpdaterState {
  phase: UpdatePhase;
  /** Newer version found by the latest successful check. */
  update: UpdateInfo | null;
  downloaded: number;
  total: number | null;
  error: { stage: UpdateStage; message: string } | null;
  /** Unix milliseconds of the latest check, successful or not. */
  checkedAt: number | null;
}

export type UpdaterAction =
  | { type: 'check' }
  | { type: 'checked'; update: UpdateInfo | null; at: number }
  | { type: 'checkFailed'; message: string; at: number }
  | { type: 'downloadStarted' }
  | { type: 'downloadEvent'; event: DownloadEvent }
  | { type: 'downloaded' }
  | { type: 'downloadFailed'; message: string }
  | { type: 'install' }
  | { type: 'installFailed'; message: string };

const HOUR = 3_600_000;
/** An automatic check that failed (for example, no network at boot) is retried sooner. */
export const RETRY_FAILED_CHECK_AFTER = 15 * 60_000;

export function initialUpdaterState(checkedAt: number | null = null): UpdaterState {
  return { phase: 'idle', update: null, downloaded: 0, total: null, error: null, checkedAt };
}

/** Download, verification and install own the flow until they finish. */
export function isBusy(state: UpdaterState): boolean {
  return state.phase === 'downloading' || state.phase === 'verifying' || state.phase === 'installing';
}

export function updaterReducer(state: UpdaterState, action: UpdaterAction): UpdaterState {
  switch (action.type) {
    case 'check':
      return isBusy(state) ? state : { ...state, phase: 'checking' };
    case 'checked': {
      const base = { ...state, checkedAt: action.at, error: null, downloaded: 0, total: null };
      if (!action.update) return { ...base, phase: 'current', update: null };
      return { ...base, phase: action.update.ready ? 'ready' : 'available', update: action.update };
    }
    case 'checkFailed':
      // Rust keeps a verified download when a later check fails, so "ready" stays valid.
      if (state.phase === 'ready') return { ...state, checkedAt: action.at };
      return { ...state, phase: 'error', checkedAt: action.at, error: { stage: 'check', message: action.message } };
    case 'downloadStarted':
      return state.update ? { ...state, phase: 'downloading', downloaded: 0, total: null, error: null } : state;
    case 'downloadEvent': {
      if (state.phase !== 'downloading') return state;
      const { event } = action;
      if (event.event === 'started') return { ...state, total: event.data.contentLength };
      if (event.event === 'progress') return { ...state, downloaded: event.data.downloaded, total: event.data.contentLength ?? state.total };
      return { ...state, phase: 'verifying', downloaded: state.total ?? state.downloaded };
    }
    case 'downloaded':
      return state.update ? { ...state, phase: 'ready', update: { ...state.update, ready: true }, error: null } : state;
    case 'downloadFailed':
      return { ...state, phase: 'error', downloaded: 0, total: null, error: { stage: 'download', message: action.message } };
    case 'install':
      return canInstall(state) ? { ...state, phase: 'installing', error: null } : state;
    case 'installFailed':
      return { ...state, phase: 'error', error: { stage: 'install', message: action.message } };
  }
}

/** With auto-download on, a check that finds a not-yet-downloaded update starts the download. */
export function shouldAutoDownload(update: UpdateInfo | null, autoDownload: boolean): boolean {
  return autoDownload && update !== null && !update.ready;
}

/** A verified download exists: either ready, or a failed install that can be retried. */
export function canInstall(state: UpdaterState): boolean {
  return Boolean(state.update?.ready) && (state.phase === 'ready' || (state.phase === 'error' && state.error?.stage === 'install'));
}

/** What "Tentar de novo" repeats. */
export function retryStage(state: UpdaterState): UpdateStage {
  if (state.error?.stage === 'install' && canInstall(state)) return 'install';
  return state.error?.stage === 'download' && state.update ? 'download' : 'check';
}

/** 0–1 while the size is known, null otherwise. */
export function downloadRatio(state: UpdaterState): number | null {
  if (state.phase === 'verifying') return 1;
  if (state.phase !== 'downloading' || !state.total) return null;
  return Math.min(1, Math.max(0, state.downloaded / state.total));
}

/**
 * Automatic checks: the first one after start always runs, since Rust keeps nothing across a
 * restart; later ones follow the interval, and a failed check is retried after 15 minutes.
 */
export function isCheckDue(state: UpdaterState, intervalHours: number, now: number, firstOfSession: boolean): boolean {
  if (intervalHours <= 0 || isBusy(state) || state.phase === 'checking' || state.phase === 'ready') return false;
  if (firstOfSession || state.checkedAt === null) return true;
  const wait = state.error?.stage === 'check' ? Math.min(RETRY_FAILED_CHECK_AFTER, intervalHours * HOUR) : intervalHours * HOUR;
  return now - state.checkedAt >= wait;
}

/** Links in release notes that the panel may open (the same releases `open_url` accepts). */
export function isReleaseLink(href: string): boolean {
  return /^https:\/\/github\.com\/danielharrisonlf\/pc-remote-(desktop|android)\/releases(\/[^\s@\\]*)?$/.test(href) && !href.includes('/../');
}

/** The release page for a version, or the release list; both pass the `open_url` allowlist. */
export function releasePageUrl(version?: string | null): string {
  return version && /^\d+\.\d+\.\d+([-+][0-9A-Za-z.-]+)?$/.test(version) ? `${RELEASES_URL}/tag/v${version}` : RELEASES_URL;
}

export type StatusTone = 'muted' | 'text-success' | 'text-accent' | 'text-warning' | 'text-danger';

export function updateStatus(state: UpdaterState, now: number): { text: string; tone: StatusTone } {
  const checked = state.checkedAt ? ` Verificado ${formatRelative(state.checkedAt, now)}.` : '';
  const version = state.update?.version;
  switch (state.phase) {
    case 'idle':
      return { text: 'Ainda não verificado.', tone: 'muted' };
    case 'checking':
      return { text: 'Verificando…', tone: 'muted' };
    case 'current':
      return { text: `Você está na versão mais recente.${checked}`, tone: 'text-success' };
    case 'available':
      return { text: `A versão ${version} está disponível.${checked}`, tone: 'text-accent' };
    case 'downloading': {
      // Bytes are shown under the progress bar.
      const ratio = downloadRatio(state);
      return { text: `Baixando a versão ${version}…${ratio === null ? '' : ` ${Math.round(ratio * 100)}%`}`, tone: 'text-accent' };
    }
    case 'verifying':
      return { text: `Conferindo a assinatura da versão ${version}…`, tone: 'text-accent' };
    case 'ready':
      return { text: `A versão ${version} foi baixada e verificada. Reinicie para atualizar.`, tone: 'text-success' };
    case 'installing':
      return { text: 'Instalando. O PC Remote vai fechar e abrir de novo sozinho.', tone: 'text-accent' };
    case 'error': {
      // Download and install errors show their detail and the release link below the status.
      if (state.error?.stage === 'download') return { text: `Não foi possível baixar a versão ${version}.`, tone: 'text-danger' };
      if (state.error?.stage === 'install') return { text: `Não foi possível instalar a versão ${version}.`, tone: 'text-danger' };
      return { text: `${state.error?.message ?? 'Não foi possível verificar.'}${checked}`, tone: 'text-warning' };
    }
  }
}

export interface UpdateNotice {
  label: string;
  title: string;
  tone: 'available' | 'busy' | 'ready';
}

/** The header pill, or null when there is nothing to announce. */
export function updateNotice(state: UpdaterState): UpdateNotice | null {
  const version = state.update?.version;
  if (!version) return null;
  switch (state.phase) {
    case 'ready':
      return { label: 'Atualização pronta', title: `Versão ${version} pronta. Reinicie para atualizar.`, tone: 'ready' };
    case 'installing':
      return { label: 'Instalando…', title: `Instalando a versão ${version}`, tone: 'busy' };
    case 'downloading':
    case 'verifying': {
      const ratio = downloadRatio(state);
      return { label: `Baixando atualização${ratio === null ? '' : ` ${Math.round(ratio * 100)}%`}`, title: `Baixando a versão ${version}`, tone: 'busy' };
    }
    case 'available':
    case 'error':
      return { label: `Versão ${version} disponível`, title: `Ver novidades da versão ${version}`, tone: 'available' };
    default:
      return null;
  }
}
