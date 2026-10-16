// Mirrors the serde output of `core::Status` in src-tauri/src/core.rs.
// Keep field names and optionality in sync with the Rust structs.

export interface PairingSession {
  code: string;
  /** Unix milliseconds. */
  expiresAt: number;
  qrPayload: string;
}

export interface Device {
  id: string;
  name: string;
  connected: boolean;
  /** Unix milliseconds of the latest connect/disconnect seen since the app started. */
  lastSeenAt: number | null;
}

export interface PendingDevice {
  id: string;
  deviceName: string;
  /** Unix milliseconds. */
  requestedAt: number;
}

export interface Settings {
  upload: boolean;
  download: boolean;
  paused: boolean;
  autostart: boolean;
  allowInput: boolean;
}

export interface DiagnosticEntry {
  /** Unix milliseconds. */
  at: number;
  message: string;
}

export type TransferDirection = 'toPhone' | 'toPc';

/** Clipboard transfer metadata. The backend never sends the copied text. */
export interface TransferEntry {
  at: number;
  direction: TransferDirection;
  characters: number;
  automatic: boolean;
}

export interface DesktopStatus {
  serviceRunning: boolean;
  inputActive: boolean;
  pcId: string;
  pcName: string;
  addresses: string[];
  port: number;
  fingerprint: string;
  pairing: PairingSession | null;
  devices: Device[];
  pending: PendingDevice[];
  settings: Settings;
  diagnostics: DiagnosticEntry[];
  /** Newest first. */
  transfers: TransferEntry[];
}

export const PROTOCOL_VERSION = 1;

export function preferredAddress(addresses: string[]): string | undefined {
  return addresses.find(address => !/^(127\.|0\.|::1$|localhost$)/i.test(address)) ?? addresses[0];
}

export function errorMessage(reason: unknown): string {
  if (typeof reason === 'string' && reason.trim()) return reason;
  if (reason instanceof Error && reason.message) return reason.message;
  return 'Não foi possível concluir. Tente novamente.';
}
