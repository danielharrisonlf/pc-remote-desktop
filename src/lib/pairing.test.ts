import { describe, expect, it } from 'vitest';
import type { DesktopStatus, PairingResult, PendingDevice } from '../types';
import { pairingProgress, shouldStartPairing, type AwaitedPairing } from './pairing';

const approval: AwaitedPairing = { requestId: 'request-b', deviceId: 'phone-b', name: 'Phone B', startedAt: 1_000 };
const result = (state: PairingResult['state']): PairingResult => ({ requestId: approval.requestId, deviceId: approval.deviceId, state, message: null });
const pending: PendingDevice = { id: 'request-b', deviceId: 'phone-b', deviceName: 'Phone B', requestedAt: 1_000, expiresAt: 121_000 };
const expiredInvitation = { code: '12345678', expiresAt: 1_000, qrPayload: '{}' };
type PairingStatus = Pick<DesktopStatus, 'pairing' | 'pending' | 'pairingResult'>;

describe('pairing request and code lifetimes', () => {
  it('does not rotate an expired code while a valid request is waiting', () => {
    const status: PairingStatus = { pairing: expiredInvitation, pending: [pending] };
    expect(shouldStartPairing(status, 2_000)).toBe(false);
    expect(shouldStartPairing({ ...status, pairing: null }, 2_000)).toBe(false);
  });

  it('does not invalidate an approval while the phone is completing it', () => {
    expect(shouldStartPairing({ pairing: null, pending: [], pairingResult: result('approved') }, 2_000)).toBe(false);
  });

  it('starts a new invitation when there is no active code or request', () => {
    expect(shouldStartPairing({ pairing: null, pending: [] }, 2_000)).toBe(true);
    expect(shouldStartPairing({ pairing: expiredInvitation, pending: [], pairingResult: result('completed') }, 2_000)).toBe(true);
    expect(shouldStartPairing({ pairing: { ...expiredInvitation, expiresAt: 3_000 }, pending: [] }, 2_000)).toBe(false);
  });
});

describe('confirmed pairing outcome', () => {
  it('does not show success for an approved command or saved credential alone', () => {
    for (const state of ['approved', 'completed'] as const) {
      expect(pairingProgress({ devices: [], pairingResult: result(state) }, approval, 2_000).state).toBe('waiting');
      expect(pairingProgress({ devices: [{ id: 'phone-b', name: 'Phone B', connected: false, lastSeenAt: null }], pairingResult: result(state) }, approval, 2_000).state).toBe('waiting');
    }
  });

  it('reports persistence failure without announcing a paired phone', () => {
    expect(pairingProgress({ devices: [], pairingResult: { ...result('failed'), message: 'Não foi possível salvar.' } }, approval, 2_000))
      .toEqual({ state: 'error', message: 'Não foi possível salvar.' });
  });

  it('requires the expected phone to connect and ignores outcomes from another request', () => {
    const other = { id: 'phone-a', name: 'Phone A', connected: true, lastSeenAt: null };
    expect(pairingProgress({ devices: [other], pairingResult: { ...result('failed'), requestId: 'request-a' } }, approval, 2_000).state).toBe('waiting');
    expect(pairingProgress({ devices: [{ ...other, id: 'phone-b' }] }, approval, 2_000).state).toBe('connected');
  });

  it('ends the wait if the phone never authenticates after approval', () => {
    expect(pairingProgress({ devices: [], pairingResult: result('completed') }, approval, 36_000).state).toBe('error');
  });
});
