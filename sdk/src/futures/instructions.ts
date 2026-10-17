/**
 * Futures instruction encoding (Borsh `FuturesInstruction`,
 * `programs/futures/src/lib.rs`) and `FuturesMarket`, the builders with the
 * exact account lists of `futures-client`'s `Market`.
 *
 * Every futures instruction carries the governance envelope on a governed
 * deployment (dev and mainnet): bind it with
 * `GovernanceGateBinding.fromAccount(controller, program, gate, owner, data).bind(ix)`
 * before signing. The futures API's sponsored transactions are already bound.
 *
 * `Submit` and `Quote` carry a solved proposal (`proposal.ts`); get one from
 * the futures API (`FuturesClient.prepare`) or the Rust client
 * (`Book::propose`), and send `Submit` with `heapFrame()` and a 1.4M compute
 * limit.
 *
 * `FuturesEarnVault` builds the Earn vault's instructions (the client's
 * `EarnVault`): one vault for the whole product, which replaced the
 * per-market Earn pools. `CreatePool` (tag 15) and `PoolDeposit` (16) are
 * retired: the program refuses them (`InvalidInstruction`), so there are no
 * builders for them, but the codec still reads and writes them (decoded with
 * `retired: true`) so an old transaction reviews instead of throwing.
 */
import { Buffer } from "buffer";
import { ComputeBudgetProgram, PublicKey, TransactionInstruction, type AccountMeta } from "@solana/web3.js";
import { ByteReader, ByteWriter, U64_MAX, toPublicKey, type Address, type IntegerLike } from "./codec.js";
import { BPF_LOADER_UPGRADEABLE_PROGRAM_ID, FUTURES_COMPUTE_UNIT_LIMIT, SYSTEM_PROGRAM_ID, TOKEN_PROGRAM_ID } from "./constants.js";
import { decodeGovernanceTail } from "./governance.js";
import { productSymbol, toProductId } from "./pda.js";
import {
  HEAP_FRAME, NO_CLAIM, FuturesMode, FuturesSide, findAuthorityAddress, findLpAddress, findPoolAddress, findRegistryAddress,
  findVaultAddress, findVaultPositionAddress, futuresMode, futuresSide, marketLen, setValue, type FuturesModeLike,
  type FuturesSideLike,
} from "./program.js";
import type { FuturesClaim, FuturesHeader, FuturesVault } from "./accounts.js";

/** Enum tags of `FuturesInstruction`. */
export const FuturesInstructionTag = {
  CreateMarket: 0,
  Deposit: 1,
  Withdraw: 2,
  Submit: 3,
  Cancel: 4,
  Quote: 5,
  CancelQuote: 6,
  Split: 7,
  Merge: 8,
  Redeem: 9,
  Transfer: 10,
  Finalize: 11,
  Settle: 12,
  SettleMaker: 13,
  Prune: 14,
  /** Retired: always `InvalidInstruction` (kept so later tags hold). */
  CreatePool: 15,
  /** Retired: always `InvalidInstruction`. */
  PoolDeposit: 16,
  /** Legacy pools' exit (dev builds only). */
  PoolWithdraw: 17,
  CouncilSettle: 18,
  CreateVault: 19,
  VaultDeposit: 20,
  VaultWithdraw: 21,
  VaultFund: 22,
  VaultDefund: 23,
  /** Legacy pools into the vault (dev builds only). */
  PoolMigrate: 24,
  /** The operator sets the Earn vault's reward stream (AMBA for Earn). */
  VaultRewards: 25,
  /** Pays a position's rewards into its owner's account (anyone may send it). */
  VaultRewardsClaim: 26,
  /** The operator takes back the reward account's surplus. */
  VaultRewardsWithdraw: 27,
} as const;
export type FuturesInstructionTag = (typeof FuturesInstructionTag)[keyof typeof FuturesInstructionTag];

/** A claim or order is named by its slot and its current sequence. */
export interface FuturesRef {
  readonly slot: number;
  readonly seq: IntegerLike;
}

export interface CreateFuturesMarketParams {
  /** Symbol (`"RAMX"`) or the 32-byte product id. */
  readonly productId: string | Uint8Array;
  readonly expiryTs: IntegerLike;
  /** Highest grid position; positions are `0..=grid`. */
  readonly grid: number;
  /** Atoms one lot of a one-step claim pays in full. */
  readonly k: IntegerLike;
  /** Oracle value units per grid step (at `valueDecimals`). */
  readonly stepValue: IntegerLike;
  readonly valueDecimals: number;
  /** Reference index value: the dev settlement if the period fails. */
  readonly refValue: IntegerLike;
  readonly traderCap: number;
  readonly orderCap: number;
  readonly makerCap: number;
  readonly claimCap: number;
}

/** The market account's exact size for these parameters (`CreateArgs::space`). */
export function marketSpace(p: Pick<CreateFuturesMarketParams, "grid" | "traderCap" | "orderCap" | "makerCap" | "claimCap">): number {
  return marketLen(p.grid, p.traderCap, p.orderCap, p.makerCap, p.claimCap);
}

/** An order to submit: one claim shape at a limit premium per lot. */
export interface FuturesSubmitArgs {
  readonly side: FuturesSideLike;
  readonly lo: number;
  readonly hi: number;
  readonly lots: IntegerLike;
  /** Long: most premium per lot. Short: least premium per lot (atoms). */
  readonly limit: IntegerLike;
  readonly minFill: IntegerLike;
  readonly mode: FuturesModeLike;
  /** The owner's claim this order closes (its exact complement), or none. */
  readonly close?: FuturesRef | null;
}

/** One knot of a maker ladder's mid curve: the premium per lot of the one-step claim `[pos, pos + 1)`. */
export interface FuturesKnot {
  readonly pos: number;
  readonly price: number;
}

/** A maker ladder quote over `[lo, hi)` (`state::Ladder`). */
export interface FuturesLadderArgs {
  readonly lo: number;
  readonly hi: number;
  readonly bidSpread: number;
  readonly askSpread: number;
  readonly bidLots: IntegerLike;
  readonly askLots: IntegerLike;
  /** Keep consumption: each step's lots become at most what is left. */
  readonly keep: boolean;
  /** Depth tiers (count 0 or 1: one price per step and side). */
  readonly tiers: { readonly count: number; readonly ratio: number; readonly spread: number };
}

export type FuturesInstruction =
  | ({ readonly kind: "createMarket" } & CreateFuturesMarketParams)
  | { readonly kind: "deposit"; readonly amount: IntegerLike }
  | { readonly kind: "withdraw"; readonly amount: IntegerLike }
  | ({ readonly kind: "submit"; readonly proposal: Uint8Array } & FuturesSubmitArgs)
  | { readonly kind: "cancel"; readonly slot: number; readonly seq: IntegerLike }
  | ({
    readonly kind: "quote"; readonly knots: readonly FuturesKnot[]; readonly coreLo: number; readonly coreHi: number;
    readonly proposal: Uint8Array;
  } & FuturesLadderArgs)
  | { readonly kind: "cancelQuote" }
  | { readonly kind: "split"; readonly claim: number; readonly seq: IntegerLike; readonly at: number }
  | { readonly kind: "merge"; readonly keep: number; readonly keepSeq: IntegerLike; readonly absorb: number; readonly absorbSeq: IntegerLike }
  | {
    readonly kind: "redeem"; readonly long: number; readonly longSeq: IntegerLike; readonly short: number;
    readonly shortSeq: IntegerLike; readonly lots: IntegerLike;
  }
  | { readonly kind: "transfer"; readonly claim: number; readonly seq: IntegerLike; readonly lots: IntegerLike; readonly to: Address }
  | { readonly kind: "finalize" }
  | { readonly kind: "settle"; readonly claims: readonly number[] }
  | { readonly kind: "settleMaker"; readonly maker: number }
  | { readonly kind: "prune"; readonly orders: readonly number[] }
  /** Retired (the program refuses it); encodable only so decoded data round-trips. */
  | { readonly kind: "createPool"; readonly retired?: true }
  /** Retired (the program refuses it); encodable only so decoded data round-trips. */
  | { readonly kind: "poolDeposit"; readonly amount: IntegerLike; readonly retired?: true }
  | { readonly kind: "poolWithdraw"; readonly shares: IntegerLike }
  | { readonly kind: "councilSettle"; readonly value: IntegerLike }
  | { readonly kind: "createVault" }
  | { readonly kind: "vaultDeposit"; readonly amount: IntegerLike }
  | { readonly kind: "vaultWithdraw"; readonly shares: IntegerLike }
  | { readonly kind: "vaultFund"; readonly amount: IntegerLike }
  | { readonly kind: "vaultDefund"; readonly amount: IntegerLike }
  | { readonly kind: "poolMigrate" }
  | {
    readonly kind: "vaultRewards"; readonly startTs: IntegerLike; readonly endTs: IntegerLike; readonly amount: IntegerLike;
    readonly house: Address;
  }
  | { readonly kind: "vaultRewardsClaim" }
  | { readonly kind: "vaultRewardsWithdraw"; readonly amount: IntegerLike };

function bytesVec(w: ByteWriter, bytes: Uint8Array, name: string): void {
  w.u32(bytes.length, `${name}.length`).bytes(bytes);
}

/** Borsh bytes of one futures instruction (without the governance tail). */
export function encodeFuturesInstruction(ix: FuturesInstruction): Buffer {
  const w = new ByteWriter();
  switch (ix.kind) {
    case "createMarket":
      return w.u8(0).bytes(toProductId(ix.productId), 32).u64(ix.expiryTs, "expiryTs").u16(ix.grid, "grid").u64(ix.k, "k")
        .u64(ix.stepValue, "stepValue").u8(ix.valueDecimals, "valueDecimals").u64(ix.refValue, "refValue")
        .u16(ix.traderCap, "traderCap").u16(ix.orderCap, "orderCap").u8(ix.makerCap, "makerCap").u32(ix.claimCap, "claimCap")
        .toBuffer();
    case "deposit":
      return w.u8(1).u64(ix.amount, "amount").toBuffer();
    case "withdraw":
      return w.u8(2).u64(ix.amount, "amount").toBuffer();
    case "submit": {
      w.u8(3).u8(futuresSide(ix.side)).u16(ix.lo, "lo").u16(ix.hi, "hi").u64(ix.lots, "lots").u64(ix.limit, "limit")
        .u64(ix.minFill, "minFill").u8(futuresMode(ix.mode)).u32(ix.close ? ix.close.slot : NO_CLAIM, "close.slot")
        .u64(ix.close ? ix.close.seq : 0, "close.seq");
      bytesVec(w, ix.proposal, "proposal");
      return w.toBuffer();
    }
    case "cancel":
      return w.u8(4).u16(ix.slot, "slot").u64(ix.seq, "seq").toBuffer();
    case "quote": {
      w.u8(5).u16(ix.lo, "lo").u16(ix.hi, "hi").u32(ix.knots.length, "knots.length");
      for (const k of ix.knots) w.u16(k.pos, "knot.pos").u32(k.price, "knot.price");
      w.u32(ix.bidSpread, "bidSpread").u32(ix.askSpread, "askSpread").u64(ix.bidLots, "bidLots").u64(ix.askLots, "askLots")
        .u8(ix.keep ? 1 : 0).u8(ix.tiers.count, "tiers.count").u16(ix.tiers.ratio, "tiers.ratio")
        .u32(ix.tiers.spread, "tiers.spread").u16(ix.coreLo, "coreLo").u16(ix.coreHi, "coreHi");
      bytesVec(w, ix.proposal, "proposal");
      return w.toBuffer();
    }
    case "cancelQuote":
      return w.u8(6).toBuffer();
    case "split":
      return w.u8(7).u32(ix.claim, "claim").u64(ix.seq, "seq").u16(ix.at, "at").toBuffer();
    case "merge":
      return w.u8(8).u32(ix.keep, "keep").u64(ix.keepSeq, "keepSeq").u32(ix.absorb, "absorb").u64(ix.absorbSeq, "absorbSeq")
        .toBuffer();
    case "redeem":
      return w.u8(9).u32(ix.long, "long").u64(ix.longSeq, "longSeq").u32(ix.short, "short").u64(ix.shortSeq, "shortSeq")
        .u64(ix.lots, "lots").toBuffer();
    case "transfer":
      return w.u8(10).u32(ix.claim, "claim").u64(ix.seq, "seq").u64(ix.lots, "lots").pubkey(ix.to).toBuffer();
    case "finalize":
      return w.u8(11).toBuffer();
    case "settle":
      w.u8(12).u32(ix.claims.length, "claims.length");
      for (const c of ix.claims) w.u32(c, "claim");
      return w.toBuffer();
    case "settleMaker":
      return w.u8(13).u8(ix.maker, "maker").toBuffer();
    case "prune":
      w.u8(14).u32(ix.orders.length, "orders.length");
      for (const o of ix.orders) w.u16(o, "order");
      return w.toBuffer();
    case "createPool":
      return w.u8(15).toBuffer();
    case "poolDeposit":
      return w.u8(16).u64(ix.amount, "amount").toBuffer();
    case "poolWithdraw":
      return w.u8(17).u64(ix.shares, "shares").toBuffer();
    case "councilSettle":
      return w.u8(18).u64(ix.value, "value").toBuffer();
    case "createVault":
      return w.u8(19).toBuffer();
    case "vaultDeposit":
      return w.u8(20).u64(ix.amount, "amount").toBuffer();
    case "vaultWithdraw":
      return w.u8(21).u64(ix.shares, "shares").toBuffer();
    case "vaultFund":
      return w.u8(22).u64(ix.amount, "amount").toBuffer();
    case "vaultDefund":
      return w.u8(23).u64(ix.amount, "amount").toBuffer();
    case "poolMigrate":
      return w.u8(24).toBuffer();
    case "vaultRewards":
      return w.u8(25).u64(ix.startTs, "startTs").u64(ix.endTs, "endTs").u64(ix.amount, "amount")
        .pubkey(ix.house).toBuffer();
    case "vaultRewardsClaim":
      return w.u8(26).toBuffer();
    case "vaultRewardsWithdraw":
      return w.u8(27).u64(ix.amount, "amount").toBuffer();
    default: {
      const never: never = ix;
      throw new Error(`unknown futures instruction ${(never as { kind: string }).kind}`);
    }
  }
}

/** Decoded instruction with integers normalized (u64 as bigint, keys as `PublicKey`). */
export type DecodedFuturesInstruction =
  | {
    kind: "createMarket"; productId: Uint8Array; symbol: string; expiryTs: bigint; grid: number; k: bigint; stepValue: bigint;
    valueDecimals: number; refValue: bigint; traderCap: number; orderCap: number; makerCap: number; claimCap: number;
  }
  | { kind: "deposit"; amount: bigint }
  | { kind: "withdraw"; amount: bigint }
  | {
    /** 0 long, 1 short (as on the wire; the program rejects anything else). */
    kind: "submit"; side: number; lo: number; hi: number; lots: bigint; limit: bigint; minFill: bigint;
    /** 0 ioc, 1 gtc, 2 post (as on the wire). */
    mode: number; close: { slot: number; seq: bigint } | null; proposal: Uint8Array;
  }
  | { kind: "cancel"; slot: number; seq: bigint }
  | {
    kind: "quote"; lo: number; hi: number; knots: FuturesKnot[]; bidSpread: number; askSpread: number; bidLots: bigint;
    askLots: bigint; keep: boolean; tiers: { count: number; ratio: number; spread: number }; coreLo: number; coreHi: number;
    proposal: Uint8Array;
  }
  | { kind: "cancelQuote" }
  | { kind: "split"; claim: number; seq: bigint; at: number }
  | { kind: "merge"; keep: number; keepSeq: bigint; absorb: number; absorbSeq: bigint }
  | { kind: "redeem"; long: number; longSeq: bigint; short: number; shortSeq: bigint; lots: bigint }
  | { kind: "transfer"; claim: number; seq: bigint; lots: bigint; to: PublicKey }
  | { kind: "finalize" }
  | { kind: "settle"; claims: number[] }
  | { kind: "settleMaker"; maker: number }
  | { kind: "prune"; orders: number[] }
  /** Tag 15, retired: the program refuses it. */
  | { kind: "createPool"; retired: true }
  /** Tag 16, retired: the program refuses it. */
  | { kind: "poolDeposit"; amount: bigint; retired: true }
  | { kind: "poolWithdraw"; shares: bigint }
  | { kind: "councilSettle"; value: bigint }
  | { kind: "createVault" }
  | { kind: "vaultDeposit"; amount: bigint }
  | { kind: "vaultWithdraw"; shares: bigint }
  | { kind: "vaultFund"; amount: bigint }
  | { kind: "vaultDefund"; amount: bigint }
  | { kind: "poolMigrate" }
  | { kind: "vaultRewards"; startTs: bigint; endTs: bigint; amount: bigint; house: PublicKey }
  | { kind: "vaultRewardsClaim" }
  | { kind: "vaultRewardsWithdraw"; amount: bigint };

export interface DecodeFuturesInstructionOptions {
  /**
   * The data ends with the 16-byte governance tail (a bound instruction, as
   * in every transaction the futures API prepares): strip and check it.
   */
  readonly governed?: boolean;
}

/**
 * Decodes futures instruction data; throws on an unknown tag, short data or
 * trailing bytes (Borsh rejects both). Use it to review a prepared
 * transaction before signing it. The retired tags 15 and 16 decode with
 * `retired: true` (the program refuses them).
 */
export function decodeFuturesInstruction(data: Uint8Array, options: DecodeFuturesInstructionOptions = {}): DecodedFuturesInstruction {
  const body = options.governed ? decodeGovernanceTail(data).payload : data;
  const r = new ByteReader(body);
  let at = 1;
  const take = <T>(n: number, f: (o: number) => T): T => {
    const v = f(at);
    at += n;
    return v;
  };
  const b = () => take(1, (o) => r.u8(o));
  const h = () => take(2, (o) => r.u16(o));
  const w = () => take(4, (o) => r.u32(o));
  const q = () => take(8, (o) => r.u64(o));
  const key = () => take(32, (o) => r.pubkey(o));
  const bool = () => {
    const v = b();
    if (v > 1) throw new RangeError("invalid bool");
    return v === 1;
  };
  const vec = <T>(f: () => T): T[] => {
    const n = w();
    if (n > body.length) throw new RangeError("vector length exceeds data");
    return Array.from({ length: n }, f);
  };
  const bytes = () => {
    const n = w();
    return take(n, (o) => r.slice(o, n));
  };
  const tag = r.u8(0);
  let out: DecodedFuturesInstruction;
  switch (tag) {
    case 0: {
      const productId = take(32, (o) => r.slice(o, 32));
      out = {
        kind: "createMarket", productId, symbol: productSymbol(productId), expiryTs: q(), grid: h(), k: q(), stepValue: q(),
        valueDecimals: b(), refValue: q(), traderCap: h(), orderCap: h(), makerCap: b(), claimCap: w(),
      };
      break;
    }
    case 1: out = { kind: "deposit", amount: q() }; break;
    case 2: out = { kind: "withdraw", amount: q() }; break;
    case 3: {
      const s = b(), lo = h(), hi = h(), lots = q(), limit = q(), minFill = q(), mode = b();
      const closeSlot = w(), closeSeq = q();
      out = {
        kind: "submit", side: s, lo, hi, lots, limit, minFill, mode,
        close: closeSlot === NO_CLAIM ? null : { slot: closeSlot, seq: closeSeq }, proposal: bytes(),
      };
      break;
    }
    case 4: out = { kind: "cancel", slot: h(), seq: q() }; break;
    case 5: {
      const lo = h(), hi = h(), knots = vec(() => ({ pos: h(), price: w() }));
      out = {
        kind: "quote", lo, hi, knots, bidSpread: w(), askSpread: w(), bidLots: q(), askLots: q(), keep: bool(),
        tiers: { count: b(), ratio: h(), spread: w() }, coreLo: h(), coreHi: h(), proposal: bytes(),
      };
      break;
    }
    case 6: out = { kind: "cancelQuote" }; break;
    case 7: out = { kind: "split", claim: w(), seq: q(), at: h() }; break;
    case 8: out = { kind: "merge", keep: w(), keepSeq: q(), absorb: w(), absorbSeq: q() }; break;
    case 9: out = { kind: "redeem", long: w(), longSeq: q(), short: w(), shortSeq: q(), lots: q() }; break;
    case 10: out = { kind: "transfer", claim: w(), seq: q(), lots: q(), to: key() }; break;
    case 11: out = { kind: "finalize" }; break;
    case 12: out = { kind: "settle", claims: vec(w) }; break;
    case 13: out = { kind: "settleMaker", maker: b() }; break;
    case 14: out = { kind: "prune", orders: vec(h) }; break;
    case 15: out = { kind: "createPool", retired: true }; break;
    case 16: out = { kind: "poolDeposit", amount: q(), retired: true }; break;
    case 17: out = { kind: "poolWithdraw", shares: q() }; break;
    case 18: out = { kind: "councilSettle", value: q() }; break;
    case 19: out = { kind: "createVault" }; break;
    case 20: out = { kind: "vaultDeposit", amount: q() }; break;
    case 21: out = { kind: "vaultWithdraw", shares: q() }; break;
    case 22: out = { kind: "vaultFund", amount: q() }; break;
    case 23: out = { kind: "vaultDefund", amount: q() }; break;
    case 24: out = { kind: "poolMigrate" }; break;
    case 25: out = { kind: "vaultRewards", startTs: q(), endTs: q(), amount: q(), house: key() }; break;
    case 26: out = { kind: "vaultRewardsClaim" }; break;
    case 27: out = { kind: "vaultRewardsWithdraw", amount: q() }; break;
    default: throw new RangeError(`unknown futures instruction tag ${tag}`);
  }
  if (at !== body.length) throw new RangeError("futures instruction has trailing bytes");
  return out;
}

const ro = (pubkey: PublicKey, isSigner = false): AccountMeta => ({ pubkey, isSigner, isWritable: false });
const rw = (pubkey: PublicKey, isSigner = false): AccountMeta => ({ pubkey, isSigner, isWritable: true });

/** `ComputeBudgetInstruction::RequestHeapFrame`; `Submit` and `Quote` need `HEAP_FRAME` (the client's `heap_frame`). */
export function heapFrame(bytes: number = HEAP_FRAME): TransactionInstruction {
  return ComputeBudgetProgram.requestHeapFrame({ bytes });
}

/** `ComputeBudgetInstruction::SetComputeUnitLimit` (the client's `compute_limit`); the API uses 1.4M for every transaction. */
export function computeLimit(units: number = FUTURES_COMPUTE_UNIT_LIMIT): TransactionInstruction {
  return ComputeBudgetProgram.setComputeUnitLimit({ units });
}

interface MarketFields {
  readonly program: PublicKey;
  readonly market: PublicKey;
  readonly registry: PublicKey;
  readonly authority: PublicKey;
  readonly vault: PublicKey;
  readonly period: PublicKey;
  readonly mint: PublicKey;
}

/**
 * Closing a claim: the opposite side on its interval, naming it, `ioc`
 * (what the futures API's `close` sends). Defaults as the API's: all
 * unreserved lots, the most permissive limit (0 selling a long, the set
 * buying back a short) and `minFill` 1.
 */
export function closeArgs(claim: Pick<FuturesClaim, "slot" | "seq" | "side" | "lo" | "hi" | "lots" | "reserved">,
  k: IntegerLike, options: { readonly lots?: IntegerLike; readonly limit?: IntegerLike; readonly minFill?: IntegerLike } = {},
): FuturesSubmitArgs {
  const long = futuresSide(claim.side as FuturesSide) === FuturesSide.Long;
  const free = claim.lots > claim.reserved ? claim.lots - claim.reserved : 0n;
  return {
    side: long ? FuturesSide.Short : FuturesSide.Long,
    lo: claim.lo,
    hi: claim.hi,
    lots: options.lots ?? free,
    limit: options.limit ?? (long ? 0n : setValue(claim.lo, claim.hi, k)),
    minFill: options.minFill ?? 1n,
    mode: FuturesMode.Ioc,
    close: { slot: claim.slot, seq: claim.seq },
  };
}

/** One market's addresses and its instructions (mirrors the Rust client's `Market`). */
export class FuturesMarket implements MarketFields {
  readonly program: PublicKey;
  readonly market: PublicKey;
  /** `["ameba-range-v1", "registry", product_id, expiry]` (the deployed program's seeds). */
  readonly registry: PublicKey;
  /** Vault authority `["ameba-range-v1", "authority", market]`. */
  readonly authority: PublicKey;
  readonly vault: PublicKey;
  /** The oracle `Period` the market settles against. */
  readonly period: PublicKey;
  /** The quote (USDC) mint. */
  readonly mint: PublicKey;

  private constructor(f: MarketFields) {
    this.program = f.program;
    this.market = f.market;
    this.registry = f.registry;
    this.authority = f.authority;
    this.vault = f.vault;
    this.period = f.period;
    this.mint = f.mint;
  }

  /** Addresses of `market` (its vault is the authority's token account; pass it in). */
  static derive(program: Address, market: Address, accounts: {
    readonly vault: Address; readonly period: Address; readonly mint: Address;
  }, product: string | Uint8Array, expiryTs: IntegerLike): FuturesMarket {
    const p = toPublicKey(program), m = toPublicKey(market);
    return new FuturesMarket({
      program: p,
      market: m,
      registry: findRegistryAddress(p, product, expiryTs)[0],
      authority: findAuthorityAddress(p, m)[0],
      vault: toPublicKey(accounts.vault),
      period: toPublicKey(accounts.period),
      mint: toPublicKey(accounts.mint),
    });
  }

  /** From a decoded market account (`decodeHeader` / `FuturesBook.header`) and its address. */
  static fromHeader(program: Address, market: Address,
    header: Pick<FuturesHeader, "productId" | "expiryTs" | "vault" | "period" | "quoteMint">): FuturesMarket {
    return FuturesMarket.derive(program, market, { vault: header.vault, period: header.period, mint: header.quoteMint },
      header.productId, header.expiryTs);
  }

  /** The vault authority PDA of a market. */
  static authorityOf(program: Address, market: Address): PublicKey {
    return findAuthorityAddress(program, market)[0];
  }

  private ix(data: FuturesInstruction, keys: AccountMeta[]): TransactionInstruction {
    return new TransactionInstruction({ programId: this.program, keys, data: encodeFuturesInstruction(data) });
  }

  private trading(owner: Address): AccountMeta[] {
    return [rw(this.market), ro(toPublicKey(owner), true), ro(this.period)];
  }

  private owned(owner: Address): AccountMeta[] {
    return [rw(this.market), ro(toPublicKey(owner), true)];
  }

  /**
   * `CreateMarket`, once the market account exists (zeroed, owned by the
   * program, `marketSpace(params)` bytes) and the vault is the
   * authority's. `creator` is the program's upgrade authority (with the
   * council's authorization on a governed deployment).
   */
  create(creator: Address, oracleProgram: Address, params: CreateFuturesMarketParams): TransactionInstruction {
    const programData = PublicKey.findProgramAddressSync([this.program.toBytes()], BPF_LOADER_UPGRADEABLE_PROGRAM_ID)[0];
    return this.ix({ kind: "createMarket", ...params }, [
      rw(this.market),
      rw(this.registry),
      ro(this.period),
      ro(toPublicKey(oracleProgram)),
      ro(this.mint),
      ro(this.vault),
      rw(toPublicKey(creator), true),
      ro(SYSTEM_PROGRAM_ID),
      ro(this.program),
      ro(programData),
    ]);
  }

  /** Deposits `amount` atoms from `token` (the owner's USDC account) into the owner's free cash. */
  deposit(owner: Address, token: Address, amount: IntegerLike): TransactionInstruction {
    return this.ix({ kind: "deposit", amount }, [
      rw(this.market), ro(toPublicKey(owner), true), rw(toPublicKey(token)), rw(this.vault), ro(TOKEN_PROGRAM_ID),
    ]);
  }

  /** Pays free cash only; `amount` defaults to `u64::MAX`, all of it. */
  withdraw(owner: Address, token: Address, amount: IntegerLike = U64_MAX): TransactionInstruction {
    return this.ix({ kind: "withdraw", amount }, [
      rw(this.market), ro(toPublicKey(owner), true), rw(toPublicKey(token)), rw(this.vault), ro(TOKEN_PROGRAM_ID),
      ro(this.authority),
    ]);
  }

  /**
   * `Submit`: trades one claim shape against the book with the verified
   * package `proposal`, then (`gtc`/`post`) rests the remainder. Send it with
   * `heapFrame()` and a 1.4M compute limit.
   */
  submit(owner: Address, args: FuturesSubmitArgs, proposal: Uint8Array): TransactionInstruction {
    return this.ix({ kind: "submit", ...args, proposal }, this.trading(owner));
  }

  /** Closes (part of) a claim: `submit` with `closeArgs(claim, k, options)`. */
  close(owner: Address, claim: Pick<FuturesClaim, "slot" | "seq" | "side" | "lo" | "hi" | "lots" | "reserved">, k: IntegerLike,
    proposal: Uint8Array, options: { readonly lots?: IntegerLike; readonly limit?: IntegerLike; readonly minFill?: IntegerLike } = {},
  ): TransactionInstruction {
    return this.submit(owner, closeArgs(claim, k, options), proposal);
  }

  /** Cancels the owner's resting order `(slot, seq)`, releasing its escrow or claim reservation. */
  cancel(owner: Address, slot: number, seq: IntegerLike): TransactionInstruction {
    return this.ix({ kind: "cancel", slot, seq }, this.owned(owner));
  }

  /** Publishes the owner's maker ladder (post-only: `proposal` carries the book's potentials). Send it with `heapFrame()`. */
  quote(owner: Address, ladder: FuturesLadderArgs, knots: readonly FuturesKnot[], proposal: Uint8Array): TransactionInstruction {
    return this.quoteCore(owner, ladder, { lo: ladder.lo, hi: ladder.hi }, knots, proposal);
  }

  /** A ladder whose steps outside `core` quote only their last (widest) tier. */
  quoteCore(owner: Address, ladder: FuturesLadderArgs, core: { readonly lo: number; readonly hi: number },
    knots: readonly FuturesKnot[], proposal: Uint8Array): TransactionInstruction {
    return this.ix({ kind: "quote", ...ladder, knots, coreLo: core.lo, coreHi: core.hi, proposal }, this.trading(owner));
  }

  cancelQuote(owner: Address): TransactionInstruction {
    return this.ix({ kind: "cancelQuote" }, this.owned(owner));
  }

  /** Splits a claim at `at` into `[lo, at)` and `[at, hi)`, same lots. */
  split(owner: Address, claim: FuturesRef, at: number): TransactionInstruction {
    return this.ix({ kind: "split", claim: claim.slot, seq: claim.seq, at }, this.owned(owner));
  }

  /** Merges two of the owner's claims on one side: identical intervals, or adjacent ones with equal lots. */
  merge(owner: Address, keep: FuturesRef, absorb: FuturesRef): TransactionInstruction {
    return this.ix({ kind: "merge", keep: keep.slot, keepSeq: keep.seq, absorb: absorb.slot, absorbSeq: absorb.seq },
      this.owned(owner));
  }

  /** Redeems `lots` of an exact complement pair (same interval) for its backing. */
  redeem(owner: Address, long: FuturesRef, short: FuturesRef, lots: IntegerLike): TransactionInstruction {
    return this.ix({ kind: "redeem", long: long.slot, longSeq: long.seq, short: short.slot, shortSeq: short.seq, lots },
      this.owned(owner));
  }

  /** Gives `lots` of a claim to `to` unchanged. */
  transfer(owner: Address, claim: FuturesRef, lots: IntegerLike, to: Address): TransactionInstruction {
    return this.ix({ kind: "transfer", claim: claim.slot, seq: claim.seq, lots, to }, this.owned(owner));
  }

  /** Permissionless: records the period's settlement. */
  finalize(): TransactionInstruction {
    return this.ix({ kind: "finalize" }, [rw(this.market), ro(this.period)]);
  }

  /**
   * `CouncilSettle`: settles a market whose oracle period failed, or is unsettled at or after expiry, at `value`
   * (oracle units at the market's decimals). `authority` signs; on a governed program append the council config and
   * three seats (as for `create`).
   */
  councilSettle(authority: Address, value: IntegerLike): TransactionInstruction {
    const programData = PublicKey.findProgramAddressSync([this.program.toBytes()], BPF_LOADER_UPGRADEABLE_PROGRAM_ID)[0];
    return this.ix({ kind: "councilSettle", value }, [
      rw(this.market), ro(this.period), ro(toPublicKey(authority), true), ro(this.program), ro(programData),
    ]);
  }

  /** Permissionless after finalization: pays claims (by slot) into free balances. */
  settle(claims: readonly number[]): TransactionInstruction {
    return this.ix({ kind: "settle", claims }, [rw(this.market)]);
  }

  /** Permissionless after finalization: pays a maker's ladder inventory and releases its escrow. */
  settleMaker(maker: number): TransactionInstruction {
    return this.ix({ kind: "settleMaker", maker }, [rw(this.market)]);
  }

  /** Permissionless after finalization: releases resting orders' escrow. */
  prune(orders: readonly number[]): TransactionInstruction {
    return this.ix({ kind: "prune", orders }, [rw(this.market)]);
  }

  // --------------------------------------------------------------- Legacy Earn pools (dev)
  /** The legacy Earn pool `operator` runs in this market (dev only). */
  pool(operator: Address): PublicKey {
    return findPoolAddress(this.program, this.market, operator)[0];
  }

  /** `owner`'s position account in a legacy `pool`. */
  lp(pool: Address, owner: Address): PublicKey {
    return findLpAddress(this.program, pool, owner)[0];
  }

  /**
   * `PoolMigrate` (dev builds): moves the legacy `pool`, whose shares `holder`
   * owns entirely, into the Earn vault. The pool's trader slot becomes the
   * vault's in this market and the holder gets vault shares; the holder and
   * the vault's operator sign, `payer` funds the holder's vault position the
   * first time. `markets`: the vault's, in its order (`FuturesVault.markets`).
   */
  poolMigrate(pool: Address, holder: Address, vault: FuturesEarnVault, payer: Address,
    markets: readonly Address[]): TransactionInstruction {
    return this.ix({ kind: "poolMigrate" }, [
      rw(this.market), rw(toPublicKey(pool)), rw(this.lp(pool, holder)), rw(vault.vault), rw(vault.position(holder)),
      ro(toPublicKey(holder), true), ro(vault.operator, true), rw(toPublicKey(payer), true), ro(SYSTEM_PROGRAM_ID),
      ...markets.map((m) => ro(toPublicKey(m))),
    ]);
  }

  /**
   * `PoolWithdraw` (legacy pools' exit, dev builds): redeems `shares`
   * (default `u64::MAX`, all) at the pool's lowest value, paid from its free cash.
   */
  poolWithdraw(pool: Address, owner: Address, token: Address, shares: IntegerLike = U64_MAX): TransactionInstruction {
    return this.ix({ kind: "poolWithdraw", shares }, [
      rw(this.market), rw(toPublicKey(pool)), rw(this.lp(pool, owner)), ro(toPublicKey(owner), true), rw(toPublicKey(token)),
      rw(this.vault), ro(TOKEN_PROGRAM_ID), ro(this.authority),
    ]);
  }
}

/** A market the Earn vault funds, defunds or draws from: a `FuturesMarket`, or just these addresses. */
export type FuturesVaultMarket = Pick<FuturesMarket, "market" | "vault" | "authority">;

/**
 * The Earn vault's addresses and instructions (mirrors the Rust client's
 * `EarnVault`): one vault for the whole product, run by `operator`, its idle
 * cash in `cash`. Depositors get shares of everything the vault holds; the
 * operator trades the vault's trader slot in each of its markets
 * (`operated`) and moves cash between them and the idle cash (`fund`,
 * `defund`), but can never withdraw it.
 */
export class FuturesEarnVault {
  readonly program: PublicKey;
  /** The vault's address, `["ameba-range-v1", "earn", operator]`. */
  readonly vault: PublicKey;
  readonly operator: PublicKey;
  /** Its idle cash: a token account of `mint` owned by `vault`. */
  readonly cash: PublicKey;
  /** The quote (USDC) mint. */
  readonly mint: PublicKey;

  private constructor(program: PublicKey, operator: PublicKey, cash: PublicKey, mint: PublicKey) {
    this.program = program;
    this.vault = FuturesEarnVault.address(program, operator);
    this.operator = operator;
    this.cash = cash;
    this.mint = mint;
  }

  /** The vault `operator` runs, with `cash` as its token account (the client's `EarnVault::new`). */
  static derive(program: Address, operator: Address, accounts: { readonly cash: Address; readonly mint: Address }): FuturesEarnVault {
    return new FuturesEarnVault(toPublicKey(program), toPublicKey(operator), toPublicKey(accounts.cash),
      toPublicKey(accounts.mint));
  }

  /** From the decoded vault account (`decodeVault`; the client's `EarnVault::of`). */
  static fromAccount(program: Address, vault: Pick<FuturesVault, "operator" | "cashAccount" | "quoteMint">): FuturesEarnVault {
    return FuturesEarnVault.derive(program, vault.operator, { cash: vault.cashAccount, mint: vault.quoteMint });
  }

  /** The address of the vault `operator` runs. */
  static address(program: Address, operator: Address): PublicKey {
    return findVaultAddress(program, operator)[0];
  }

  /** `owner`'s position account. */
  position(owner: Address): PublicKey {
    return findVaultPositionAddress(this.program, this.vault, owner)[0];
  }

  private ix(data: FuturesInstruction, keys: AccountMeta[]): TransactionInstruction {
    return new TransactionInstruction({ programId: this.program, keys, data: encodeFuturesInstruction(data) });
  }

  /**
   * `CreateVault`, once `cash` (an empty token account of `mint` owned by the
   * vault's address) exists. The operator signs; `payer` funds the rent.
   */
  create(payer: Address): TransactionInstruction {
    return this.ix({ kind: "createVault" }, [
      rw(this.vault), ro(this.operator, true), rw(toPublicKey(payer), true), ro(this.cash), ro(this.mint), ro(SYSTEM_PROGRAM_ID),
    ]);
  }

  /**
   * `VaultDeposit`: `amount` atoms from `token` for shares priced at the
   * vault's highest value over every settlement outcome (1:1 into an empty
   * vault). `markets` are the vault's, in its order (`FuturesVault.markets`);
   * `payer` funds the position account the first time.
   */
  deposit(owner: Address, token: Address, amount: IntegerLike, payer: Address, markets: readonly Address[]): TransactionInstruction {
    return this.ix({ kind: "vaultDeposit", amount }, [
      rw(this.vault), rw(this.position(owner)), ro(toPublicKey(owner), true), rw(toPublicKey(token)), rw(this.cash),
      ro(TOKEN_PROGRAM_ID), ro(SYSTEM_PROGRAM_ID), rw(toPublicKey(payer), true),
      ...markets.map((m) => ro(toPublicKey(m))),
    ]);
  }

  /**
   * `VaultWithdraw`: redeems `shares` (`U64_MAX`: all) at the vault's lowest
   * value over every settlement outcome, paid to `token` from the idle cash,
   * then from the vault's free cash in `draws` (markets of `markets`, in that
   * order). `markets` are the vault's, in its order; those drawn from are
   * writable. The vault's last shares wait while it may still pay more
   * (`NotSettled`).
   */
  withdraw(owner: Address, token: Address, shares: IntegerLike, markets: readonly Address[],
    draws: readonly FuturesVaultMarket[] = []): TransactionInstruction {
    const drawn = (m: PublicKey) => draws.some((d) => d.market.equals(m));
    return this.ix({ kind: "vaultWithdraw", shares }, [
      rw(this.vault), rw(this.position(owner)), ro(toPublicKey(owner), true), rw(toPublicKey(token)), rw(this.cash),
      ro(TOKEN_PROGRAM_ID),
      ...markets.map((a) => {
        const m = toPublicKey(a);
        return drawn(m) ? rw(m) : ro(m);
      }),
      ...draws.flatMap((d) => [rw(d.vault), ro(d.authority)]),
    ]);
  }

  /** `VaultFund`: the operator moves `amount` of idle cash into the vault's trader slot in `m` (adding `m` to its markets). */
  fund(m: FuturesVaultMarket, amount: IntegerLike): TransactionInstruction {
    return this.ix({ kind: "vaultFund", amount }, [
      rw(m.market), rw(this.vault), rw(this.cash), rw(m.vault), ro(TOKEN_PROGRAM_ID), ro(this.operator, true),
    ]);
  }

  /**
   * `VaultDefund`: the operator moves `amount` (default `u64::MAX`, all) of
   * the vault's free cash in `m` back to idle cash; a market where the vault
   * then holds nothing leaves its markets.
   */
  defund(m: FuturesVaultMarket, amount: IntegerLike = U64_MAX): TransactionInstruction {
    return this.ix({ kind: "vaultDefund", amount }, [
      rw(m.market), rw(this.vault), rw(this.cash), rw(m.vault), ro(TOKEN_PROGRAM_ID), ro(m.authority), ro(this.operator, true),
    ]);
  }
}

/**
 * `ix` built for the Earn vault (or a legacy pool) as its owner (`submit`,
 * `cancel`, `quote`, `cancelQuote`, `split`, `merge` or `redeem`), signed by
 * its operator instead: the vault is no signer and the operator signs as the
 * last account (the client's `operated`). The market must be one of the
 * vault's. Withdrawals and transfers never act for the vault: its cash moves
 * only between the vault and its markets and leaves only to depositors.
 */
export function operated(ix: TransactionInstruction, operator: Address): TransactionInstruction {
  const keys = ix.keys.map((k, i) => (i === 1 ? { ...k, isSigner: false } : { ...k }));
  keys.push(ro(toPublicKey(operator), true));
  return new TransactionInstruction({ programId: ix.programId, keys, data: Buffer.from(ix.data) });
}
