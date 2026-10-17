/**
 * Program ids, limits and error helpers shared by the futures client
 * and the oracle `Period` PDA.
 */
import { PublicKey } from "@solana/web3.js";
export declare const SYSTEM_PROGRAM_ID: PublicKey;
export declare const TOKEN_PROGRAM_ID: PublicKey;
export declare const BPF_LOADER_UPGRADEABLE_PROGRAM_ID: PublicKey;
/** Oracle `Period` PDA: `[PERIOD_NAMESPACE, PERIOD_SEED, product_id, expiry u64 LE]`. */
export declare const PERIOD_NAMESPACE = "ameba-oracle-v1";
export declare const PERIOD_SEED = "period";
/** Recommended compute limit for every trading transaction. */
export declare const FUTURES_COMPUTE_UNIT_LIMIT = 1400000;
/** Shared signed governance verifier codes (all governed targets). */
export declare const GOVERNANCE_ERROR_BASE = 3100;
export declare const GOVERNANCE_ERROR_NAMES: readonly ["InvalidGovernanceTail", "InvalidGovernanceGateData", "MissingGovernanceTail", "MissingGovernanceGate", "InvalidGovernanceGatePda", "DuplicateGovernanceGate", "InvalidGovernanceGatePrivileges", "InvalidGovernanceGateOwner", "GovernanceGateFrozen", "GovernanceGateEpochMismatch"];
/**
 * Extracts a `Custom(n)` program error code from a web3.js error, a transaction
 * `err` value (`{ InstructionError: [i, { Custom: n }] }`) or a log/message
 * string (`custom program error: 0x46520012`). Returns null when none is found.
 */
export declare function customErrorCode(error: unknown): number | null;
//# sourceMappingURL=constants.d.ts.map