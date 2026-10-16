import { describe, expect, it } from 'vitest';
import { compareVersions, displayVersion, isNewerVersion, parseVersion } from './semver';

const order = (a: string, b: string) => compareVersions(parseVersion(a)!, parseVersion(b)!);

describe('semantic version comparison', () => {
  it('parses tags with or without the v prefix', () => {
    expect(parseVersion('v0.2.2')).toEqual({ major: 0, minor: 2, patch: 2, prerelease: [] });
    expect(parseVersion('1.10.0-rc.1+build.5')).toEqual({ major: 1, minor: 10, patch: 0, prerelease: ['rc', '1'] });
    expect(parseVersion('0.2')).toBeNull();
    expect(parseVersion('latest')).toBeNull();
  });

  it('compares numerically, not as text', () => {
    expect(order('0.10.0', '0.9.9')).toBe(1);
    expect(order('1.0.0', '1.0.0')).toBe(0);
    expect(order('0.2.2', '0.3.0')).toBe(-1);
  });

  it('ranks prereleases below the release, following SemVer 2.0', () => {
    expect(order('1.0.0-alpha', '1.0.0')).toBe(-1);
    expect(order('1.0.0', '1.0.0-rc.1')).toBe(1);
    expect(order('1.0.0-alpha', '1.0.0-alpha.1')).toBe(-1);
    expect(order('1.0.0-alpha.beta', '1.0.0-beta')).toBe(-1);
    expect(order('1.0.0-beta.2', '1.0.0-beta.11')).toBe(-1);
    expect(order('1.0.0-rc.1', '1.0.0-beta.11')).toBe(1);
  });

  it('only reports newer when both versions are valid', () => {
    expect(isNewerVersion('v0.3.0', '0.2.2')).toBe(true);
    expect(isNewerVersion('v0.2.2', '0.2.2')).toBe(false);
    expect(isNewerVersion('garbage', '0.2.2')).toBe(false);
    expect(displayVersion(' v0.3.0 ')).toBe('0.3.0');
  });
});
