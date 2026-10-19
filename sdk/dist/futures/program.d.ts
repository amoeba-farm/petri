import { type Address, type IntegerLike } from "./codec.js";
import { type ProgramAddress } from "./pda.js";
export declare const FUTURES_NAMESPACE = "ameba-range-v1";
/** Registry PDA: `[FUTURES_NAMESPACE, "registry", product_id, expiry u64 LE]`, one market per index and expiry. */
export declare const REGISTRY_SEED = "registry";
/** Vault authority PDA: `[FUTURES_NAMESPACE, "authority", market]`, owner of the market's USDC vault. */
export declare const AUTHORITY_SEED = "authority";
/** The Earn vault PDA: `[FUTURES_NAMESPACE, "earn", operator]`, one vault for the whole product. */
export declare const VAULT_SEED = "earn";
/** Earn vault position PDA: `[FUTURES_NAMESPACE, "earn-lp", vault, owner]`. */
export declare const VAULT_LP_SEED = "earn-lp";
/** Legacy per-market Earn pool PDA (dev only): `[FUTURES_NAMESPACE, "pool", market, operator]`. */
export declare const POOL_SEED = "pool";
/** Legacy pool position PDA (dev only): `[FUTURES_NAMESPACE, "lp", pool, owner]`. */
export declare const LP_SEED = "lp";
export declare const FUTURES_MAGIC = "AFRANGE1";
export declare const FUTURES_VERSION = 1;
export declare const HEADER_LEN = 512;
export declare const TRADER_LEN = 64;
export declare const ORDER_LEN = 64;
export declare const CLAIM_LEN = 56;
export declare const MAKER_HEAD = 64;
export declare const STEP_LEN = 32;
export declare const REGISTRY_MAGIC = "AFRNGREG";
export declare const REGISTRY_LEN = 48;
export declare const POOL_MAGIC = "AFRPOOL1";
export declare const POOL_LEN = 128;
export declare const LP_MAGIC = "AFRLPOS1";
export declare const LP_LEN = 128;
export declare const VAULT_MAGIC = "AFRVALT1";
export declare const VAULT_LP_MAGIC = "AFRVLPS1";
/**
 * Markets one Earn vault can be in at once (`state::VAULT_MARKETS`):
 * `VaultDeposit` and `VaultWithdraw` name every one of them, and 13 (with two
 * markets drawn on and the owner's token account created) fill a legacy
 * transaction. Funding a 14th fails with `VaultCapacity`.
 */
export declare const VAULT_MARKETS = 13;
/** Where the vault's market list starts. */
export declare const VAULT_HEAD = 192;
/** The vault account's exact length: `192 + 32 * VAULT_MARKETS` (608). */
export declare const VAULT_LEN: number;
export declare const VAULT_LP_LEN = 128;
/** The reward stream's section, after the vault's markets once its operator has set one (`state::REWARDS_LEN`). */
export declare const REWARDS_LEN = 192;
/** The vault account's length with a reward stream: `VAULT_LEN + REWARDS_LEN` (800). */
export declare const VAULT_REWARDS_LEN: number;
export declare const REWARDS_MAGIC = "AFRVRWD1";
/** Fixed point of the reward accumulator (`state::REWARD_SCALE`). */
export declare const REWARD_SCALE = 1000000000000n;
/** Most depth tiers a ladder step has per side (`ameba_futures_math::MAX_TIERS`). */
export declare const MAX_TIERS = 8;
/** `64 + 32 * grid`: one maker ladder's head and steps. */
export declare function makerLen(grid: number): number;
/** The market account's exact length (`state::market_len`). */
export declare function marketLen(grid: number, traderCap: number, orderCap: number, makerCap: number, claimCap: number): number;
/** Market status (header byte 9). */
export declare const FuturesStatus: {
    readonly Open: 0;
    readonly Settled: 1;
    readonly Failed: 2;
};
export type FuturesStatus = (typeof FuturesStatus)[keyof typeof FuturesStatus];
export declare const STATUS_NAMES: readonly ["open", "settled", "failed"];
export type FuturesStatusName = (typeof STATUS_NAMES)[number];
/** Claim and order side: 0 = long (holds the rising claim, pays the premium), 1 = short. */
export declare const FuturesSide: {
    readonly Long: 0;
    readonly Short: 1;
};
export type FuturesSide = (typeof FuturesSide)[keyof typeof FuturesSide];
export type FuturesSideLike = FuturesSide | "long" | "short";
/** `Submit` modes: immediate-or-cancel, match then rest the remainder, rest only. */
export declare const FuturesMode: {
    readonly Ioc: 0;
    readonly Gtc: 1;
    readonly Post: 2;
};
export type FuturesMode = (typeof FuturesMode)[keyof typeof FuturesMode];
export type FuturesModeLike = FuturesMode | "ioc" | "gtc" | "post";
/** `close_claim` of an ordinary opening order. */
export declare const NO_CLAIM = 4294967295;
/** Order flag: closes (and reserves) one of the owner's claims. */
export declare const ORDER_CLOSE = 1;
/** The heap frame `Submit` and `Quote` request (`heap::HEAP_MAX`, 256 KiB). */
export declare const HEAP_FRAME: number;
export declare function futuresSide(side: FuturesSideLike, name?: string): FuturesSide;
export declare function futuresMode(mode: FuturesModeLike): FuturesMode;
export declare function futuresSideName(side: number): "long" | "short";
/** Futures program errors are `Custom(ERROR_BASE + n)`. */
export declare const ERROR_BASE = 1179779072;
/** `FuturesError` in declaration order; the code is `ERROR_BASE + index + 1`. */
export declare const ERROR_NAMES: readonly ["InvalidInstruction", "InvalidAccount", "Unauthorized", "NotTradable", "InvalidShape", "InvalidPrice", "InsufficientFunds", "TraderCapacity", "OrderCapacity", "ClaimCapacity", "MakerCapacity", "BadProposal", "Unbalanced", "NotBestPrice", "NotMaxFill", "FifoViolation", "SearchLimit", "MinFill", "WouldCross", "NotComplement", "ClaimNotFound", "OrderNotFound", "NotSettled", "AlreadyFinal", "Overflow", "Custody", "Insolvent", "LimitExceeded", "NotExpired", "ClaimReserved", "VaultCapacity", "CreditLimit", "NeedsRecovery"];
export type FuturesErrorName = (typeof ERROR_NAMES)[number];
/** Futures error code by name, e.g. `ERROR_CODES.MinFill === 0x46520012`. */
export declare const ERROR_CODES: Readonly<Record<FuturesErrorName, number>>;
/** Mirrors the client's `error_name`: the futures program's own codes. */
export declare function futuresErrorName(code: number): FuturesErrorName | null;
/**
 * A custom error code of a futures transaction: the futures program's own
 * names, then the shared governance verifier's (every futures instruction
 * carries the governance envelope).
 */
export declare function transactionErrorName(code: number): string | null;
/** The registry that pins one market per product and expiry. */
export declare function findRegistryAddress(program: Address, product: string | Uint8Array, expiryTs: IntegerLike): ProgramAddress;
/** The market's vault authority (owner of its USDC vault). */
export declare function findAuthorityAddress(program: Address, market: Address): ProgramAddress;
/**
 * The Earn vault `operator` runs: one for the whole product. Its idle cash
 * is a token account it owns; in each of its markets it holds a trader slot
 * (owner: this address) that the operator trades and funds from or returns to
 * the idle cash, but can never withdraw.
 */
export declare function findVaultAddress(program: Address, operator: Address): ProgramAddress;
/** One depositor's shares in the Earn vault. */
export declare function findVaultPositionAddress(program: Address, vault: Address, owner: Address): ProgramAddress;
/**
 * A legacy per-market Earn pool (dev only; the Earn vault replaced them):
 * `operator` runs its trader slot in `market`. Its holders leave through
 * `PoolWithdraw` or move into the vault with `PoolMigrate`.
 */
export declare function findPoolAddress(program: Address, market: Address, operator: Address): ProgramAddress;
/** One depositor's shares in a legacy pool. */
export declare function findLpAddress(program: Address, pool: Address, owner: Address): ProgramAddress;
/** Backing per lot of a complete set on `[lo, hi)`: `(hi - lo) * k`. */
export declare function setValue(lo: number, hi: number, k: IntegerLike): bigint;
/** The long claim's payout per lot at settlement `s` (fine units). */
export declare function longPerLot(lo: number, hi: number, s: IntegerLike, k: IntegerLike): bigint;
/** The short (complement) claim's payout per lot. */
export declare function shortPerLot(lo: number, hi: number, s: IntegerLike, k: IntegerLike): bigint;
/** Settlement in fine units, `floor(value * k / max(stepValue, 1))` (saturating at `u64::MAX`, as the program's cast). */
export declare function settleFine(value: IntegerLike, stepValue: IntegerLike, k: IntegerLike): bigint;
/** Cash one lot puts into the set at `price`: a long pays the premium, a short funds the rest of the backing. */
export declare function contribution(side: FuturesSideLike, lo: number, hi: number, price: IntegerLike, k: IntegerLike): bigint;
/** Premium per lot on `[lo, hi)` for an entry in fine units: `entry - lo * k`, clamped to the claim (`premium_at`). */
export declare function premiumAt(lo: number, hi: number, entryFine: IntegerLike, k: IntegerLike): bigint;
/** The entry (fine units) a premium per lot implies: `lo * k + premium` (`entry_of`). */
export declare function entryOf(lo: number, premium: IntegerLike, k: IntegerLike): bigint;
/**
 * A trade's average entry (fine units) as the program records it:
 * `lo * k + max(premium, 0) / lots` for a signed total premium.
 */
export declare function averageEntry(lo: number, premiumTotal: bigint, lots: IntegerLike, k: IntegerLike): bigint;
/** Index value of a grid position: `pos * stepValue` value units, as a decimal string. */
export declare function positionValue(position: number, stepValue: IntegerLike, valueDecimals: number): string;
/** Index value units of a fine-unit entry or settlement: `floor(fine * stepValue / k)`. */
export declare function fineToValueUnits(fine: IntegerLike, stepValue: IntegerLike, k: IntegerLike): bigint;
/** Index value of a fine-unit entry or settlement as a decimal string (floored to `valueDecimals`). */
export declare function fineToValue(fine: IntegerLike, stepValue: IntegerLike, k: IntegerLike, valueDecimals: number): string;
//# sourceMappingURL=program.d.ts.map