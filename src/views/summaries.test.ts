import { describe, expect, it } from 'vitest';
import type { Settings } from '../types';
import { fileProgress, filesSummary, inputSummary } from './summaries';

const settings: Settings = { upload: false, download: false, paused: false, autostart: false, allowInput: false, allowFiles: false };

describe('permission summaries', () => {
  it('describes file reception like mouse and keyboard', () => {
    expect(filesSummary(settings, null)).toBe('Bloqueado');
    expect(filesSummary({ ...settings, allowFiles: true }, null)).toBe('Liberado');
    expect(filesSummary({ ...settings, allowFiles: true }, { name: 'foto.png', bytes: 10, received: 2 })).toBe('Recebendo');
    expect(inputSummary({ ...settings, allowInput: true }, false)).toBe('Liberado');
  });

  it('bounds the incoming file progress', () => {
    expect(fileProgress({ name: 'a.pdf', bytes: 200, received: 50 })).toBe(0.25);
    expect(fileProgress({ name: 'a.pdf', bytes: 0, received: 0 })).toBe(0);
    expect(fileProgress({ name: 'a.pdf', bytes: 10, received: 15 })).toBe(1);
  });
});
