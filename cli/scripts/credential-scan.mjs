import { createHash } from "node:crypto";

// Fingerprints are safe to publish internally; the compromised credential is not.
const forbiddenCredentialSha256Prefixes = new Set(["f767596811f8"]);
const knownCompromisedCredentialLengths = new Set([36]);
const tokenRunPattern = /[A-Za-z0-9_-]{32,}/g;

function sha256(value) {
  return createHash("sha256").update(value).digest("hex");
}

function assertNoCredentialFingerprints(bytes, file, scope, prefixes, lengths) {
  const text = Buffer.isBuffer(bytes) ? bytes.toString("utf8") : String(bytes);
  for (const match of text.matchAll(tokenRunPattern)) {
    for (const length of lengths) {
      if (match[0].length < length) continue;
      for (let start = 0; start <= match[0].length - length; start += 1) {
        const digest = sha256(match[0].slice(start, start + length));
        for (const prefix of prefixes) {
          if (digest.startsWith(prefix)) {
            throw new Error(`${scope} contains a known compromised credential fingerprint: ${file}`);
          }
        }
      }
    }
  }
}

export function assertNoKnownCompromisedCredential(bytes, file, scope) {
  assertNoCredentialFingerprints(
    bytes,
    file,
    scope,
    forbiddenCredentialSha256Prefixes,
    knownCompromisedCredentialLengths,
  );
}

export function assertCredentialScannerSelfTest() {
  const canary = "petri-credential-scanner-canary-0001";
  const prefix = sha256(canary).slice(0, 12);
  try {
    assertNoCredentialFingerprints(
      `adjacent_${canary}_text`,
      "self-test",
      "credential scanner self-test",
      new Set([prefix]),
      new Set([canary.length]),
    );
  } catch (error) {
    const message = String(error?.message ?? error);
    if (message.includes(canary)) {
      throw new Error("credential scanner self-test exposed its token in an error");
    }
    if (message === "credential scanner self-test contains a known compromised credential fingerprint: self-test") {
      return;
    }
    throw error;
  }
  throw new Error("credential scanner self-test did not reject its fingerprint");
}
