// Fingerprint presentation shared with the Android app (src/domain/fingerprint.ts
// in pc-remote-android). Both sides must derive the same groups and colors so the
// user can compare them at a glance.

export function normalizeFingerprint(value: string): string {
  return value.replace(/[^a-fA-F0-9]/g, '').toUpperCase();
}

/** Groups of four hex characters: 64 characters become 16 groups. */
export function fingerprintGroups(value: string): string[] {
  const hex = normalizeFingerprint(value);
  const groups: string[] = [];
  for (let index = 0; index < hex.length; index += 4) groups.push(hex.slice(index, index + 4));
  return groups;
}

/** First two groups and the last one, enough to spot a mismatch quickly. */
export function fingerprintShort(value: string): string {
  const groups = fingerprintGroups(value);
  if (groups.length <= 3) return groups.join(' ');
  return `${groups[0]} ${groups[1]} … ${groups[groups.length - 1]}`;
}

/**
 * Eight colors derived from the SHA-256 bytes. Bytes are folded with XOR so every
 * byte of the digest influences the strip, and each folded byte picks a hue.
 */
export function fingerprintSwatches(value: string): string[] {
  const hex = normalizeFingerprint(value);
  if (hex.length < 16) return [];
  const bytes: number[] = [];
  for (let index = 0; index + 1 < hex.length; index += 2) bytes.push(parseInt(hex.slice(index, index + 2), 16));
  const folded = new Array<number>(8).fill(0);
  bytes.forEach((byte, index) => {
    folded[index % 8] ^= byte;
  });
  return folded.map(byte => {
    const hue = Math.round((byte / 256) * 360);
    const lightness = 46 + (byte % 3) * 6;
    return `hsl(${hue}, 64%, ${lightness}%)`;
  });
}
