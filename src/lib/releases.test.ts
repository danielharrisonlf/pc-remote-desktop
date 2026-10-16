import { describe, expect, it, vi } from 'vitest';
import { fetchLatestRelease, parseRelease, pickInstaller } from './releases';

const release = (tag: string, extra: Record<string, unknown> = {}) => ({
  tag_name: tag,
  name: `PC Remote ${tag}`,
  body: '- Correções',
  published_at: '2026-10-16T00:56:50Z',
  html_url: `https://github.com/danielharrisonlf/pc-remote-desktop/releases/tag/${tag}`,
  draft: false,
  prerelease: false,
  assets: [
    {
      name: `pc-remote-desktop-${tag.slice(1)}-x64-setup.exe`,
      size: 3_470_367,
      browser_download_url: `https://github.com/danielharrisonlf/pc-remote-desktop/releases/download/${tag}/setup.exe`,
    },
  ],
  ...extra,
});

const json = (body: unknown, status = 200, headers: Record<string, string> = {}) =>
  new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json', ...headers } });

describe('GitHub release parsing', () => {
  it('keeps only fields the UI needs and rejects foreign download hosts', () => {
    const parsed = parseRelease(
      release('v0.3.0', {
        assets: [
          { name: 'ok.exe', size: 10, browser_download_url: 'https://github.com/danielharrisonlf/pc-remote-desktop/releases/download/v0.3.0/ok.exe' },
          { name: 'evil.exe', size: 10, browser_download_url: 'https://evil.example/evil.exe' },
        ],
      }),
    );
    expect(parsed?.version).toBe('0.3.0');
    expect(parsed?.assets.map(asset => asset.name)).toEqual(['ok.exe']);
  });

  it('rejects releases without a semantic version tag', () => {
    expect(parseRelease(release('nightly'))).toBeNull();
    expect(parseRelease(null)).toBeNull();
  });

  it('picks the NSIS installer first', () => {
    const assets = [
      { name: 'SHA256SUMS.txt', size: 1, downloadUrl: 'https://github.com/a' },
      { name: 'app.msi', size: 1, downloadUrl: 'https://github.com/b' },
      { name: 'pc-remote-desktop-0.3.0-x64-setup.exe', size: 1, downloadUrl: 'https://github.com/c' },
    ];
    expect(pickInstaller(assets)?.name).toBe('pc-remote-desktop-0.3.0-x64-setup.exe');
    expect(pickInstaller(assets.slice(0, 2))?.name).toBe('app.msi');
  });
});

describe('latest release lookup', () => {
  it('returns the latest release when its tag belongs to this app', async () => {
    const fetchImpl = vi.fn(async () => json(release('v0.3.0')));
    const result = await fetchLatestRelease('owner/repo', { fetchImpl });
    expect(result.kind === 'found' && result.release.version).toBe('0.3.0');
    expect(fetchImpl).toHaveBeenCalledTimes(1);
  });

  it('treats 404 as an unavailable feed, which is how a private repository answers', async () => {
    const result = await fetchLatestRelease('owner/repo', { fetchImpl: async () => json({ message: 'Not Found' }, 404) });
    expect(result).toMatchObject({ kind: 'unavailable', reason: 'not-found' });
  });

  it('reports the rate-limit reset time', async () => {
    const result = await fetchLatestRelease('owner/repo', {
      fetchImpl: async () => json({ message: 'API rate limit exceeded' }, 403, { 'x-ratelimit-reset': '1800000000' }),
    });
    expect(result).toMatchObject({ kind: 'unavailable', reason: 'rate-limited', retryAt: 1_800_000_000_000 });
  });

  it('reports network failures without throwing', async () => {
    const result = await fetchLatestRelease('owner/repo', {
      fetchImpl: async () => {
        throw new TypeError('Failed to fetch');
      },
    });
    expect(result).toMatchObject({ kind: 'unavailable', reason: 'network' });
  });

  it('falls back to the newest stable release of this flavor when latest belongs to another', async () => {
    const fetchImpl = vi.fn(async (url: RequestInfo | URL) =>
      String(url).endsWith('/latest')
        ? json(release('v0.2.4-kotlin'))
        : json([
            release('v0.2.4-kotlin'),
            release('v0.2.1'),
            release('v0.2.3', { prerelease: true }),
            release('v0.2.2', { draft: true }),
            release('v0.1.0'),
          ]),
    );
    const result = await fetchLatestRelease('owner/repo', { fetchImpl });
    expect(result.kind === 'found' && result.release.tag).toBe('v0.2.1');
    expect(fetchImpl).toHaveBeenCalledTimes(2);
  });
});
