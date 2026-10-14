import { describe, expect, it } from 'vitest';
import { errorMessage, preferredAddress } from './types';

describe('display data from the local service', () => {
  it('prefers a LAN address over loopback for manual pairing', () => {
    expect(preferredAddress(['127.0.0.1', '192.168.1.42'])).toBe('192.168.1.42');
  });

  it('preserves a backend error without exposing an unknown object', () => {
    expect(errorMessage('Código expirado')).toBe('Código expirado');
    expect(errorMessage({ token: 'secret' })).not.toContain('secret');
  });
});
