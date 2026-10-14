export interface PairingSession {
  code: string;
  expiresAt: string | number;
  qrPayload: string;
}

export interface Device {
  id: string;
  name: string;
  connected: boolean;
}

export interface PendingDevice {
  id: string;
  deviceName: string;
}

export interface Settings {
  upload: boolean;
  download: boolean;
  paused: boolean;
  autostart: boolean;
}

export interface DesktopStatus {
  serviceRunning: boolean;
  pcId: string;
  pcName: string;
  addresses: string[];
  port: number;
  fingerprint: string;
  pairing: PairingSession | null;
  devices: Device[];
  pending: PendingDevice[];
  settings: Settings;
  diagnostics: string[];
}

export function preferredAddress(addresses: string[]): string | undefined {
  return addresses.find(address => !/^(127\.|0\.|::1$|localhost$)/i.test(address)) ?? addresses[0];
}

export function formatExpiry(value: string | number): string {
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? 'em instantes' : date.toLocaleTimeString('pt-BR', { hour: '2-digit', minute: '2-digit' });
}

export function errorMessage(reason: unknown): string {
  if (typeof reason === 'string' && reason.trim()) return reason;
  if (reason instanceof Error && reason.message) return reason.message;
  return 'Não foi possível concluir a ação. Confira o diagnóstico e tente novamente.';
}
