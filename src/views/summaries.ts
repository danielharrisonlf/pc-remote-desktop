import { formatBytes, formatClock, plural } from '../lib/format';
import type { DesktopStatus, PowerAction, PowerSchedule, Settings, TransferEntry } from '../types';

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

const POWER_VERBS: Record<PowerAction, string> = {
  lock: 'Bloquear',
  sleep: 'Suspender',
  shutdown: 'Desligar',
  restart: 'Reiniciar',
};

/** "Desligar às 23:40 · pedido por Pixel 8", in local time; "amanhã" past midnight. */
export function powerSummary(schedule: PowerSchedule, now: number): string {
  const today = new Date(now);
  today.setHours(24, 0, 0, 0);
  const day = schedule.dueAt >= today.getTime() ? 'amanhã ' : '';
  return `${POWER_VERBS[schedule.action]} ${day}às ${formatClock(schedule.dueAt)} · pedido por ${schedule.deviceName}`;
}

/** Meta line of a clipboard copy: characters for text, size for images. */
export function transferMeta(entry: TransferEntry): string {
  const mode = entry.automatic ? 'automático' : 'manual';
  if (entry.kind === 'image') return `Imagem · ${formatBytes(entry.bytes) || '0 B'}, ${mode}`;
  return `${plural(entry.characters, 'caractere', 'caracteres')}, ${mode}`;
}

/** Share of a file already moved, from 0 to 1. */
export function fileProgress(file: { bytes: number; received: number }): number {
  if (!Number.isFinite(file.bytes) || file.bytes <= 0) return 0;
  return Math.max(0, Math.min(1, file.received / file.bytes));
}
