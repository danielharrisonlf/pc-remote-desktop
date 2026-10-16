import { describe, expect, it } from 'vitest';
import type { UpdateInfo } from '../types';
import {
  canInstall,
  downloadRatio,
  initialUpdaterState,
  isCheckDue,
  isReleaseLink,
  releasePageUrl,
  retryStage,
  RETRY_FAILED_CHECK_AFTER,
  shouldAutoDownload,
  updateNotice,
  updaterReducer,
  updateStatus,
  type UpdaterAction,
  type UpdaterState,
} from './updates';

const HOUR = 3_600_000;
const NOW = Date.UTC(2026, 9, 16, 12);

const update = (extra: Partial<UpdateInfo> = {}): UpdateInfo => ({
  version: '0.5.1',
  currentVersion: '0.5.0',
  notes: '- Correções',
  publishedAt: NOW - HOUR,
  ready: false,
  ...extra,
});

const run = (...actions: UpdaterAction[]): UpdaterState => actions.reduce(updaterReducer, initialUpdaterState());

const found = (extra?: Partial<UpdateInfo>): UpdaterAction => ({ type: 'checked', update: update(extra), at: NOW });

describe('update state machine', () => {
  it('goes from check to available to a verified, installable download', () => {
    const steps: UpdaterAction[] = [
      { type: 'check' },
      found(),
      { type: 'downloadStarted' },
      { type: 'downloadEvent', event: { event: 'started', data: { contentLength: 4_000 } } },
      { type: 'downloadEvent', event: { event: 'progress', data: { downloaded: 1_000, contentLength: 4_000 } } },
    ];
    const downloading = run(...steps);
    expect(downloading).toMatchObject({ phase: 'downloading', downloaded: 1_000, total: 4_000 });
    expect(downloadRatio(downloading)).toBe(0.25);
    expect(canInstall(downloading)).toBe(false);

    const verifying = updaterReducer(downloading, { type: 'downloadEvent', event: { event: 'finished' } });
    expect(verifying.phase).toBe('verifying');
    expect(downloadRatio(verifying)).toBe(1);

    const ready = updaterReducer(verifying, { type: 'downloaded' });
    expect(ready).toMatchObject({ phase: 'ready', update: { ready: true } });
    expect(canInstall(ready)).toBe(true);
    expect(updaterReducer(ready, { type: 'install' }).phase).toBe('installing');
  });

  it('reports the running version as current and clears an earlier update', () => {
    const state = run(found(), { type: 'checked', update: null, at: NOW + 1 });
    expect(state).toMatchObject({ phase: 'current', update: null, checkedAt: NOW + 1 });
  });

  it('restores "ready" when Rust still holds the verified installer', () => {
    expect(run(found({ ready: true })).phase).toBe('ready');
  });

  it('ignores checks and stray progress while a download runs', () => {
    const downloading = run(found(), { type: 'downloadStarted' });
    expect(updaterReducer(downloading, { type: 'check' })).toBe(downloading);
    const ready = updaterReducer(downloading, { type: 'downloaded' });
    expect(updaterReducer(ready, { type: 'downloadEvent', event: { event: 'progress', data: { downloaded: 9, contentLength: 9 } } })).toBe(ready);
  });

  it('keeps "ready" when a later check fails, and reports other check failures', () => {
    const ready = run(found({ ready: true }), { type: 'checkFailed', message: 'Sem conexão com o servidor de atualizações.', at: NOW + 5 });
    expect(ready).toMatchObject({ phase: 'ready', checkedAt: NOW + 5, error: null });
    const failed = run({ type: 'check' }, { type: 'checkFailed', message: 'Sem conexão com o servidor de atualizações.', at: NOW });
    expect(failed).toMatchObject({ phase: 'error', error: { stage: 'check' } });
    expect(retryStage(failed)).toBe('check');
  });

  it('retries the step that failed', () => {
    const download = run(found(), { type: 'downloadStarted' }, { type: 'downloadFailed', message: 'A assinatura da atualização não confere.' });
    expect(download).toMatchObject({ phase: 'error', update: { version: '0.5.1' }, error: { stage: 'download' } });
    expect(retryStage(download)).toBe('download');
    expect(canInstall(download)).toBe(false);
    expect(updaterReducer(download, { type: 'install' })).toBe(download);

    const install = run(found({ ready: true }), { type: 'install' }, { type: 'installFailed', message: 'Não foi possível iniciar o instalador.' });
    expect(install).toMatchObject({ phase: 'error', error: { stage: 'install' } });
    expect(retryStage(install)).toBe('install');
    expect(updaterReducer(install, { type: 'install' }).phase).toBe('installing');
  });

  it('only installs from a verified download', () => {
    expect(updaterReducer(run(found()), { type: 'install' }).phase).toBe('available');
    expect(updaterReducer(initialUpdaterState(), { type: 'downloadStarted' }).phase).toBe('idle');
  });

  it('auto-downloads only updates that are not downloaded yet', () => {
    expect(shouldAutoDownload(update(), true)).toBe(true);
    expect(shouldAutoDownload(update(), false)).toBe(false);
    expect(shouldAutoDownload(update({ ready: true }), true)).toBe(false);
    expect(shouldAutoDownload(null, true)).toBe(false);
  });
});

describe('automatic check schedule', () => {
  const checked = (at: number, failed = false): UpdaterState =>
    failed ? run({ type: 'checkFailed', message: 'x', at }) : run({ type: 'checked', update: null, at });

  it('never checks on its own in manual mode', () => {
    expect(isCheckDue(initialUpdaterState(), 0, NOW, true)).toBe(false);
  });

  it('checks once after start, then on the interval', () => {
    expect(isCheckDue(checked(NOW - 60_000), 6, NOW, true)).toBe(true);
    expect(isCheckDue(checked(NOW - 5 * HOUR), 6, NOW, false)).toBe(false);
    expect(isCheckDue(checked(NOW - 6 * HOUR), 6, NOW, false)).toBe(true);
  });

  it('retries a failed check after 15 minutes', () => {
    expect(isCheckDue(checked(NOW - RETRY_FAILED_CHECK_AFTER + 1, true), 24, NOW, false)).toBe(false);
    expect(isCheckDue(checked(NOW - RETRY_FAILED_CHECK_AFTER, true), 24, NOW, false)).toBe(true);
  });

  it('leaves a running download or a ready update alone', () => {
    expect(isCheckDue(run(found(), { type: 'downloadStarted' }), 1, NOW, true)).toBe(false);
    expect(isCheckDue(run(found({ ready: true })), 1, NOW + 48 * HOUR, false)).toBe(false);
  });
});

describe('update copy', () => {
  it('describes each phase in Portuguese', () => {
    expect(updateStatus(initialUpdaterState(), NOW).text).toBe('Ainda não verificado.');
    expect(updateStatus(run({ type: 'checked', update: null, at: NOW - 3 * 60_000 }), NOW)).toEqual({
      text: 'Você está na versão mais recente. Verificado há 3 min.',
      tone: 'text-success',
    });
    const downloading = run(found(), { type: 'downloadStarted' }, {
      type: 'downloadEvent',
      event: { event: 'progress', data: { downloaded: 1_048_576, contentLength: 4_194_304 } },
    });
    expect(updateStatus(downloading, NOW).text).toBe('Baixando a versão 0.5.1… 25%');
    expect(updateStatus(run(found({ ready: true })), NOW)).toEqual({
      text: 'A versão 0.5.1 foi baixada e verificada. Reinicie para atualizar.',
      tone: 'text-success',
    });
    const failed = run(found(), { type: 'downloadStarted' }, { type: 'downloadFailed', message: 'O download foi interrompido.' });
    expect(updateStatus(failed, NOW)).toEqual({ text: 'Não foi possível baixar a versão 0.5.1.', tone: 'text-danger' });
    const offline = run({ type: 'checkFailed', message: 'Sem conexão com o servidor de atualizações.', at: NOW - 60_000 });
    expect(updateStatus(offline, NOW)).toEqual({
      text: 'Sem conexão com o servidor de atualizações. Verificado há 1 min.',
      tone: 'text-warning',
    });
  });

  it('announces available, downloading and ready updates in the header', () => {
    expect(updateNotice(run({ type: 'checked', update: null, at: NOW }))).toBeNull();
    expect(updateNotice(run(found()))).toMatchObject({ label: 'Versão 0.5.1 disponível', tone: 'available' });
    const half = run(found(), { type: 'downloadStarted' }, {
      type: 'downloadEvent',
      event: { event: 'progress', data: { downloaded: 50, contentLength: 100 } },
    });
    expect(updateNotice(half)).toMatchObject({ label: 'Baixando atualização 50%', tone: 'busy' });
    expect(updateNotice(run(found({ ready: true })))).toMatchObject({ label: 'Atualização pronta', tone: 'ready' });
  });

  it('opens only this project release pages', () => {
    expect(releasePageUrl('0.5.1')).toBe('https://github.com/danielharrisonlf/pc-remote-desktop/releases/tag/v0.5.1');
    expect(releasePageUrl('../evil')).toBe('https://github.com/danielharrisonlf/pc-remote-desktop/releases');
    expect(releasePageUrl(null)).toBe('https://github.com/danielharrisonlf/pc-remote-desktop/releases');
    expect(isReleaseLink('https://github.com/danielharrisonlf/pc-remote-android/releases/tag/v0.4.0-kotlin')).toBe(true);
    expect(isReleaseLink('https://github.com/danielharrisonlf/pc-remote-desktop/releasesx')).toBe(false);
    expect(isReleaseLink('https://github.com/danielharrisonlf/pc-remote-desktop/releases/../../evil')).toBe(false);
    expect(isReleaseLink('https://evil.example/danielharrisonlf/pc-remote-desktop/releases')).toBe(false);
  });
});
