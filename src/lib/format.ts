const clock = new Intl.DateTimeFormat('pt-BR', { hour: '2-digit', minute: '2-digit' });
const clockSeconds = new Intl.DateTimeFormat('pt-BR', { hour: '2-digit', minute: '2-digit', second: '2-digit' });
const shortDay = new Intl.DateTimeFormat('pt-BR', { day: 'numeric', month: 'short' });
const longDate = new Intl.DateTimeFormat('pt-BR', { day: 'numeric', month: 'long', year: 'numeric' });
const DAY = 24 * 60 * 60 * 1000;

function validDate(value: number | string): Date | null {
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? null : date;
}

export function formatClock(value: number, withSeconds = false): string {
  const date = validDate(value);
  if (!date) return '--:--';
  return (withSeconds ? clockSeconds : clock).format(date);
}

/** "agora", "há 12 s", "há 5 min", "há 3 h". Falls back to a day label after 24 h. */
export function formatRelative(value: number, now: number): string {
  const elapsed = Math.max(0, now - value);
  if (elapsed < 5_000) return 'agora';
  if (elapsed < 60_000) return `há ${Math.floor(elapsed / 1000)} s`;
  if (elapsed < 3_600_000) return `há ${Math.floor(elapsed / 60_000)} min`;
  if (elapsed < DAY) return `há ${Math.floor(elapsed / 3_600_000)} h`;
  return formatDay(value, now);
}

/** "hoje às 14:20", "ontem às 09:10" or "15 de out. às 14:20". */
export function formatDay(value: number, now: number): string {
  const date = validDate(value);
  if (!date) return 'data desconhecida';
  const startOfToday = new Date(now);
  startOfToday.setHours(0, 0, 0, 0);
  const time = clock.format(date);
  if (value >= startOfToday.getTime()) return `hoje às ${time}`;
  if (value >= startOfToday.getTime() - DAY) return `ontem às ${time}`;
  return `${shortDay.format(date)} às ${time}`;
}

export function formatLongDate(value: string | null): string | null {
  if (!value) return null;
  const date = validDate(value);
  return date ? longDate.format(date) : null;
}

/** Remaining time as m:ss, never negative. */
export function formatCountdown(remainingMs: number): string {
  const seconds = Math.max(0, Math.ceil(remainingMs / 1000));
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, '0')}`;
}

export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return '';
  const units = ['B', 'KB', 'MB', 'GB'];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  const digits = unit === 0 || value >= 100 ? 0 : 1;
  return `${value.toLocaleString('pt-BR', { maximumFractionDigits: digits, minimumFractionDigits: digits })} ${units[unit]}`;
}

/** Pairing codes are 8 digits; show them as two groups for reading aloud. */
export function groupCode(code: string): string {
  return code.length === 8 ? `${code.slice(0, 4)} ${code.slice(4)}` : code;
}

export function plural(count: number, singular: string, pluralForm: string): string {
  return `${count} ${count === 1 ? singular : pluralForm}`;
}
