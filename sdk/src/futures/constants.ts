/**
 * Program ids, limits and error helpers shared by the futures client
 * and the oracle `Period` PDA.
 */
import { PublicKey } from "@solana/web3.js";

export const SYSTEM_PROGRAM_ID = new PublicKey("11111111111111111111111111111111");
export const TOKEN_PROGRAM_ID = new PublicKey("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");
export const BPF_LOADER_UPGRADEABLE_PROGRAM_ID = new PublicKey("BPFLoaderUpgradeab1e11111111111111111111111");

/** Oracle `Period` PDA: `[PERIOD_NAMESPACE, PERIOD_SEED, product_id, expiry u64 LE]`. */
export const PERIOD_NAMESPACE = "ameba-oracle-v1";
export const PERIOD_SEED = "period";

/** Recommended compute limit for every trading transaction. */
export const FUTURES_COMPUTE_UNIT_LIMIT = 1_400_000;

/** Shared signed governance verifier codes (all governed targets). */
export const GOVERNANCE_ERROR_BASE = 3100;
export const GOVERNANCE_ERROR_NAMES = [
  "InvalidGovernanceTail", "InvalidGovernanceGateData", "MissingGovernanceTail",
  "MissingGovernanceGate", "InvalidGovernanceGatePda", "DuplicateGovernanceGate",
  "InvalidGovernanceGatePrivileges", "InvalidGovernanceGateOwner",
  "GovernanceGateFrozen", "GovernanceGateEpochMismatch",
] as const;

/**
 * Extracts a `Custom(n)` program error code from a web3.js error, a transaction
 * `err` value (`{ InstructionError: [i, { Custom: n }] }`) or a log/message
 * string (`custom program error: 0x46520012`). Returns null when none is found.
 */
export function customErrorCode(error: unknown): number | null {
  if (error == null) return null;
  if (typeof error === "string") {
    const m = /custom program error: (0x[0-9a-fA-F]+|\d+)/u.exec(error);
    return m ? Number(m[1]) : null;
  }
  if (typeof error !== "object") return null;
  const e = error as Record<string, unknown>;
  const ie = e.InstructionError;
  if (Array.isArray(ie) && ie.length === 2) {
    const inner = ie[1] as Record<string, unknown> | string | null;
    if (inner && typeof inner === "object" && typeof inner.Custom === "number") return inner.Custom;
  }
  for (const key of ["err", "error", "cause"]) {
    if (key in e && e[key] !== error) {
      const nested = customErrorCode(e[key]);
      if (nested !== null) return nested;
    }
  }
  if (Array.isArray(e.logs)) {
    for (const line of e.logs) {
      const nested = customErrorCode(line);
      if (nested !== null) return nested;
    }
  }
  if (typeof e.message === "string") return customErrorCode(e.message);
  return null;
}
