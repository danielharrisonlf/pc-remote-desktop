import { invoke } from '@tauri-apps/api/core';
import type { DesktopStatus, PairingSession, Settings } from './types';

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
};
