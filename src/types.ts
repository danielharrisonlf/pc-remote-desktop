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

/** Files and mouse/keyboard need no switch: they work after pairing. */
export interface Settings {
  /** Automatic clipboard sync in both directions; manual actions never depend on it. */
  realtime: boolean;
  autostart: boolean;
  /** Set by "Interromper"; only "Liberar" clears it. */
  inputBlocked: boolean;
}

export interface DiagnosticEntry {
  /** Unix milliseconds. */
  at: number;
  message: string;
}

export type TransferDirection = 'toPhone' | 'toPc';

/** Clipboard transfer metadata. The backend never sends the copied text or image. */
export interface TransferEntry {
  at: number;
  direction: TransferDirection;
  /** `image` only with phones that negotiated clipboard.v2 (Android 0.6.1+). */
  kind: 'text' | 'image';
  /** Text only; 0 for images. */
  characters: number;
  /** Images only; 0 for text. */
  bytes: number;
  automatic: boolean;
}

export type PowerAction = 'lock' | 'sleep' | 'shutdown' | 'restart';

/** An action a phone scheduled on this PC (power.v1). Kept in RAM by the backend. */
export interface PowerSchedule {
  action: PowerAction;
  /** Unix milliseconds. */
  dueAt: number;
  deviceName: string;
}

/** A file received from (`toPc`) or sent to (`toPhone`) the phone. Metadata only, kept in RAM. */
export interface FileEntry {
  /** Unix milliseconds. */
  at: number;
  name: string;
  bytes: number;
  direction: TransferDirection;
}

/** The file being offered or sent to the phone. */
export interface OutgoingFile {
  name: string;
  bytes: number;
  /** Bytes the phone confirmed it wrote. */
  sent: number;
  /** Announced, waiting for the phone to start. */
  waiting: boolean;
}

export interface OutgoingResult {
  at: number;
  ok: boolean;
  message: string;
}

/** The upload in progress, if any. */
export interface IncomingFile {
  name: string;
  bytes: number;
  received: number;
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
  /** Both directions, newest first, at most 12. */
  fileHistory: FileEntry[];
  incomingFile: IncomingFile | null;
  outgoing: OutgoingFile | null;
  /** Names waiting to go to the phone, in order. */
  outgoingQueue: string[];
  outgoingResult: OutgoingResult | null;
  /** The connected phone accepts files from the PC (Android 0.6+). */
  phoneReceivesFiles: boolean;
  /** Lock, sleep, shutdown or restart a phone scheduled; null when nothing is scheduled. */
  powerScheduled: PowerSchedule | null;
}

/** Mirrors `update::UpdateInfo` in src-tauri/src/update.rs: a newer version from latest.json. */
export interface UpdateInfo {
  version: string;
  currentVersion: string;
  /** Markdown from the manifest `notes`; may be empty. */
  notes: string;
  /** Unix milliseconds. */
  publishedAt: number | null;
  /** The installer for this version is already downloaded and verified in Rust. */
  ready: boolean;
}

/** Mirrors `update::DownloadEvent`, sent over a Channel during `update_download`. */
export type DownloadEvent =
  | { event: 'started'; data: { contentLength: number | null } }
  | { event: 'progress'; data: { downloaded: number; contentLength: number | null } }
  | { event: 'finished' };

export const PROTOCOL_VERSION = 1;

export function preferredAddress(addresses: string[]): string | undefined {
  return addresses.find(address => !/^(127\.|0\.|::1$|localhost$)/i.test(address)) ?? addresses[0];
}

export function errorMessage(reason: unknown): string {
  if (typeof reason === 'string' && reason.trim()) return reason;
  if (reason instanceof Error && reason.message) return reason.message;
  return 'Não foi possível concluir. Tente novamente.';
}
