import type { DesktopStatus, Settings } from '../types';

export function clipboardSummary(settings: Settings): string {
  return settings.realtime ? 'Área de transferência nos dois sentidos' : 'Desligada; envios manuais continuam';
}

export function inputSummary(settings: Settings, inputActive: boolean): string {
  if (settings.inputBlocked) return 'Bloqueado';
  if (inputActive) return 'Em uso';
  return 'Pronto';
}

/** What the "Enviar ao celular" area says when nothing is moving. */
export function sendSummary(status: DesktopStatus): string {
  const connected = status.devices.some(device => device.connected);
  if (!connected) return 'Conecte o celular para enviar.';
  if (!status.phoneReceivesFiles) return 'Atualize o PC Remote no celular para receber arquivos.';
  return 'Até 1 GB por arquivo. O celular salva em Download/PC Remote.';
}

/** Share of a file already moved, from 0 to 1. */
export function fileProgress(file: { bytes: number; received: number }): number {
  if (!Number.isFinite(file.bytes) || file.bytes <= 0) return 0;
  return Math.max(0, Math.min(1, file.received / file.bytes));
}
