import type { IncomingFile, Settings } from '../types';

export function clipboardSummary(settings: Settings): string {
  if (settings.paused) return 'Pausada';
  if (settings.upload && settings.download) return 'Nos dois sentidos';
  if (settings.upload) return 'Só do PC para o celular';
  if (settings.download) return 'Só do celular para o PC';
  return 'Nenhum sentido ativo';
}

export function inputSummary(settings: Settings, inputActive: boolean): string {
  if (inputActive) return 'Em uso';
  if (settings.allowInput) return 'Liberado';
  return 'Bloqueado';
}

export function filesSummary(settings: Settings, incoming: IncomingFile | null): string {
  if (incoming) return 'Recebendo';
  if (settings.allowFiles) return 'Liberado';
  return 'Bloqueado';
}

/** Share of the incoming file already received, from 0 to 1. */
export function fileProgress(incoming: IncomingFile): number {
  if (!Number.isFinite(incoming.bytes) || incoming.bytes <= 0) return 0;
  return Math.max(0, Math.min(1, incoming.received / incoming.bytes));
}
