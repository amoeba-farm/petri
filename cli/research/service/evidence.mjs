import { openSync, fstatSync, readSync, closeSync } from 'node:fs';
import { resolve } from 'node:path';
import { fail } from './security.mjs';

// Web proofs (TLSNotary, `petri evidence prove`; evidence/README.md): a page
// behind the owner's sign-in, proven from their own browser session. The
// workspace never opens a bundle's proof: it checks the file is a bundle and
// hands its bytes to the futures API unchanged, which verifies it
// (POST /oracle/evidence) before the candidate names it.

export const BUNDLE_FORMAT = 'amoeba-evidence-bundle-v1';
// The verifier refuses a larger bundle (evidence/crates/evidence/src/policy.rs,
// DEFAULT_LIMITS.max_bundle_bytes), so petri refuses it before uploading.
export const MAX_BUNDLE_BYTES = 2 * 1024 * 1024;
// The workspace API carries a bundle as base64 of its bytes, next to the
// candidate, on top of its ordinary 32 KiB request bound.
export const MAX_BUNDLE_BASE64 = 4 * Math.ceil(MAX_BUNDLE_BYTES / 3);
export const MAX_SUBMISSION_BODY = 32768 + MAX_BUNDLE_BASE64;

const tooLarge = () => fail('BUNDLE_TOO_LARGE', 'This evidence bundle is larger than 2 MiB, more than a web proof can verify. Prove an endpoint with a smaller response.', 413);
const notBundle = () => fail('INVALID_BUNDLE', 'This is not an evidence bundle. Give the file petri evidence prove wrote (its --out file).');

// The bytes of a bundle as petri wrote it: a JSON object in the bundle format.
export function bundleBytes(bytes) {
  if (bytes.length > MAX_BUNDLE_BYTES) throw tooLarge();
  let value;
  try { value = JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(bytes)); } catch { throw notBundle(); }
  if (!value || typeof value !== 'object' || Array.isArray(value) || value.format !== BUNDLE_FORMAT) throw notBundle();
  return bytes;
}

// Reads at most one byte past the bound, whatever the file claims its size is.
export function readBundleFile(path) {
  if (typeof path !== 'string' || !path.trim()) throw notBundle();
  let fd;
  try { fd = openSync(resolve(path), 'r'); }
  catch { throw fail('BUNDLE_NOT_FOUND', 'Petri could not open that evidence bundle. Give the path petri evidence prove wrote (its --out file).'); }
  try {
    if (!fstatSync(fd).isFile()) throw notBundle();
    if (fstatSync(fd).size > MAX_BUNDLE_BYTES) throw tooLarge();
    const buffer = Buffer.alloc(MAX_BUNDLE_BYTES + 1);
    let length = 0, read;
    while (length < buffer.length && (read = readSync(fd, buffer, length, buffer.length - length, null)) > 0) length += read;
    return bundleBytes(buffer.subarray(0, length));
  } finally { closeSync(fd); }
}

export const encodeBundle = bytes => Buffer.from(bytes).toString('base64');
export function decodeBundle(text) {
  if (typeof text !== 'string' || text.length > MAX_BUNDLE_BASE64) throw tooLarge();
  if (!text || text.length % 4 || !/^[A-Za-z0-9+/]*={0,2}$/u.test(text)) throw fail('INVALID_BUNDLE', 'Invalid evidence bundle encoding.');
  return bundleBytes(Buffer.from(text, 'base64'));
}
