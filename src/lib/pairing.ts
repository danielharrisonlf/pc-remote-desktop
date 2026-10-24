import type { DesktopStatus } from '../types';

export interface AwaitedPairing {
  requestId: string;
  deviceId: string;
  name: string;
  startedAt: number;
}

export type PairingProgress = { state: 'waiting' | 'connected' } | { state: 'error'; message: string };

/** Reopening the drawer must not invalidate a request already being approved. */
export function shouldStartPairing(status: Pick<DesktopStatus, 'pairing' | 'pending' | 'pairingResult'>, now: number): boolean {
  return status.pending.length === 0
    && status.pairingResult?.state !== 'approved'
    && (!status.pairing || status.pairing.expiresAt <= now);
}

/** Local approval is not success: wait for this phone to authenticate. */
export function pairingProgress(status: Pick<DesktopStatus, 'devices' | 'pairingResult'>, approval: AwaitedPairing, now: number): PairingProgress {
  if (status.devices.some(device => device.id === approval.deviceId && device.connected)) return { state: 'connected' };
  const result = status.pairingResult;
  if (result?.requestId === approval.requestId && result.deviceId === approval.deviceId
    && !['approved', 'completed'].includes(result.state)) {
    return { state: 'error', message: result.message || 'Não foi possível concluir o pareamento. Tente novamente.' };
  }
  if (now - approval.startedAt >= 35_000) {
    return { state: 'error', message: 'O celular não concluiu a conexão. Confira o app no celular e tente novamente.' };
  }
  return { state: 'waiting' };
}
