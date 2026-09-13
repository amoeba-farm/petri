// Lossless runtime storage. V1 is per-file zlib; opt-in V2 shares bounded
// related-file blocks and can compress the manifest with the same decoder.
import fs from 'node:fs';
import path from 'node:path';
import { createHash } from 'node:crypto';
import { deflateSync, inflateSync } from 'node:zlib';

export const sha256 = bytes => createHash('sha256').update(bytes).digest('hex');
export const MAX_FILE_BYTES = 128 * 1024 * 1024;
export const MAX_EXPANDED_BYTES = 2 * 1024 * 1024 * 1024;
export const MAX_HEADER_BYTES = 16 * 1024 * 1024;
export const MAX_GROUP_BYTES = 4 * 1024 * 1024;
const COMPRESSED_HEADER = 1n << 63n;
const MAX_STORED_BLOCK_BYTES = MAX_FILE_BYTES + 64 * 1024;
const validPath = name => typeof name === 'string' && !/[\\:\0]/.test(name)
  && !name.split('/').some(p => !p || p === '.' || p === '..');
const digestPattern = /^[a-f0-9]{64}$/;

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
  const groups = new Map(), categories = {}, extents = new Map();
  let expandedBytes = 0, storedBytes = 0;
  for (const file of files) {
    expandedBytes += file.bytes;
    const extent = `${file.offset}:${file.length}`;
    const unique = !extents.has(extent);
    const references = extents.get(extent) ?? [];
    references.push(file);
    extents.set(extent, references);
    if (unique) storedBytes += file.length;
    const row = categories[category(file.path)] ??= { files: 0, expandedBytes: 0, storedBytes: 0, exclusiveStoredBytes: 0 };
    row.files++;
    row.expandedBytes += file.bytes;
    if (unique) row.storedBytes += file.length;
    const group = groups.get(file.sha256) ?? [];
    group.push(file);
    groups.set(file.sha256, group);
  }
  const storedExtents = [...extents.values()].map(references => {
    const first = references[0], owners = [...new Set(references.map(f => category(f.path)))].sort();
    if (owners.length === 1) categories[owners[0]].exclusiveStoredBytes += first.length;
    return { offset: first.offset, storedBytes: first.length,
      expandedBytes: first.blockBytes ?? first.bytes, references: references.length,
      categories: owners, paths: references.map(f => f.path),
      // Deleting one alias/member does not remove the enclosing stored extent.
      removableWholeExtentBytes: references.length === 1 ? first.length : 0 };
  });
  return {
    files: files.length, expandedBytes, storedBytes, uniquePayloads: extents.size, categories,
    largest: [...files].sort((a, b) => b.bytes - a.bytes || a.path.localeCompare(b.path)).slice(0, 25),
    storedAttribution: 'storedBytes charges each extent once to its first file; exclusiveStoredBytes excludes cross-category extents',
    crossCategoryStoredBytes: storedExtents.filter(e => e.categories.length > 1).reduce((n, e) => n + e.storedBytes, 0),
    largestStoredExtents: storedExtents.sort((a, b) => b.storedBytes - a.storedBytes || a.offset - b.offset).slice(0, 25),
    duplicates: [...groups.values()].filter(group => group.length > 1).map(group => ({
      sha256: group[0].sha256, bytes: group[0].bytes, paths: group.map(f => f.path),
      // Separate files remain present after extraction, so this is storage only.
      repeatedExpandedBytes: group[0].bytes * (group.length - 1),
    })),
  };
}

function relatedGroup(name) {
  // Executables, native add-ons, vendor archives and unresolved resources stay
  // independent. Only JS/JSON/legal bytes share a dictionary, never semantics.
  if (!/\.(?:[cm]?js|json)$/.test(name) && category(name) !== 'legal-required') return `file:${name}`;
  if (/^node(?:\.exe)?$|^compressed-verifier/.test(name)) return `file:${name}`;
  const dependency = name.match(/^sdk\/node_modules\/(@[^/]+\/[^/]+|[^/]+)/);
  return dependency ? dependency[0] : name.startsWith('sdk/dist/') ? 'sdk/dist' : 'root-text';
}

export function packRuntime(stage, names, metadata, {
  level = 9, deduplicate = true, groupBytes = 0, compressManifest = groupBytes > 0,
} = {}) {
  if (!Number.isInteger(level) || level < 0 || level > 9) throw new Error('Invalid compression level');
  if (!Number.isInteger(groupBytes) || groupBytes < 0 || groupBytes > MAX_GROUP_BYTES) throw new Error('Invalid runtime group bound');
  const version = groupBytes || compressManifest ? 2 : 1;
  const records = [], chunks = [], seen = new Set(), content = new Map();
  let offset = 0, expanded = 0, pending = [], pendingBytes = 0, pendingGroup;
  const flush = () => {
    if (!pending.length) return;
    const bytes = Buffer.concat(pending.map(file => file.bytes));
    const packed = deflateSync(bytes, { level }), blockSha256 = sha256(bytes);
    for (const file of pending) Object.assign(file.extent, {
      offset, length: packed.length, blockBytes: bytes.length, blockSha256,
    });
    chunks.push(packed);
    offset += packed.length;
    pending = []; pendingBytes = 0;
  };
  const sortedNames = [...names].sort((a, b) => {
    const left = groupBytes ? relatedGroup(a) : a, right = groupBytes ? relatedGroup(b) : b;
    return left < right ? -1 : left > right ? 1 : a < b ? -1 : a > b ? 1 : 0;
  });
  for (const name of sortedNames) {
    if (seen.has(name) || !validPath(name)) {
      throw new Error(`Invalid or duplicate runtime path: ${name}`);
    }
    seen.add(name);
    const location = path.join(stage, name);
    const stat = fs.lstatSync(location);
    if (!stat.isFile() || stat.isSymbolicLink() || stat.size > MAX_FILE_BYTES) throw new Error(`Invalid runtime file: ${name}`);
    const bytes = fs.readFileSync(location);
    expanded += bytes.length;
    if (expanded > MAX_EXPANDED_BYTES || seen.size > 40000) throw new Error('Runtime exceeds extraction bounds');
    const digest = sha256(bytes);
    const previous = deduplicate && content.get(digest);
    let extent;
    if (previous && previous.bytes === bytes.length && fs.readFileSync(path.join(stage, previous.path)).equals(bytes)) {
      extent = previous.extent;
    } else {
      const group = relatedGroup(name);
      if (!groupBytes || group !== pendingGroup || pendingBytes + bytes.length > groupBytes) flush();
      extent = { blockOffset: pendingBytes };
      pending.push({ bytes, extent });
      pendingBytes += bytes.length;
      pendingGroup = group;
      content.set(digest, { path: name, bytes: bytes.length, extent });
      if (!groupBytes || pendingBytes >= groupBytes || group.startsWith('file:')) flush();
    }
    records.push({ path: name, bytes: bytes.length, sha256: digest, extent });
  }
  flush();
  const files = records.sort((a, b) => a.path < b.path ? -1 : a.path > b.path ? 1 : 0).map(({ extent, ...file }) => ({
    ...file, offset: extent.offset, length: extent.length,
    ...(version === 2 ? { blockOffset: extent.blockOffset, blockBytes: extent.blockBytes, blockSha256: extent.blockSha256 } : {}),
  }));
  const manifest = { ...metadata, schemaVersion: version, files };
  const decodedHeader = Buffer.from(JSON.stringify(manifest));
  if (decodedHeader.length > MAX_HEADER_BYTES) throw new Error('Runtime manifest exceeds extraction bound');
  const header = compressManifest ? deflateSync(decodedHeader, { level }) : decodedHeader;
  if (header.length > MAX_HEADER_BYTES) throw new Error('Stored runtime manifest exceeds extraction bound');
  const size = Buffer.alloc(8);
  size.writeBigUInt64LE(BigInt(header.length) | (compressManifest ? COMPRESSED_HEADER : 0n));
  return { bundle: Buffer.concat([size, header, ...chunks]), manifest, inventory: {
    ...inventory(files), manifestStoredBytes: header.length,
    manifestExpandedBytes: decodedHeader.length, manifestCompression: compressManifest ? 'zlib' : 'none',
  } };
}

export function readRuntime(file) {
  return decodeRuntime(fs.readFileSync(file));
}

function inflateExact(bytes, maximum, expected) {
  const { buffer, engine } = inflateSync(bytes, { maxOutputLength: Math.max(1, maximum), info: true });
  if (engine.bytesWritten !== bytes.length || (expected !== undefined && buffer.length !== expected)) throw new Error('Invalid runtime compressed stream');
  return buffer;
}

export function decodeRuntime(bundle) {
  if (bundle.length < 8) throw new Error('Missing full SDK runtime');
  const word = bundle.readBigUInt64LE(), compressed = (word & COMPRESSED_HEADER) !== 0n;
  const size = word & ~COMPRESSED_HEADER;
  if (size > BigInt(MAX_HEADER_BYTES) || size + 8n > BigInt(bundle.length)) throw new Error('Invalid runtime header');
  const start = 8 + Number(size), header = bundle.subarray(8, start);
  const decodedHeader = compressed ? inflateExact(header, MAX_HEADER_BYTES) : header;
  const manifest = JSON.parse(decodedHeader.toString('utf8'));
  if (![1, 2].includes(manifest.schemaVersion) || (compressed && manifest.schemaVersion !== 2)
      || !Array.isArray(manifest.files) || !manifest.files.length || manifest.files.length > 40000) throw new Error('Invalid runtime manifest');
  const integer = (value, max) => Number.isSafeInteger(value) && value >= 0 && value <= max;
  const blocks = new Map(), paths = new Set();
  let expanded = 0;
  for (const file of manifest.files) {
    if (!validPath(file.path) || paths.has(file.path) || !integer(file.bytes, MAX_FILE_BYTES)
        || typeof file.sha256 !== 'string' || !digestPattern.test(file.sha256)) throw new Error('Invalid runtime file');
    paths.add(file.path);
    expanded += file.bytes;
    if (expanded > MAX_EXPANDED_BYTES || !integer(file.offset, bundle.length - start)
        || !integer(file.length, MAX_STORED_BLOCK_BYTES) || !file.length
        || file.offset + file.length > bundle.length - start) throw new Error('Invalid runtime extent');
    const bytes = manifest.schemaVersion === 1 ? file.bytes : file.blockBytes;
    const digest = manifest.schemaVersion === 1 ? file.sha256 : file.blockSha256;
    const slice = manifest.schemaVersion === 1 ? 0 : file.blockOffset;
    if (!integer(bytes, MAX_FILE_BYTES) || !integer(slice, bytes) || slice + file.bytes > bytes
        || typeof digest !== 'string' || !digestPattern.test(digest)) throw new Error('Invalid runtime block');
    const key = `${file.offset}:${file.length}`;
    const block = blocks.get(key) ?? { offset: file.offset, length: file.length, bytes, sha256: digest, files: [] };
    if (block.bytes !== bytes || block.sha256 !== digest) throw new Error('Inconsistent runtime block');
    block.files.push({ file, slice }); blocks.set(key, block);
  }
  const ordered = [...blocks.values()].sort((a, b) => a.offset - b.offset);
  let storedEnd = 0;
  for (const block of ordered) {
    if (block.offset !== storedEnd) throw new Error('Overlapping or incomplete runtime extents');
    storedEnd += block.length;
    let decodedEnd = 0, previous;
    for (const member of [...block.files].sort((a, b) => a.slice - b.slice || a.file.bytes - b.file.bytes)) {
      if (previous && member.slice === previous.slice && member.file.bytes === previous.file.bytes
          && member.file.sha256 === previous.file.sha256) continue;
      if (member.slice !== decodedEnd) throw new Error('Overlapping or incomplete runtime slices');
      decodedEnd += member.file.bytes; previous = member;
    }
    if (decodedEnd !== block.bytes) throw new Error('Invalid runtime expanded block');
  }
  if (storedEnd !== bundle.length - start) throw new Error('Trailing runtime payload');
  return { bundle, manifest, start, blocks: ordered, inventory: {
    ...inventory(manifest.files), manifestStoredBytes: header.length,
    manifestExpandedBytes: decodedHeader.length, manifestCompression: compressed ? 'zlib' : 'none',
  } };
}

// Extract at most one bounded block at a time. Consumers still write each path
// independently; deduplication never creates mutable hard links or symlinks.
export function* runtimeFiles(runtime) {
  for (const block of runtime.blocks) {
    const decoded = inflateExact(runtime.bundle.subarray(runtime.start + block.offset,
      runtime.start + block.offset + block.length), block.bytes, block.bytes);
    if (sha256(decoded) !== block.sha256) throw new Error('Runtime block checksum mismatch');
    for (const { file, slice } of block.files) {
      const bytes = decoded.subarray(slice, slice + file.bytes);
      if (sha256(bytes) !== file.sha256) throw new Error(`Runtime checksum mismatch: ${file.path}`);
      yield { entry: file, bytes };
    }
  }
}
