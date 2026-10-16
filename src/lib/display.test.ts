import { describe, expect, it } from 'vitest';
import { fingerprintGroups, fingerprintShort, fingerprintSwatches, normalizeFingerprint } from './fingerprint';
import { formatBytes, formatCountdown, formatRelative, groupCode } from './format';
import { parseInline, parseMarkdown } from './markdown';

// sha256('pc-remote')
const digest = '6fab114e66015573129c77331ae7a7a6f891e79d840750f32bcd7a3d55bd3d53';

describe('fingerprint presentation', () => {
  it('normalizes separators and case', () => {
    expect(normalizeFingerprint('a1:b2 c3')).toBe('A1B2C3');
  });

  it('splits a SHA-256 into sixteen groups of four', () => {
    const groups = fingerprintGroups(digest);
    expect(groups).toHaveLength(16);
    expect(groups[0]).toBe('6FAB');
    expect(fingerprintShort(digest)).toBe('6FAB 114E … 3D53');
  });

  it('derives a stable strip of eight colors shared with the Android app', () => {
    const swatches = fingerprintSwatches(digest);
    expect(swatches).toHaveLength(8);
    expect(fingerprintSwatches(digest.toUpperCase())).toEqual(swatches);
    // Golden value: the Android implementation asserts the same output.
    expect(swatches).toEqual([
      'hsl(245, 64%, 46%)',
      'hsl(150, 64%, 58%)',
      'hsl(353, 64%, 58%)',
      'hsl(311, 64%, 58%)',
      'hsl(243, 64%, 58%)',
      'hsl(129, 64%, 58%)',
      'hsl(224, 64%, 46%)',
      'hsl(165, 64%, 46%)',
    ]);
    expect(fingerprintSwatches('abc')).toEqual([]);
  });

  it('changes when any byte of the digest changes', () => {
    const tampered = `${digest.slice(0, -2)}54`;
    expect(fingerprintSwatches(tampered)).not.toEqual(fingerprintSwatches(digest));
  });
});

describe('formatting', () => {
  it('formats countdowns as m:ss and never negative', () => {
    expect(formatCountdown(119_200)).toBe('2:00');
    expect(formatCountdown(61_000)).toBe('1:01');
    expect(formatCountdown(-5)).toBe('0:00');
  });

  it('describes recent moments relatively', () => {
    const now = Date.UTC(2026, 9, 15, 15, 0, 0);
    expect(formatRelative(now - 2_000, now)).toBe('agora');
    expect(formatRelative(now - 42_000, now)).toBe('há 42 s');
    expect(formatRelative(now - 5 * 60_000, now)).toBe('há 5 min');
  });

  it('groups pairing codes and sizes', () => {
    expect(groupCode('12345678')).toBe('1234 5678');
    expect(formatBytes(3_470_367)).toBe('3,3 MB');
    expect(formatBytes(0)).toBe('');
  });
});

describe('release notes markdown', () => {
  it('parses headings, lists and inline emphasis into data', () => {
    const blocks = parseMarkdown('## Novidades\n\n- **Rápido** e `seguro`\n- Veja [notas](https://github.com/x)\n\nTexto final.');
    expect(blocks.map(block => block.type)).toEqual(['heading', 'list', 'paragraph']);
    const list = blocks[1];
    expect(list.type === 'list' && list.items).toHaveLength(2);
    expect(parseInline('**a** `b`')).toEqual([
      { type: 'strong', children: [{ type: 'text', text: 'a' }] },
      { type: 'text', text: ' ' },
      { type: 'code', text: 'b' },
    ]);
  });

  it('keeps raw HTML as plain text', () => {
    const [block] = parseMarkdown('<img src=x onerror=alert(1)>');
    expect(block).toEqual({ type: 'paragraph', children: [{ type: 'text', text: '<img src=x onerror=alert(1)>' }] });
  });
});
