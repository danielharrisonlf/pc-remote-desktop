import { invoke } from '@tauri-apps/api/core';
import type { DesktopStatus, PairingSession, Settings } from './types';

export const desktopApi = {
  getStatus: () => invoke<DesktopStatus>('get_status'),
  startPairing: () => invoke<PairingSession>('start_pairing'),
  approvePairing: (id: string, approved: boolean) => invoke<void>('approve_pairing', { id, approved }),
  revokeDevice: (id: string) => invoke<void>('revoke_device', { id }),
  updateSettings: (settings: Settings) => invoke<void>('update_settings', { settings }),
};
