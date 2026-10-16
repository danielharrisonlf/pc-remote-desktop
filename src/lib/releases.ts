import { compareVersions, displayVersion, parseVersion } from './semver';

export interface ReleaseAsset {
  name: string;
  size: number;
  downloadUrl: string;
}

export interface ReleaseInfo {
  tag: string;
  version: string;
  title: string;
  notes: string;
  publishedAt: string | null;
  pageUrl: string;
  assets: ReleaseAsset[];
}

export type ReleaseLookup =
  | { kind: 'found'; release: ReleaseInfo }
  | { kind: 'none' }
  | { kind: 'unavailable'; reason: 'not-found' | 'rate-limited' | 'network' | 'invalid'; message: string; retryAt?: number };

export interface FetchReleaseOptions {
  /** Accepts the tags that belong to this app. Defaults to plain `vX.Y.Z` tags. */
  acceptTag?: (tag: string) => boolean;
  fetchImpl?: typeof fetch;
  signal?: AbortSignal;
}

const API = 'https://api.github.com/repos';
export const stableTag = (tag: string) => /^v?\d+\.\d+\.\d+$/.test(tag.trim());

function text(value: unknown): string {
  return typeof value === 'string' ? value : '';
}

export function parseRelease(raw: unknown): ReleaseInfo | null {
  if (!raw || typeof raw !== 'object') return null;
  const value = raw as Record<string, unknown>;
  const tag = text(value.tag_name).trim();
  const pageUrl = text(value.html_url);
  if (!tag || !parseVersion(tag) || !pageUrl.startsWith('https://github.com/')) return null;
  const assets = Array.isArray(value.assets)
    ? value.assets.flatMap(item => {
        if (!item || typeof item !== 'object') return [];
        const asset = item as Record<string, unknown>;
        const downloadUrl = text(asset.browser_download_url);
        const name = text(asset.name);
        if (!name || !downloadUrl.startsWith('https://github.com/')) return [];
        return [{ name, downloadUrl, size: typeof asset.size === 'number' ? asset.size : 0 }];
      })
    : [];
  return {
    tag,
    version: displayVersion(tag),
    title: text(value.name).trim() || `Versão ${displayVersion(tag)}`,
    notes: text(value.body).replace(/\r\n/g, '\n').trim(),
    publishedAt: text(value.published_at) || null,
    pageUrl,
    assets,
  };
}

function rateLimitReset(response: Response): number | undefined {
  const reset = Number(response.headers.get('x-ratelimit-reset'));
  return Number.isFinite(reset) && reset > 0 ? reset * 1000 : undefined;
}

async function request(url: string, options: FetchReleaseOptions): Promise<{ response?: Response; failure?: ReleaseLookup }> {
  const fetchImpl = options.fetchImpl ?? fetch;
  try {
    // Only the CORS-safelisted Accept header is sent, so the webview makes a
    // simple request without a preflight.
    const response = await fetchImpl(url, { headers: { Accept: 'application/vnd.github+json' }, signal: options.signal });
    if (response.status === 403 || response.status === 429) {
      return {
        failure: {
          kind: 'unavailable',
          reason: 'rate-limited',
          message: 'GitHub indisponível no momento. Tente mais tarde.',
          retryAt: rateLimitReset(response),
        },
      };
    }
    return { response };
  } catch (reason) {
    if (reason instanceof DOMException && reason.name === 'AbortError') throw reason;
    return { failure: { kind: 'unavailable', reason: 'network', message: 'Sem conexão com o GitHub.' } };
  }
}

/**
 * Reads `/releases/latest` and falls back to the release list when the latest
 * release belongs to another flavor (for example a `-kotlin` tag).
 */
export async function fetchLatestRelease(repo: string, options: FetchReleaseOptions = {}): Promise<ReleaseLookup> {
  const accept = options.acceptTag ?? stableTag;
  const latest = await request(`${API}/${repo}/releases/latest`, options);
  if (latest.failure) return latest.failure;
  const response = latest.response!;
  if (response.status === 404) {
    return {
      kind: 'unavailable',
      reason: 'not-found',
      message: 'Nenhuma versão publicada encontrada.',
    };
  }
  if (!response.ok) {
    return { kind: 'unavailable', reason: 'network', message: `O GitHub respondeu com o código ${response.status}.` };
  }
  let body: unknown;
  try {
    body = await response.json();
  } catch {
    return { kind: 'unavailable', reason: 'invalid', message: 'Resposta inválida do GitHub.' };
  }
  const release = parseRelease(body);
  if (!release) return { kind: 'unavailable', reason: 'invalid', message: 'A release mais recente não tem uma versão válida.' };
  if (accept(release.tag)) return { kind: 'found', release };

  const list = await request(`${API}/${repo}/releases?per_page=30`, options);
  if (list.failure) return list.failure;
  if (!list.response!.ok) return { kind: 'none' };
  let items: unknown;
  try {
    items = await list.response!.json();
  } catch {
    return { kind: 'unavailable', reason: 'invalid', message: 'Resposta inválida do GitHub.' };
  }
  const candidates = (Array.isArray(items) ? items : [])
    .filter(item => item && typeof item === 'object' && !(item as { draft?: boolean }).draft && !(item as { prerelease?: boolean }).prerelease)
    .map(parseRelease)
    .filter((item): item is ReleaseInfo => item !== null && accept(item.tag))
    .sort((a, b) => compareVersions(parseVersion(b.tag)!, parseVersion(a.tag)!));
  return candidates[0] ? { kind: 'found', release: candidates[0] } : { kind: 'none' };
}

/** Prefers the NSIS installer, then any .exe, then an .msi. */
export function pickInstaller(assets: ReleaseAsset[]): ReleaseAsset | undefined {
  return (
    assets.find(asset => /-setup\.exe$/i.test(asset.name)) ??
    assets.find(asset => /\.exe$/i.test(asset.name)) ??
    assets.find(asset => /\.msi$/i.test(asset.name))
  );
}
