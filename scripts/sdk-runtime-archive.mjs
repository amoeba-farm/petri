// Version-1 runtime storage: independent zlib streams with content-addressed
// extents. Aliased extents still extract to independent, hash-checked files.
import fs from 'node:fs';
import path from 'node:path';
import { createHash } from 'node:crypto';
import { deflateSync } from 'node:zlib';

export const sha256 = bytes => createHash('sha256').update(bytes).digest('hex');
export const MAX_FILE_BYTES = 128 * 1024 * 1024;
export const MAX_EXPANDED_BYTES = 2 * 1024 * 1024 * 1024;
export const MAX_HEADER_BYTES = 16 * 1024 * 1024;

export function category(name) {
  if (/^node(\.exe)?$/.test(name)) return 'node';
  if (/^compressed-verifier/.test(name)) return 'native-verifier';
  if (/(^|\/)(?:[^/]*LICENSE[^/]*|NOTICE[^/]*|COPYING[^/]*)$/i.test(name)) return 'legal-required';
  if (/\.tgz$/.test(name)) return 'installation-archive-unresolved';
  if (/\.node$/.test(name)) return 'native-dependency';
  if (!name.includes('/') && name.endsWith('.mjs')) return 'worker';
  if (name.startsWith('sdk/dist/')) return 'sdk';
  if (name.startsWith('sdk/node_modules/')) return 'dependency';
  return 'resource-unresolved';
}

export function inventory(files) {
  const groups = new Map(), categories = {}, extents = new Set();
  let expandedBytes = 0, storedBytes = 0;
  for (const file of files) {
    expandedBytes += file.bytes;
    const extent = `${file.offset}:${file.length}`;
    const unique = !extents.has(extent);
    extents.add(extent);
    if (unique) storedBytes += file.length;
    const row = categories[category(file.path)] ??= { files: 0, expandedBytes: 0, storedBytes: 0 };
    row.files++;
    row.expandedBytes += file.bytes;
    if (unique) row.storedBytes += file.length;
    const group = groups.get(file.sha256) ?? [];
    group.push(file);
    groups.set(file.sha256, group);
  }
  return {
    files: files.length, expandedBytes, storedBytes, uniquePayloads: extents.size, categories,
    largest: [...files].sort((a, b) => b.bytes - a.bytes || a.path.localeCompare(b.path)).slice(0, 25),
    duplicates: [...groups.values()].filter(group => group.length > 1).map(group => ({
      sha256: group[0].sha256, bytes: group[0].bytes, paths: group.map(f => f.path),
      // Separate files remain present after extraction, so this is storage only.
      repeatedExpandedBytes: group[0].bytes * (group.length - 1),
    })),
  };
}

export function packRuntime(stage, names, metadata, { level = 9, deduplicate = true } = {}) {
  const files = [], chunks = [], seen = new Set(), content = new Map();
  let offset = 0, expanded = 0;
  for (const name of [...names].sort()) {
    if (seen.has(name) || /[\\:]/.test(name) || name.split('/').some(p => !p || p === '.' || p === '..')) {
      throw new Error(`Invalid or duplicate runtime path: ${name}`);
    }
    seen.add(name);
    const location = path.join(stage, name);
    const stat = fs.lstatSync(location);
    if (!stat.isFile() || stat.isSymbolicLink() || stat.size > MAX_FILE_BYTES) throw new Error(`Invalid runtime file: ${name}`);
    const bytes = fs.readFileSync(location);
    expanded += bytes.length;
    if (expanded > MAX_EXPANDED_BYTES || seen.size > 40000) throw new Error('Runtime exceeds extraction bounds');
    const digest = sha256(bytes), packed = deflateSync(bytes, { level });
    const previous = deduplicate && content.get(digest);
    let extent;
    if (previous && previous.bytes === bytes.length && previous.packed.equals(packed)) {
      extent = previous;
    } else {
      extent = { offset, length: packed.length, bytes: bytes.length, packed };
      content.set(digest, extent);
      chunks.push(packed);
      offset += packed.length;
    }
    files.push({ path: name, bytes: bytes.length, sha256: digest, offset: extent.offset, length: extent.length });
  }
  const manifest = { ...metadata, schemaVersion: 1, files };
  const header = Buffer.from(JSON.stringify(manifest));
  if (header.length > MAX_HEADER_BYTES) throw new Error('Runtime manifest exceeds extraction bound');
  const size = Buffer.alloc(8);
  size.writeBigUInt64LE(BigInt(header.length));
  return { bundle: Buffer.concat([size, header, ...chunks]), manifest, inventory: inventory(files) };
}

export function readRuntime(file) {
  const bundle = fs.readFileSync(file);
  if (bundle.length < 8) throw new Error('Missing full SDK runtime');
  const size = bundle.readBigUInt64LE();
  if (size > BigInt(MAX_HEADER_BYTES) || size + 8n > BigInt(bundle.length)) throw new Error('Invalid runtime header');
  const manifest = JSON.parse(bundle.subarray(8, 8 + Number(size)).toString('utf8'));
  if (manifest.schemaVersion !== 1 || !Array.isArray(manifest.files) || !manifest.files.length) throw new Error('Invalid runtime manifest');
  return { bundle, manifest, inventory: inventory(manifest.files) };
}
