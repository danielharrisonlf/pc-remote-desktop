import { describe, expect, it } from 'vitest';
import type { DesktopStatus, Settings } from '../types';
import { clipboardSummary, fileProgress, inputSummary, sendSummary } from './summaries';

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

  it('bounds the progress of a file', () => {
    expect(fileProgress({ bytes: 200, received: 50 })).toBe(0.25);
    expect(fileProgress({ bytes: 0, received: 0 })).toBe(0);
    expect(fileProgress({ bytes: 10, received: 15 })).toBe(1);
  });
});
