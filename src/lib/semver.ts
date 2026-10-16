export interface Version {
  major: number;
  minor: number;
  patch: number;
  prerelease: string[];
}

const pattern = /^v?(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?(?:\+[0-9A-Za-z.-]+)?$/;

export function parseVersion(input: string): Version | null {
  const match = pattern.exec(input.trim());
  if (!match) return null;
  return {
    major: Number(match[1]),
    minor: Number(match[2]),
    patch: Number(match[3]),
    prerelease: match[4] ? match[4].split('.') : [],
  };
}

function compareIdentifiers(a: string, b: string): number {
  const numericA = /^\d+$/.test(a);
  const numericB = /^\d+$/.test(b);
  if (numericA && numericB) return Math.sign(Number(a) - Number(b));
  if (numericA) return -1;
  if (numericB) return 1;
  return a < b ? -1 : a > b ? 1 : 0;
}

/** Semantic Versioning 2.0 precedence: negative when a < b, zero when equal. */
export function compareVersions(a: Version, b: Version): number {
  for (const key of ['major', 'minor', 'patch'] as const) {
    if (a[key] !== b[key]) return Math.sign(a[key] - b[key]);
  }
  // A normal release has higher precedence than any prerelease of it.
  if (!a.prerelease.length && !b.prerelease.length) return 0;
  if (!a.prerelease.length) return 1;
  if (!b.prerelease.length) return -1;
  const length = Math.max(a.prerelease.length, b.prerelease.length);
  for (let index = 0; index < length; index += 1) {
    if (a.prerelease[index] === undefined) return -1;
    if (b.prerelease[index] === undefined) return 1;
    const result = compareIdentifiers(a.prerelease[index], b.prerelease[index]);
    if (result) return result;
  }
  return 0;
}

/** True only when both strings parse and `candidate` has higher precedence. */
export function isNewerVersion(candidate: string, current: string): boolean {
  const next = parseVersion(candidate);
  const installed = parseVersion(current);
  return Boolean(next && installed && compareVersions(next, installed) > 0);
}

export function displayVersion(input: string): string {
  return input.trim().replace(/^v/, '');
}
