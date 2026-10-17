/**
 * The futures program (`ameba-futures`): constants, PDAs, error names and the
 * exact payoff and unit math. Mirrors `futures-client/src/lib.rs`,
 * `programs/futures/src/{lib,state}.rs` and `crates/futures-math/src/payoff.rs`;
 * golden vectors from those crates pin it (`test/fixtures/futures/futures.json`).
 *
 * A market is one account per index and expiry. Prices sit on a grid of
 * positions `0..=grid`. A long claim on `[lo, hi)` pays
 * `clamp(S - lo, 0, hi - lo)` steps of `k` atoms per lot at settlement `S`,
 * the short its complement; a long plus a short on one interval is a complete
 * set, backed by `(hi - lo) * k` atoms per lot. Premiums are atoms per lot
 * (what the long pays); entries and settlements are in fine units of `1/k`
 * step: `entry = lo * k + premium`.
 */
import { PublicKey } from "@solana/web3.js";
import { ascii, toPublicKey, u16, u64, type Address, type IntegerLike } from "./codec.js";
import { GOVERNANCE_ERROR_BASE, GOVERNANCE_ERROR_NAMES } from "./constants.js";
import { formatUnits } from "./math.js";
import { toProductId, type ProgramAddress } from "./pda.js";

// ----------------------------------------------------------------- PDA seeds
// "range" in the seed is the deployed program's on-chain identity; it never changes.
export const FUTURES_NAMESPACE = "ameba-range-v1";
/** Registry PDA: `[FUTURES_NAMESPACE, "registry", product_id, expiry u64 LE]`, one market per index and expiry. */
export const REGISTRY_SEED = "registry";
/** Vault authority PDA: `[FUTURES_NAMESPACE, "authority", market]`, owner of the market's USDC vault. */
export const AUTHORITY_SEED = "authority";
/** The Earn vault PDA: `[FUTURES_NAMESPACE, "earn", operator]`, one vault for the whole product. */
export const VAULT_SEED = "earn";
/** Earn vault position PDA: `[FUTURES_NAMESPACE, "earn-lp", vault, owner]`. */
export const VAULT_LP_SEED = "earn-lp";
/** Legacy per-market Earn pool PDA (dev only): `[FUTURES_NAMESPACE, "pool", market, operator]`. */
export const POOL_SEED = "pool";
/** Legacy pool position PDA (dev only): `[FUTURES_NAMESPACE, "lp", pool, owner]`. */
export const LP_SEED = "lp";

// ----------------------------------------------------------------- Layout
// The magics' "RANGE"/"RNG" is the deployed program's on-chain identity; they never change.
export const FUTURES_MAGIC = "AFRANGE1";
export const FUTURES_VERSION = 1;
export const HEADER_LEN = 512;
export const TRADER_LEN = 64;
export const ORDER_LEN = 64;
export const CLAIM_LEN = 56;
export const MAKER_HEAD = 64;
export const STEP_LEN = 32;
export const REGISTRY_MAGIC = "AFRNGREG";
export const REGISTRY_LEN = 48;
export const POOL_MAGIC = "AFRPOOL1";
export const POOL_LEN = 128;
export const LP_MAGIC = "AFRLPOS1";
export const LP_LEN = 128;
export const VAULT_MAGIC = "AFRVALT1";
export const VAULT_LP_MAGIC = "AFRVLPS1";
/**
 * Markets one Earn vault can be in at once (`state::VAULT_MARKETS`):
 * `VaultDeposit` and `VaultWithdraw` name every one of them, and 13 (with two
 * markets drawn on and the owner's token account created) fill a legacy
 * transaction. Funding a 14th fails with `VaultCapacity`.
 */
export const VAULT_MARKETS = 13;
/** Where the vault's market list starts. */
export const VAULT_HEAD = 192;
/** The vault account's exact length: `192 + 32 * VAULT_MARKETS` (608). */
export const VAULT_LEN = VAULT_HEAD + 32 * VAULT_MARKETS;
export const VAULT_LP_LEN = 128;
/** The reward stream's section, after the vault's markets once its operator has set one (`state::REWARDS_LEN`). */
export const REWARDS_LEN = 192;
/** The vault account's length with a reward stream: `VAULT_LEN + REWARDS_LEN` (800). */
export const VAULT_REWARDS_LEN = VAULT_LEN + REWARDS_LEN;
export const REWARDS_MAGIC = "AFRVRWD1";
/** Fixed point of the reward accumulator (`state::REWARD_SCALE`). */
export const REWARD_SCALE = 1_000_000_000_000n;
/** Most depth tiers a ladder step has per side (`ameba_futures_math::MAX_TIERS`). */
export const MAX_TIERS = 8;

/** `64 + 32 * grid`: one maker ladder's head and steps. */
export function makerLen(grid: number): number {
  return MAKER_HEAD + u16(grid, "grid") * STEP_LEN;
}

/** The market account's exact length (`state::market_len`). */
export function marketLen(grid: number, traderCap: number, orderCap: number, makerCap: number, claimCap: number): number {
  return HEADER_LEN + u16(traderCap, "traderCap") * TRADER_LEN + u16(orderCap, "orderCap") * ORDER_LEN
    + makerCap * makerLen(grid) + claimCap * CLAIM_LEN;
}

// ----------------------------------------------------------------- Enums
/** Market status (header byte 9). */
export const FuturesStatus = { Open: 0, Settled: 1, Failed: 2 } as const;
export type FuturesStatus = (typeof FuturesStatus)[keyof typeof FuturesStatus];
export const STATUS_NAMES = ["open", "settled", "failed"] as const;
export type FuturesStatusName = (typeof STATUS_NAMES)[number];

/** Claim and order side: 0 = long (holds the rising claim, pays the premium), 1 = short. */
export const FuturesSide = { Long: 0, Short: 1 } as const;
export type FuturesSide = (typeof FuturesSide)[keyof typeof FuturesSide];
export type FuturesSideLike = FuturesSide | "long" | "short";

/** `Submit` modes: immediate-or-cancel, match then rest the remainder, rest only. */
export const FuturesMode = { Ioc: 0, Gtc: 1, Post: 2 } as const;
export type FuturesMode = (typeof FuturesMode)[keyof typeof FuturesMode];
export type FuturesModeLike = FuturesMode | "ioc" | "gtc" | "post";

/** `close_claim` of an ordinary opening order. */
export const NO_CLAIM = 0xffff_ffff;
/** Order flag: closes (and reserves) one of the owner's claims. */
export const ORDER_CLOSE = 1;
/** The heap frame `Submit` and `Quote` request (`heap::HEAP_MAX`, 256 KiB). */
export const HEAP_FRAME = 256 * 1024;

export function futuresSide(side: FuturesSideLike, name = "side"): FuturesSide {
  if (side === "long" || side === 0) return FuturesSide.Long;
  if (side === "short" || side === 1) return FuturesSide.Short;
  throw new RangeError(`${name} must be 'long', 'short', 0 or 1`);
}

export function futuresMode(mode: FuturesModeLike): FuturesMode {
  if (mode === "ioc" || mode === 0) return FuturesMode.Ioc;
  if (mode === "gtc" || mode === 1) return FuturesMode.Gtc;
  if (mode === "post" || mode === 2) return FuturesMode.Post;
  throw new RangeError("mode must be 'ioc', 'gtc', 'post', 0, 1 or 2");
}

export function futuresSideName(side: number): "long" | "short" {
  return futuresSide(side as FuturesSide) === FuturesSide.Long ? "long" : "short";
}

// ----------------------------------------------------------------- Errors
/** Futures program errors are `Custom(ERROR_BASE + n)`. */
export const ERROR_BASE = 0x4652_0000;

/** `FuturesError` in declaration order; the code is `ERROR_BASE + index + 1`. */
export const ERROR_NAMES = [
  "InvalidInstruction",
  "InvalidAccount",
  "Unauthorized",
  "NotTradable",
  "InvalidShape",
  "InvalidPrice",
  "InsufficientFunds",
  "TraderCapacity",
  "OrderCapacity",
  "ClaimCapacity",
  "MakerCapacity",
  "BadProposal",
  "Unbalanced",
  "NotBestPrice",
  "NotMaxFill",
  "FifoViolation",
  "SearchLimit",
  "MinFill",
  "WouldCross",
  "NotComplement",
  "ClaimNotFound",
  "OrderNotFound",
  "NotSettled",
  "AlreadyFinal",
  "Overflow",
  "Custody",
  "Insolvent",
  "LimitExceeded",
  "NotExpired",
  "ClaimReserved",
  "VaultCapacity",
] as const;
export type FuturesErrorName = (typeof ERROR_NAMES)[number];

/** Futures error code by name, e.g. `ERROR_CODES.MinFill === 0x46520012`. */
export const ERROR_CODES = Object.freeze(Object.fromEntries(
  ERROR_NAMES.map((name, i) => [name, ERROR_BASE + i + 1]),
)) as Readonly<Record<FuturesErrorName, number>>;

/** Mirrors the client's `error_name`: the futures program's own codes. */
export function futuresErrorName(code: number): FuturesErrorName | null {
  const i = code - ERROR_BASE - 1;
  return Number.isInteger(i) && i >= 0 && i < ERROR_NAMES.length ? ERROR_NAMES[i]! : null;
}

/**
 * A custom error code of a futures transaction: the futures program's own
 * names, then the shared governance verifier's (every futures instruction
 * carries the governance envelope).
 */
export function transactionErrorName(code: number): string | null {
  const own = futuresErrorName(code);
  if (own) return own;
  const g = code - GOVERNANCE_ERROR_BASE;
  return g >= 0 && g < GOVERNANCE_ERROR_NAMES.length ? GOVERNANCE_ERROR_NAMES[g]! : null;
}

// ----------------------------------------------------------------- PDAs
function le64(value: IntegerLike): Uint8Array {
  const out = new Uint8Array(8);
  new DataView(out.buffer).setBigUint64(0, u64(value, "expiry_ts"), true);
  return out;
}

/** The registry that pins one market per product and expiry. */
export function findRegistryAddress(program: Address, product: string | Uint8Array, expiryTs: IntegerLike): ProgramAddress {
  return PublicKey.findProgramAddressSync(
    [ascii(FUTURES_NAMESPACE), ascii(REGISTRY_SEED), toProductId(product), le64(expiryTs)],
    toPublicKey(program),
  );
}

/** The market's vault authority (owner of its USDC vault). */
export function findAuthorityAddress(program: Address, market: Address): ProgramAddress {
  return PublicKey.findProgramAddressSync(
    [ascii(FUTURES_NAMESPACE), ascii(AUTHORITY_SEED), toPublicKey(market).toBytes()],
    toPublicKey(program),
  );
}

/**
 * The Earn vault `operator` runs: one for the whole product. Its idle cash
 * is a token account it owns; in each of its markets it holds a trader slot
 * (owner: this address) that the operator trades and funds from or returns to
 * the idle cash, but can never withdraw.
 */
export function findVaultAddress(program: Address, operator: Address): ProgramAddress {
  return PublicKey.findProgramAddressSync(
    [ascii(FUTURES_NAMESPACE), ascii(VAULT_SEED), toPublicKey(operator).toBytes()],
    toPublicKey(program),
  );
}

/** One depositor's shares in the Earn vault. */
export function findVaultPositionAddress(program: Address, vault: Address, owner: Address): ProgramAddress {
  return PublicKey.findProgramAddressSync(
    [ascii(FUTURES_NAMESPACE), ascii(VAULT_LP_SEED), toPublicKey(vault).toBytes(), toPublicKey(owner).toBytes()],
    toPublicKey(program),
  );
}

/**
 * A legacy per-market Earn pool (dev only; the Earn vault replaced them):
 * `operator` runs its trader slot in `market`. Its holders leave through
 * `PoolWithdraw` or move into the vault with `PoolMigrate`.
 */
export function findPoolAddress(program: Address, market: Address, operator: Address): ProgramAddress {
  return PublicKey.findProgramAddressSync(
    [ascii(FUTURES_NAMESPACE), ascii(POOL_SEED), toPublicKey(market).toBytes(), toPublicKey(operator).toBytes()],
    toPublicKey(program),
  );
}

/** One depositor's shares in a legacy pool. */
export function findLpAddress(program: Address, pool: Address, owner: Address): ProgramAddress {
  return PublicKey.findProgramAddressSync(
    [ascii(FUTURES_NAMESPACE), ascii(LP_SEED), toPublicKey(pool).toBytes(), toPublicKey(owner).toBytes()],
    toPublicKey(program),
  );
}

// ----------------------------------------------------------------- Payoffs (`futures-math` payoff.rs)
function pos(value: number, name: string): bigint {
  return BigInt(u16(value, name));
}

function shape(lo: number, hi: number): [bigint, bigint] {
  const l = pos(lo, "lo"), h = pos(hi, "hi");
  if (h < l) throw new RangeError("hi must not be below lo");
  return [l, h];
}

/** Backing per lot of a complete set on `[lo, hi)`: `(hi - lo) * k`. */
export function setValue(lo: number, hi: number, k: IntegerLike): bigint {
  const [l, h] = shape(lo, hi);
  return (h - l) * u64(k, "k");
}

/** The long claim's payout per lot at settlement `s` (fine units). */
export function longPerLot(lo: number, hi: number, s: IntegerLike, k: IntegerLike): bigint {
  const [l] = shape(lo, hi);
  const kk = u64(k, "k");
  const above = u64(s, "s") - l * kk;
  const set = setValue(lo, hi, kk);
  return above <= 0n ? 0n : above < set ? above : set;
}

/** The short (complement) claim's payout per lot. */
export function shortPerLot(lo: number, hi: number, s: IntegerLike, k: IntegerLike): bigint {
  return setValue(lo, hi, k) - longPerLot(lo, hi, s, k);
}

/** Settlement in fine units, `floor(value * k / max(stepValue, 1))` (saturating at `u64::MAX`, as the program's cast). */
export function settleFine(value: IntegerLike, stepValue: IntegerLike, k: IntegerLike): bigint {
  const step = u64(stepValue, "stepValue");
  return BigInt.asUintN(64, (u64(value, "value") * u64(k, "k")) / (step > 0n ? step : 1n));
}

/** Cash one lot puts into the set at `price`: a long pays the premium, a short funds the rest of the backing. */
export function contribution(side: FuturesSideLike, lo: number, hi: number, price: IntegerLike, k: IntegerLike): bigint {
  const set = setValue(lo, hi, k);
  const p = u64(price, "price");
  return futuresSide(side) === FuturesSide.Long ? p : set > p ? set - p : 0n;
}

/** Premium per lot on `[lo, hi)` for an entry in fine units: `entry - lo * k`, clamped to the claim (`premium_at`). */
export function premiumAt(lo: number, hi: number, entryFine: IntegerLike, k: IntegerLike): bigint {
  return longPerLot(lo, hi, entryFine, k);
}

/** The entry (fine units) a premium per lot implies: `lo * k + premium` (`entry_of`). */
export function entryOf(lo: number, premium: IntegerLike, k: IntegerLike): bigint {
  return pos(lo, "lo") * u64(k, "k") + u64(premium, "premium");
}

/**
 * A trade's average entry (fine units) as the program records it:
 * `lo * k + max(premium, 0) / lots` for a signed total premium.
 */
export function averageEntry(lo: number, premiumTotal: bigint, lots: IntegerLike, k: IntegerLike): bigint {
  const n = u64(lots, "lots");
  if (n === 0n) throw new RangeError("lots must be positive");
  return pos(lo, "lo") * u64(k, "k") + (premiumTotal > 0n ? premiumTotal : 0n) / n;
}

/** Index value of a grid position: `pos * stepValue` value units, as a decimal string. */
export function positionValue(position: number, stepValue: IntegerLike, valueDecimals: number): string {
  return formatUnits(pos(position, "position") * u64(stepValue, "stepValue"), valueDecimals);
}

/** Index value units of a fine-unit entry or settlement: `floor(fine * stepValue / k)`. */
export function fineToValueUnits(fine: IntegerLike, stepValue: IntegerLike, k: IntegerLike): bigint {
  const kk = u64(k, "k");
  if (kk === 0n) throw new RangeError("k must be positive");
  return (u64(fine, "fine") * u64(stepValue, "stepValue")) / kk;
}

/** Index value of a fine-unit entry or settlement as a decimal string (floored to `valueDecimals`). */
export function fineToValue(fine: IntegerLike, stepValue: IntegerLike, k: IntegerLike, valueDecimals: number): string {
  return formatUnits(fineToValueUnits(fine, stepValue, k), valueDecimals);
}
