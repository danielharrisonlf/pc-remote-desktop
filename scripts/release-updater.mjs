#!/usr/bin/env node
// Prepares the Windows release assets for the in-app updater, after
//   npm run tauri build -- --target x86_64-pc-windows-msvc
// ran with TAURI_SIGNING_PRIVATE_KEY (and TAURI_SIGNING_PRIVATE_KEY_PASSWORD) set.
//
// Tauri writes "PC Remote_<v>_x64-setup.exe" and its minisign signature "...exe.sig". This
// script copies both, byte for byte, to the published names, verifies the signature against
// the public key in tauri.conf.json (including the signed version that `requireSignedVersion`
// enforces), and writes latest.json and WINDOWS-SHA256SUMS-<v>.txt. Renaming does not change
// the signed bytes, so the .sig stays valid for the uploaded file.
//
// Usage: npm run release:updater -- [--notes-file <file.md> | --notes <text>]
//          [--bundle-dir <dir>] [--out <dir>] [--pub-date <RFC 3339>]
// No dependencies beyond Node >= 22.

import { createHash, createPublicKey, verify } from 'node:crypto';
import { copyFileSync, existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { basename, dirname, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

export const REPOSITORY = 'danielharrisonlf/pc-remote-desktop';
export const PLATFORMS = ['windows-x86_64', 'windows-x86_64-nsis'];
const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const SEMVER = /^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$/;

/** The name Tauri gives the NSIS installer for `productName` "PC Remote". */
export const bundledInstallerName = version => `PC Remote_${version}_x64-setup.exe`;
/** The name published in the GitHub release and referenced by latest.json. */
export const installerAssetName = version => `pc-remote-desktop-${version}-x64-setup.exe`;
export const installerUrl = version =>
  `https://github.com/${REPOSITORY}/releases/download/v${version}/${installerAssetName(version)}`;
export const sumsName = version => `WINDOWS-SHA256SUMS-${version}.txt`;

const decodeLines = base64 => Buffer.from(base64.trim(), 'base64').toString('utf8').split(/\r?\n/);

/** A Tauri `pubkey` (base64 of a minisign public key file) as key id and Ed25519 key. */
export function parsePublicKey(pubkeyBase64) {
  const line = decodeLines(pubkeyBase64).find(entry => entry && !entry.startsWith('untrusted comment:'));
  const bytes = Buffer.from(line ?? '', 'base64');
  if (bytes.length !== 42 || bytes.subarray(0, 2).toString('latin1') !== 'Ed') throw new Error('Chave pública minisign inválida.');
  return { keyId: bytes.subarray(2, 10), key: bytes.subarray(10) };
}

/** A Tauri `.sig` (base64 of a minisign signature file). */
export function parseSignature(signatureBase64) {
  const lines = decodeLines(signatureBase64);
  const index = lines.findIndex(line => line.startsWith('trusted comment: '));
  if (index < 1) throw new Error('Assinatura minisign sem comentário confiável.');
  const signature = Buffer.from(lines[index - 1], 'base64');
  const global = Buffer.from(lines[index + 1] ?? '', 'base64');
  const algorithm = signature.subarray(0, 2).toString('latin1');
  if (signature.length !== 74 || global.length !== 64 || (algorithm !== 'Ed' && algorithm !== 'ED')) {
    throw new Error('Assinatura minisign malformada.');
  }
  return {
    prehashed: algorithm === 'ED',
    keyId: signature.subarray(2, 10),
    signature: signature.subarray(10),
    trustedComment: lines[index].slice('trusted comment: '.length),
    global,
  };
}

/** The `version:` field Tauri writes into the trusted comment (covered by the signature). */
export const signedVersion = trustedComment =>
  trustedComment.split('\t').find(field => field.startsWith('version:'))?.slice('version:'.length) ?? null;

const ed25519 = raw => createPublicKey({ key: { kty: 'OKP', crv: 'Ed25519', x: Buffer.from(raw).toString('base64url') }, format: 'jwk' });

/**
 * Minisign verification as the updater does it: key id, Ed25519 over the file (BLAKE2b-512
 * prehash for "ED"), and the global signature over signature + trusted comment.
 */
export function verifyMinisign(data, signatureBase64, pubkeyBase64) {
  try {
    const publicKey = parsePublicKey(pubkeyBase64);
    const parsed = parseSignature(signatureBase64);
    if (!publicKey.keyId.equals(parsed.keyId)) return { ok: false, reason: 'Assinatura feita com outra chave.' };
    const key = ed25519(publicKey.key);
    const message = parsed.prehashed ? createHash('blake2b512').update(data).digest() : data;
    if (!verify(null, message, key, parsed.signature)) return { ok: false, reason: 'A assinatura não corresponde ao arquivo.' };
    const global = Buffer.concat([parsed.signature, Buffer.from(parsed.trustedComment, 'utf8')]);
    if (!verify(null, global, key, parsed.global)) return { ok: false, reason: 'O comentário confiável foi alterado.' };
    return { ok: true, trustedComment: parsed.trustedComment, version: signedVersion(parsed.trustedComment) };
  } catch (error) {
    return { ok: false, reason: error instanceof Error ? error.message : String(error) };
  }
}

/** RFC 3339 in UTC without milliseconds, as the updater parses `pub_date`. */
export const rfc3339 = date => date.toISOString().replace(/\.\d{3}Z$/, 'Z');

/** The static manifest read from releases/latest/download/latest.json. */
export function buildManifest({ version, notes, pubDate, signature }) {
  if (!SEMVER.test(version)) throw new Error(`Versão inválida: ${version}`);
  const entry = { signature: signature.trim(), url: installerUrl(version) };
  return {
    version,
    notes: notes.replace(/\r\n/g, '\n').trim(),
    pub_date: pubDate,
    platforms: Object.fromEntries(PLATFORMS.map(platform => [platform, entry])),
  };
}

export const sha256 = data => createHash('sha256').update(data).digest('hex');

/** Same format as the earlier WINDOWS-SHA256SUMS files: "<hash>  <name>". */
export const sumsFile = entries => entries.map(([name, data]) => `${sha256(data)}  ${name}\n`).join('');

/** Reads the version from tauri.conf.json and checks package.json and Cargo.toml agree. */
export function projectVersion(root = ROOT) {
  const tauri = JSON.parse(readFileSync(join(root, 'src-tauri', 'tauri.conf.json'), 'utf8'));
  const pkg = JSON.parse(readFileSync(join(root, 'package.json'), 'utf8'));
  const cargo = /^version\s*=\s*"([^"]+)"/m.exec(readFileSync(join(root, 'src-tauri', 'Cargo.toml'), 'utf8'))?.[1];
  const versions = { 'tauri.conf.json': tauri.version, 'package.json': pkg.version, 'Cargo.toml': cargo };
  if (new Set(Object.values(versions)).size !== 1) throw new Error(`Versões divergentes: ${JSON.stringify(versions)}`);
  const pubkey = tauri.plugins?.updater?.pubkey;
  if (!pubkey) throw new Error('tauri.conf.json não tem plugins.updater.pubkey.');
  return { version: tauri.version, pubkey, requireSignedVersion: tauri.plugins.updater.requireSignedVersion === true };
}

/**
 * Copies the signed installer to its published name and writes the manifest and checksums.
 * Throws before writing anything if the signature does not verify.
 */
export function prepareRelease({ bundleDir, outDir, version, pubkey, notes, pubDate, requireSignedVersion = true }) {
  const source = join(bundleDir, bundledInstallerName(version));
  const signaturePath = `${source}.sig`;
  for (const file of [source, signaturePath]) {
    if (!existsSync(file)) throw new Error(`Não encontrado: ${file}. Rode a build com TAURI_SIGNING_PRIVATE_KEY definida.`);
  }
  const installer = readFileSync(source);
  const signature = readFileSync(signaturePath, 'utf8').trim();
  if (installer.subarray(0, 2).toString('latin1') !== 'MZ') throw new Error(`${basename(source)} não é um executável do Windows.`);
  const checked = verifyMinisign(installer, signature, pubkey);
  if (!checked.ok) throw new Error(`Assinatura recusada: ${checked.reason}`);
  if (checked.version !== version && (requireSignedVersion || checked.version !== null)) {
    throw new Error(`A assinatura é da versão ${checked.version ?? '(nenhuma)'}, não da ${version}.`);
  }

  mkdirSync(outDir, { recursive: true });
  const assetName = installerAssetName(version);
  const target = join(outDir, assetName);
  copyFileSync(source, target);
  if (sha256(readFileSync(target)) !== sha256(installer)) throw new Error('A cópia do instalador difere do original.');
  writeFileSync(join(outDir, `${assetName}.sig`), signature);
  const manifest = buildManifest({ version, notes, pubDate, signature });
  const manifestText = `${JSON.stringify(manifest, null, 2)}\n`;
  writeFileSync(join(outDir, 'latest.json'), manifestText);
  const sums = sumsFile([
    [assetName, installer],
    [`${assetName}.sig`, Buffer.from(signature)],
    ['latest.json', Buffer.from(manifestText)],
  ]);
  writeFileSync(join(outDir, sumsName(version)), sums);
  return {
    outDir,
    trustedComment: checked.trustedComment,
    files: [assetName, `${assetName}.sig`, 'latest.json', sumsName(version)].map(name => {
      const data = readFileSync(join(outDir, name));
      return { name, bytes: data.length, sha256: sha256(data) };
    }),
  };
}

function parseArgs(argv) {
  const options = {};
  for (let index = 0; index < argv.length; index += 1) {
    const flag = argv[index];
    const value = argv[index + 1];
    if (!['--notes', '--notes-file', '--bundle-dir', '--out', '--pub-date'].includes(flag) || value === undefined) {
      throw new Error(`Opção desconhecida ou sem valor: ${flag}`);
    }
    options[flag.slice(2)] = value;
    index += 1;
  }
  return options;
}

function main() {
  const options = parseArgs(process.argv.slice(2));
  const { version, pubkey, requireSignedVersion } = projectVersion();
  const notes = options['notes-file'] ? readFileSync(resolve(options['notes-file']), 'utf8') : (options.notes ?? '');
  if (!notes.trim()) console.warn('Aviso: latest.json sem notas. Use --notes-file <arquivo.md>.');
  const pubDate = options['pub-date'] ?? rfc3339(new Date());
  if (Number.isNaN(Date.parse(pubDate)) || !/Z$|[+-]\d\d:\d\d$/.test(pubDate)) throw new Error(`pub_date não é RFC 3339: ${pubDate}`);
  const bundleDir = resolve(options['bundle-dir'] ?? join(ROOT, 'src-tauri', 'target', 'x86_64-pc-windows-msvc', 'release', 'bundle', 'nsis'));
  const outDir = resolve(options.out ?? join(ROOT, 'release-assets', version));
  const result = prepareRelease({ bundleDir, outDir, version, pubkey, notes, pubDate, requireSignedVersion });
  console.log(`Assinatura conferida com a chave de tauri.conf.json (${result.trustedComment}).`);
  console.log(`Arquivos em ${result.outDir}:`);
  for (const file of result.files) console.log(`  ${file.name}  ${file.bytes} bytes  sha256 ${file.sha256}`);
  console.log('Envie os quatro arquivos para a release v' + version + '; latest.json deve manter este nome.');
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  try {
    main();
  } catch (error) {
    console.error(error instanceof Error ? error.message : error);
    process.exit(1);
  }
}
