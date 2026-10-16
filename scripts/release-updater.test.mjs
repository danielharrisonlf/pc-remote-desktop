import { createHash, generateKeyPairSync, randomBytes, sign } from 'node:crypto';
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';
import {
  buildManifest,
  bundledInstallerName,
  installerAssetName,
  parseSignature,
  prepareRelease,
  projectVersion,
  rfc3339,
  sha256,
  signedVersion,
  verifyMinisign,
} from './release-updater.mjs';

// Same fixture as src-tauri/tests/updater.rs: signed by `tauri signer sign --app-version 0.5.1`
// with a throwaway key whose private half was discarded.
const FIXTURE = Buffer.from('PC Remote updater test fixture: not an installer.\n');
const FIXTURE_PUBKEY =
  'dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEYzNDFBMzg5OTJEN0M3NEMKUldSTXg5ZVNpYU5CODdidFpVbkZSdjV0bGRWVVUvaDVtZEs1aUtnRTJlK25vN0IvclhUMnp2TDcK';
const FIXTURE_SIGNATURE =
  'dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIHRhdXJpIHNlY3JldCBrZXkKUlVSTXg5ZVNpYU5COHdaeXA3a2pQbC9weWhmSGlkcERDV0c0eGJ2TmNLRGNPOVFuTS80T1NJQkp0YnptdzY3ajZYQ3RTNlY3aDFrTXg5MkFFMVN4T0xxdCtPYmdQV2lHWGdFPQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNzkyMTU4NzA5CWZpbGU6Zml4dHVyZS5iaW4JdmVyc2lvbjowLjUuMQpQV1A0ODZiMlhzZzBOR2dtb0w1b0ExcDJIQTBYSUxzU0Y2aXNzMDR1VEJCUGJDSlRGczhtc1MwekQwYWRBUTJIOGljT2JNQUdmai90UWdwYTVlRmFEQT09Cg==';

/** Signs like `tauri signer sign`: prehashed minisign ("ED") with a trusted comment. */
function testKey() {
  const { publicKey, privateKey } = generateKeyPairSync('ed25519');
  const keyId = randomBytes(8);
  const raw = Buffer.from(publicKey.export({ format: 'jwk' }).x, 'base64url');
  const pubkey = Buffer.from(
    `untrusted comment: minisign public key: TEST\n${Buffer.concat([Buffer.from('Ed'), keyId, raw]).toString('base64')}\n`,
  ).toString('base64');
  const signFile = (data, trusted) => {
    const signature = sign(null, createHash('blake2b512').update(data).digest(), privateKey);
    const global = sign(null, Buffer.concat([signature, Buffer.from(trusted)]), privateKey);
    const text = [
      'untrusted comment: signature from tauri secret key',
      Buffer.concat([Buffer.from('ED'), keyId, signature]).toString('base64'),
      `trusted comment: ${trusted}`,
      global.toString('base64'),
      '',
    ].join('\n');
    return Buffer.from(text).toString('base64');
  };
  return { pubkey, signFile };
}

const temporary = [];
afterEach(() => {
  for (const dir of temporary.splice(0)) rmSync(dir, { recursive: true, force: true });
});

function bundle(version, { installer, signature }) {
  const dir = mkdtempSync(join(tmpdir(), 'pcremote-release-'));
  temporary.push(dir);
  const bundleDir = join(dir, 'nsis');
  mkdirSync(bundleDir);
  writeFileSync(join(bundleDir, bundledInstallerName(version)), installer);
  writeFileSync(join(bundleDir, `${bundledInstallerName(version)}.sig`), signature);
  return { bundleDir, outDir: join(dir, 'out') };
}

describe('minisign verification', () => {
  it('accepts a signature made by the Tauri CLI and reads its signed version', () => {
    const result = verifyMinisign(FIXTURE, FIXTURE_SIGNATURE, FIXTURE_PUBKEY);
    expect(result).toMatchObject({ ok: true, version: '0.5.1' });
    expect(parseSignature(FIXTURE_SIGNATURE).prehashed).toBe(true);
  });

  it('rejects tampered bytes, another key and an edited trusted comment', () => {
    const tampered = Buffer.from(FIXTURE);
    tampered[0] ^= 1;
    expect(verifyMinisign(tampered, FIXTURE_SIGNATURE, FIXTURE_PUBKEY).ok).toBe(false);

    const releaseKey = projectVersion().pubkey;
    expect(verifyMinisign(FIXTURE, FIXTURE_SIGNATURE, releaseKey)).toMatchObject({ ok: false, reason: 'Assinatura feita com outra chave.' });

    const edited = Buffer.from(FIXTURE_SIGNATURE, 'base64').toString('utf8').replace('version:0.5.1', 'version:0.9.9');
    const result = verifyMinisign(FIXTURE, Buffer.from(edited).toString('base64'), FIXTURE_PUBKEY);
    expect(result).toMatchObject({ ok: false, reason: 'O comentário confiável foi alterado.' });
    expect(verifyMinisign(FIXTURE, 'bm90IGEgc2lnbmF0dXJl', FIXTURE_PUBKEY).ok).toBe(false);
  });

  it('reads the version field from tab separated trusted comments', () => {
    expect(signedVersion('timestamp:1\tfile:a.exe\tversion:0.5.0')).toBe('0.5.0');
    expect(signedVersion('timestamp:1\tfile:a.exe')).toBeNull();
  });
});

describe('latest.json', () => {
  it('points every Windows target at the renamed installer of that version', () => {
    const manifest = buildManifest({ version: '0.5.0', notes: '- Um\r\n- Dois\n', pubDate: '2026-10-16T12:00:00Z', signature: 'c2ln\n' });
    const entry = {
      signature: 'c2ln',
      url: 'https://github.com/danielharrisonlf/pc-remote-desktop/releases/download/v0.5.0/pc-remote-desktop-0.5.0-x64-setup.exe',
    };
    expect(manifest).toEqual({
      version: '0.5.0',
      notes: '- Um\n- Dois',
      pub_date: '2026-10-16T12:00:00Z',
      platforms: { 'windows-x86_64': entry, 'windows-x86_64-nsis': entry },
    });
    expect(() => buildManifest({ version: 'v0.5', notes: '', pubDate: '', signature: '' })).toThrow();
    expect(rfc3339(new Date(Date.UTC(2026, 9, 16, 12, 30, 5, 123)))).toBe('2026-10-16T12:30:05Z');
  });

  it('keeps package.json, Cargo.toml and tauri.conf.json on one version', () => {
    const project = projectVersion();
    expect(project.version).toMatch(/^\d+\.\d+\.\d+$/);
    expect(project.requireSignedVersion).toBe(true);
  });
});

describe('release assets', () => {
  const installer = Buffer.concat([Buffer.from('MZ'), randomBytes(4096)]);

  it('copies the signed installer to the published name and lists every asset', () => {
    const { pubkey, signFile } = testKey();
    const signature = signFile(installer, `timestamp:1792158709\tfile:${bundledInstallerName('0.5.0')}\tversion:0.5.0`);
    const { bundleDir, outDir } = bundle('0.5.0', { installer, signature });
    const result = prepareRelease({ bundleDir, outDir, version: '0.5.0', pubkey, notes: 'Notas', pubDate: '2026-10-16T12:00:00Z' });

    const name = installerAssetName('0.5.0');
    expect(result.files.map(file => file.name)).toEqual([name, `${name}.sig`, 'latest.json', 'WINDOWS-SHA256SUMS-0.5.0.txt']);
    expect(readFileSync(join(outDir, name)).equals(installer)).toBe(true);
    const published = readFileSync(join(outDir, `${name}.sig`), 'utf8');
    expect(published).toBe(signature);
    expect(verifyMinisign(readFileSync(join(outDir, name)), published, pubkey).ok).toBe(true);
    const manifest = JSON.parse(readFileSync(join(outDir, 'latest.json'), 'utf8'));
    expect(manifest.platforms['windows-x86_64'].signature).toBe(signature);
    const sums = readFileSync(join(outDir, 'WINDOWS-SHA256SUMS-0.5.0.txt'), 'utf8');
    expect(sums.split('\n')[0]).toBe(`${sha256(installer)}  ${name}`);
    expect(sums).toContain(`${sha256(readFileSync(join(outDir, 'latest.json')))}  latest.json`);
  });

  it('refuses tampered installers, other versions and non-executables', () => {
    const { pubkey, signFile } = testKey();
    const comment = version => `timestamp:1\tfile:x.exe\tversion:${version}`;
    const options = { version: '0.5.0', pubkey, notes: '', pubDate: '2026-10-16T12:00:00Z' };

    const tampered = Buffer.from(installer);
    tampered[100] ^= 1;
    const a = bundle('0.5.0', { installer: tampered, signature: signFile(installer, comment('0.5.0')) });
    expect(() => prepareRelease({ ...a, ...options })).toThrow('A assinatura não corresponde ao arquivo.');

    const b = bundle('0.5.0', { installer, signature: signFile(installer, comment('0.4.0')) });
    expect(() => prepareRelease({ ...b, ...options })).toThrow('A assinatura é da versão 0.4.0, não da 0.5.0.');

    const c = bundle('0.5.0', { installer, signature: signFile(installer, 'timestamp:1\tfile:x.exe') });
    expect(() => prepareRelease({ ...c, ...options })).toThrow('A assinatura é da versão (nenhuma), não da 0.5.0.');

    const text = Buffer.from('not a PE file');
    const d = bundle('0.5.0', { installer: text, signature: signFile(text, comment('0.5.0')) });
    expect(() => prepareRelease({ ...d, ...options })).toThrow('não é um executável do Windows');
  });
});
