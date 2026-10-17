import { describe, expect, it } from 'vitest';
import type { DesktopStatus, Settings } from '../types';
import { clipboardSummary, fileProgress, inputSummary, powerSummary, sendSummary, transferMeta } from './summaries';

const settings: Settings = { realtime: true, autostart: false, inputBlocked: false };

const status = (patch: Partial<DesktopStatus>): DesktopStatus => ({
  serviceRunning: true,
  inputActive: false,
  pcId: 'pc',
  pcName: 'PC',
  addresses: [],
  port: 43127,
  fingerprint: '',
  pairing: null,
  devices: [],
  pending: [],
  settings,
  diagnostics: [],
  transfers: [],
  fileHistory: [],
  incomingFile: null,
  outgoing: null,
  outgoingQueue: [],
  outgoingResult: null,
  phoneReceivesFiles: false,
  powerScheduled: null,
  ...patch,
});

describe('summaries', () => {
  it('describes the single sync switch and the input block', () => {
    expect(clipboardSummary(settings)).toBe('Área de transferência nos dois sentidos');
    expect(clipboardSummary({ ...settings, realtime: false })).toContain('manuais');
    expect(inputSummary(settings, false)).toBe('Pronto');
    expect(inputSummary(settings, true)).toBe('Em uso');
    expect(inputSummary({ ...settings, inputBlocked: true }, true)).toBe('Bloqueado');
  });

  it('explains why files cannot go to the phone yet', () => {
    const phone = { id: 'p', name: 'Celular', connected: true, lastSeenAt: null };
    expect(sendSummary(status({}))).toBe('Conecte o celular para enviar.');
    expect(sendSummary(status({ devices: [phone] }))).toContain('Atualize');
    expect(sendSummary(status({ devices: [phone], phoneReceivesFiles: true }))).toContain('1 GB');
  });

  it('describes what a phone scheduled, in local time', () => {
    const now = new Date(2026, 9, 16, 22, 10).getTime();
    const tonight = new Date(2026, 9, 16, 23, 40).getTime();
    const tomorrow = new Date(2026, 9, 17, 1, 5).getTime();
    expect(powerSummary({ action: 'shutdown', dueAt: tonight, deviceName: 'Pixel 8' }, now)).toBe('Desligar às 23:40 · pedido por Pixel 8');
    expect(powerSummary({ action: 'sleep', dueAt: tomorrow, deviceName: 'Galaxy' }, now)).toBe('Suspender amanhã às 01:05 · pedido por Galaxy');
    expect(powerSummary({ action: 'restart', dueAt: tonight, deviceName: 'X' }, now)).toMatch(/^Reiniciar às/);
  });

  it('labels clipboard copies by kind', () => {
    const base = { at: 0, direction: 'toPc' as const, automatic: true };
    expect(transferMeta({ ...base, kind: 'text', characters: 1, bytes: 0 })).toBe('1 caractere, automático');
    expect(transferMeta({ ...base, kind: 'image', characters: 0, bytes: 2.3 * 1024 * 1024, automatic: false })).toBe('Imagem · 2,3 MB, manual');
  });

  it('bounds the progress of a file', () => {
    expect(fileProgress({ bytes: 200, received: 50 })).toBe(0.25);
    expect(fileProgress({ bytes: 0, received: 0 })).toBe(0);
    expect(fileProgress({ bytes: 10, received: 15 })).toBe(1);
  });
});
