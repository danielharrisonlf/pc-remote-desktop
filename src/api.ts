import { invoke, type Channel } from '@tauri-apps/api/core';
import type { DesktopStatus, DownloadEvent, PairingSession, Settings, UpdateInfo } from './types';

/** Typed wrappers for the commands registered in src-tauri/src/lib.rs. */
export const desktopApi = {
  getStatus: () => invoke<DesktopStatus>('get_status'),
  startPairing: () => invoke<PairingSession>('start_pairing'),
  approvePairing: (id: string, approved: boolean) => invoke<void>('approve_pairing', { id, approved }),
  revokeDevice: (id: string) => invoke<void>('revoke_device', { id }),
  updateSettings: (settings: Settings) => invoke<void>('update_settings', { settings }),
  stopInput: () => invoke<void>('stop_input'),
  /** Opens a project GitHub release URL in the default browser (allowlisted in Rust). */
  openUrl: (url: string) => invoke<void>('open_url', { url }),
  /** Creates Downloads\PC Remote if needed and opens it in Explorer. */
  openReceivedFolder: () => invoke<void>('open_received_folder'),
  /** Reads latest.json in Rust; resolves to null when the running version is the newest. */
  updateCheck: () => invoke<UpdateInfo | null>('update_check'),
  /** Downloads the update found by the last check and verifies its signature in Rust. */
  updateDownload: (onEvent: Channel<DownloadEvent>) => invoke<void>('update_download', { onEvent }),
  /** Runs the verified installer in passive mode; the app exits and the installer reopens it. */
  updateInstall: () => invoke<void>('update_install'),
};
