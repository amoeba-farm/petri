import { createCipheriv, createDecipheriv, createHash, randomBytes, timingSafeEqual } from 'node:crypto';
import { mkdirSync, lstatSync, chmodSync, readFileSync, writeFileSync, existsSync, readdirSync, renameSync, openSync, writeSync, fsyncSync, closeSync } from 'node:fs';
import { resolve, join } from 'node:path';
import { homedir } from 'node:os';
import { execFileSync } from 'node:child_process';

export const token = () => randomBytes(32).toString('hex');
export const digest = value => createHash('sha256').update(value).digest('hex');
export function equalSecret(a, b) {
  return typeof a === 'string' && typeof b === 'string' && timingSafeEqual(Buffer.from(digest(a)), Buffer.from(digest(b)));
}
export function fail(code, message, status = 400) { return Object.assign(new Error(message), { code, status }); }
export function text(value, maximum, label) {
  if (typeof value !== 'string' || !value.trim() || Buffer.byteLength(value) > maximum || /[\x00-\x08\x0b-\x1f\x7f-\x9f\u202a-\u202e\u2066-\u2069]/u.test(value)) {
    throw fail('INVALID_INPUT', `Provide a valid ${label}.`);
  }
  return value.trim();
}
export function object(body, keys, required = keys) {
  if (!body || typeof body !== 'object' || Array.isArray(body) || Object.keys(body).some(key => !keys.includes(key)) || required.some(key => !(key in body))) {
    throw fail('INVALID_INPUT', 'Unexpected or missing request fields.');
  }
  return body;
}
export function endpoint(value) {
  let url;
  try { url = new URL(value); } catch { throw fail('INVALID_ENDPOINT', 'Provide an HTTPS or loopback API address.'); }
  if (url.username || url.password || url.search || url.hash || url.pathname !== '/' ||
      !(url.protocol === 'https:' || url.protocol === 'http:' && ['127.0.0.1', '[::1]', 'localhost'].includes(url.hostname))) {
    throw fail('INVALID_ENDPOINT', 'API addresses require HTTPS, except on loopback, and must not contain credentials or a path.');
  }
  return url.origin;
}
export const stateDirectory = () => resolve(process.env.PETRI_RESEARCH_HOME || join(process.env.LOCALAPPDATA || join(homedir(), '.local', 'share'), 'PetriResearch'));
export function privateDirectory(directory) {
  const path = resolve(directory);
  if (existsSync(path) && !existsSync(join(path, '.petri-research-storage')) && readdirSync(path).length) {
    throw fail('UNSAFE_STORAGE', 'Choose an empty directory dedicated to Petri research.');
  }
  mkdirSync(path, { recursive: true, mode: 0o700 });
  if (lstatSync(path).isSymbolicLink() || !lstatSync(path).isDirectory()) throw fail('UNSAFE_STORAGE', 'Private storage must be a normal directory.');
  if (process.platform === 'win32') {
    const identity = execFileSync('whoami.exe', [], { encoding: 'utf8', windowsHide: true }).trim();
    execFileSync('icacls.exe', [path, '/inheritance:r', '/grant:r', `${identity}:(OI)(CI)F`], { stdio: 'pipe', windowsHide: true });
  } else chmodSync(path, 0o700);
  if (!existsSync(join(path, '.petri-research-storage'))) writeFileSync(join(path, '.petri-research-storage'), 'Petri research private storage\n', { mode: 0o600, flag: 'wx' });
  return path;
}
export function privateWrite(path, value, { exclusive = false } = {}) {
  if (existsSync(path) && lstatSync(path).isSymbolicLink()) throw fail('UNSAFE_STORAGE', 'Credential files must not be symbolic links.');
  const destination = exclusive ? path : `${path}.${randomBytes(8).toString('hex')}.tmp`;
  const fd = openSync(destination, 'wx', 0o600);
  try { writeSync(fd, `${JSON.stringify(value, null, 2)}\n`); fsyncSync(fd); } finally { closeSync(fd); }
  if (!exclusive) renameSync(destination, path);
  if (process.platform !== 'win32') chmodSync(path, 0o600);
}
// The futures API serves both the candidate store (`/oracle/*`) and the oracle
// reads (`/v1/oracle/*`). Petri always passes the API it uses for markets
// (PETRI_RESEARCH_UPSTREAM); this default is production's, like a release
// petri's.
export const DEFAULT_FUTURES_API = 'https://api.amoeba.farm';
// The retired local candidate API. A workspace still bound to it is rebound to
// the futures API; nothing else rebinds silently.
const RETIRED_ENDPOINTS = new Set(['http://127.0.0.1:4184']);
const bound = (stored, requested) => {
  const current = stored ? endpoint(stored) : undefined;
  if (!current || RETIRED_ENDPOINTS.has(current)) return endpoint(requested || DEFAULT_FUTURES_API);
  return current;
};
export function loadOwner(directory, upstream = undefined, port = undefined, oracleReadUrl = undefined) {
  const root = privateDirectory(directory), path = join(root, 'owner.json');
  if (!existsSync(path)) privateWrite(path, { version: 1, ownerToken: token(), masterKey: token(),
    upstream: endpoint(upstream || DEFAULT_FUTURES_API), port: port || 4185 }, { exclusive: true });
  if (lstatSync(path).isSymbolicLink()) throw fail('UNSAFE_STORAGE', 'Invalid owner configuration.');
  const config = JSON.parse(readFileSync(path, 'utf8'));
  if (config.version !== 1 || !/^[0-9a-f]{64}$/u.test(config.ownerToken) || !/^[0-9a-f]{64}$/u.test(config.masterKey)) throw fail('INVALID_CONFIG', 'Invalid workspace configuration.');
  const originalUpstream = bound(config.upstream, upstream);
  if (upstream && endpoint(upstream) !== originalUpstream) throw fail('UPSTREAM_MISMATCH', 'This workspace is bound to another candidate API. Use a separate state directory.');
  config.upstream = originalUpstream;
  const originalOracle = bound(config.oracleReadUrl, oracleReadUrl || config.upstream);
  if (oracleReadUrl && endpoint(oracleReadUrl) !== originalOracle) throw fail('ORACLE_MISMATCH', 'Use a separate workspace for a different oracle read endpoint.');
  config.oracleReadUrl = originalOracle;
  config.port = port ?? config.port;
  if (!Number.isInteger(config.port) || config.port < 1024 || config.port > 65535) throw fail('INVALID_PORT', 'Choose a port from 1024 to 65535.');
  privateWrite(path, config);
  return { ...config, directory: root, baseUrl: `http://127.0.0.1:${config.port}` };
}
export function vault(masterKey) {
  const key = Buffer.from(masterKey, 'hex');
  return {
    seal(value, purpose) {
      const iv = randomBytes(12), cipher = createCipheriv('aes-256-gcm', key, iv);
      cipher.setAAD(Buffer.from(purpose));
      const data = Buffer.concat([cipher.update(value, 'utf8'), cipher.final()]);
      return Buffer.concat([iv, cipher.getAuthTag(), data]).toString('base64');
    },
    open(value, purpose) {
      const bytes = Buffer.from(value, 'base64'), decipher = createDecipheriv('aes-256-gcm', key, bytes.subarray(0, 12));
      decipher.setAAD(Buffer.from(purpose)); decipher.setAuthTag(bytes.subarray(12, 28));
      return Buffer.concat([decipher.update(bytes.subarray(28)), decipher.final()]).toString('utf8');
    },
  };
}
