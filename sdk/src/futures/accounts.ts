/**
 * Futures market account decoders (`programs/futures/src/state.rs`): one
 * account per market, a 512-byte header followed by four fixed arenas, all
 * little-endian at fixed offsets.
 *
 * | Region | Record | Size |
 * |---|---|---|
 * | header | `FuturesHeader` | 512 |
 * | traders | `FuturesTrader` × `traderCap` | 64 each |
 * | orders | `FuturesOrder` × `orderCap` | 64 each |
 * | makers | `FuturesMaker` head + `grid` × `FuturesStep` | 64 + 32·grid each |
 * | claims | `FuturesClaim` × `claimCap` | 56 each |
 *
 * `FuturesBook` mirrors the Rust client's `Book`: the live records, the
 * complete arc list the program verifies packages against (`arcs`), and what
 * a trader's holdings are worth over every settlement outcome (`valueOf`).
 */
import { Buffer } from "buffer";
import { PublicKey, type GetProgramAccountsFilter } from "@solana/web3.js";
import { ByteReader, ascii, toPublicKey, type Address } from "./codec.js";
import { productSymbol } from "./pda.js";
import {
  CLAIM_LEN, FUTURES_MAGIC, FUTURES_VERSION, HEADER_LEN, LP_LEN, LP_MAGIC, MAKER_HEAD, MAX_TIERS, ORDER_CLOSE, ORDER_LEN,
  POOL_LEN, POOL_MAGIC, REGISTRY_LEN, REGISTRY_MAGIC, STATUS_NAMES, STEP_LEN, TRADER_LEN, VAULT_HEAD, VAULT_LEN, VAULT_LP_LEN, VAULT_REWARDS_LEN, REWARDS_MAGIC,
  VAULT_LP_MAGIC, VAULT_MAGIC, VAULT_MARKETS,
  FuturesSide, FuturesStatus, longPerLot, makerLen, marketLen, shortPerLot, type FuturesStatusName,
} from "./program.js";

/** A futures account that does not decode: wrong magic, version or length. */
export class FuturesAccountError extends Error {
  override readonly name = "FuturesAccountError";
}

const U64 = (1n << 64n) - 1n;
const DEFAULT_KEY = PublicKey.default;

// ----------------------------------------------------------------- Records
export interface FuturesHeader {
  /** 0 open, 1 settled, 2 failed (settles at the reference value). */
  readonly status: number;
  readonly statusName: FuturesStatusName | "unknown";
  readonly authorityBump: number;
  readonly valueDecimals: number;
  /** Highest grid position; positions are `0..=grid`. */
  readonly grid: number;
  readonly traderCap: number;
  readonly orderCap: number;
  readonly makerCap: number;
  readonly registryBump: number;
  readonly claimCap: number;
  /** Where the program's search for a free claim slot starts. */
  readonly claimHint: number;
  readonly productId: Uint8Array;
  readonly symbol: string;
  readonly expiryTs: bigint;
  readonly period: PublicKey;
  readonly oracleProgram: PublicKey;
  readonly quoteMint: PublicKey;
  readonly vault: PublicKey;
  readonly creator: PublicKey;
  /** Atoms one lot of a one-step claim pays in full. */
  readonly k: bigint;
  /** Oracle value units per grid step (at `valueDecimals`). */
  readonly stepValue: bigint;
  /** Reference settlement (fine units), used if the period fails. */
  readonly refFine: bigint;
  readonly settlementFine: bigint;
  /** Arrival sequence: every order, ladder update and claim takes the next. */
  readonly nextSeq: bigint;
  readonly totalFree: bigint;
  readonly totalReserved: bigint;
  /** Cash backing every open complete set. */
  readonly backing: bigint;
  /** Deposits minus withdrawals: equals free + reserved + backing. */
  readonly custody: bigint;
  readonly trades: bigint;
  readonly volumeLots: bigint;
  readonly volumePremium: bigint;
  /** Average entry of the last trade, fine units (0: none yet). */
  readonly lastEntryFine: bigint;
  readonly claimsOpen: bigint;
  readonly totalPaidOut: bigint;
  readonly settledTs: bigint;
  readonly lastTradeTs: bigint;
}

export interface FuturesTrader {
  readonly slot: number;
  /** The default key: an empty slot. */
  readonly owner: PublicKey;
  readonly free: bigint;
  readonly reserved: bigint;
  readonly claims: number;
  readonly orders: number;
  /** Maker ladder slot + 1 (0: none). */
  readonly maker: number;
  readonly paidOut: bigint;
}

export interface FuturesOrder {
  readonly slot: number;
  /** Arrival sequence (0: an empty slot). Orders are named by `(slot, seq)`. */
  readonly seq: bigint;
  /** Trader slot. */
  readonly trader: number;
  /** 0 long, 1 short. */
  readonly side: number;
  readonly flags: number;
  /** `flags & ORDER_CLOSE`: closes (and reserves) the owner's claim `claim`. */
  readonly close: boolean;
  readonly lo: number;
  readonly hi: number;
  /** Lots remaining. */
  readonly lots: bigint;
  /** Limit premium per lot; a resting order trades at exactly this. */
  readonly price: bigint;
  /** Cash reserved per remaining lot (0 for a close order). */
  readonly escrowPerLot: bigint;
  readonly claim: number;
  readonly placedLots: bigint;
  readonly placedTs: bigint;
}

export interface FuturesClaim {
  readonly slot: number;
  /** Lots (0: an empty slot). */
  readonly lots: bigint;
  /** Changes on every split, merge, transfer and partial use. Claims are named by `(slot, seq)`. */
  readonly seq: bigint;
  readonly trader: number;
  /** 0 long, 1 short. */
  readonly side: number;
  readonly lo: number;
  readonly hi: number;
  /** Cash put in for these lots (cost basis, atoms). */
  readonly cost: bigint;
  /** Lots reserved by the owner's resting close orders. */
  readonly reserved: bigint;
  readonly openedTs: bigint;
}

/** A ladder's depth tiers: `count` prices per step and side, `spread` atoms apart, each tier `ratio`% of the one before. */
export interface FuturesTiers {
  readonly count: number;
  readonly ratio: number;
  readonly spread: number;
}

/** A maker ladder's head; its steps follow it. */
export interface FuturesMaker {
  readonly slot: number;
  /** Arrival sequence of the current quote (0: no live quote). */
  readonly seq: bigint;
  readonly trader: number;
  readonly used: boolean;
  readonly lo: number;
  readonly hi: number;
  /** Cash reserved for every step's remaining lots. */
  readonly escrow: bigint;
  readonly bidSpread: number;
  readonly askSpread: number;
  readonly quotedTs: bigint;
  readonly tiers: FuturesTiers;
  /** Lots each step quoted per side when posted; with a step's remaining lots they give each tier's. */
  readonly bidTotal: bigint;
  readonly askTotal: bigint;
}

/** One ladder step `[pos, pos + 1)`: best prices (tier 0), lots left over all tiers, and the maker's inventory. */
export interface FuturesStep {
  readonly pos: number;
  readonly ask: number;
  readonly bid: number;
  readonly askLots: bigint;
  readonly bidLots: bigint;
  /** Net one-step claims held: positive long, negative short. */
  readonly inv: bigint;
}

export interface FuturesRegistry {
  readonly bump: number;
  readonly market: PublicKey;
}

/**
 * The Earn vault: one for the whole product. Depositors hold shares of
 * everything it has: its idle cash (`cash`, in `cashAccount`) and its trader
 * slot (owner: the vault's address) in each of `markets`, which its operator
 * trades and moves cash between but never withdraws.
 */
export interface FuturesVault {
  readonly bump: number;
  readonly operator: PublicKey;
  readonly quoteMint: PublicKey;
  /** The vault's USDC token account (owner: the vault's address). */
  readonly cashAccount: PublicKey;
  /** Idle cash: what `cashAccount` holds for the vault (a gift to that account is not counted). */
  readonly cash: bigint;
  /** Shares outstanding. */
  readonly shares: bigint;
  /** Cash ever deposited and withdrawn (informational). */
  readonly deposited: bigint;
  readonly withdrawn: bigint;
  readonly createdTs: bigint;
  /** Every market where the vault may hold a trader slot, in the order `VaultDeposit` and `VaultWithdraw` name them. */
  readonly markets: readonly PublicKey[];
  /** The reward stream (AMBA for Earn), once the operator has set one. */
  readonly rewards: FuturesVaultRewards | null;
}

/**
 * The Earn vault's reward stream (`state::Rewards`): `amount` atoms of `mint`, held in `account`, released linearly
 * from `startTs` to `endTs` to depositors by their shares except `house`'s. `acc` is reward atoms per share times
 * `REWARD_SCALE`, accrued to `lastTs`.
 */
export interface FuturesVaultRewards {
  readonly mint: PublicKey;
  readonly account: PublicKey;
  /** The owner whose shares earn nothing (the default key: none). */
  readonly house: PublicKey;
  readonly decimals: number;
  readonly startTs: bigint;
  readonly endTs: bigint;
  readonly amount: bigint;
  readonly lastTs: bigint;
  readonly acc: bigint;
  readonly houseShares: bigint;
  /** Atoms ever accrued to depositors, and ever paid to them. */
  readonly distributed: bigint;
  readonly paid: bigint;
}

/** A depositor's shares in the Earn vault. */
export interface FuturesVaultPosition {
  readonly bump: number;
  readonly vault: PublicKey;
  readonly owner: PublicKey;
  readonly shares: bigint;
  /** Cash this owner ever deposited and withdrew (informational). */
  readonly deposited: bigint;
  readonly withdrawn: bigint;
  /** The vault's reward `acc` at this position's last settlement. */
  readonly rewardSnapshot: bigint;
  /** Reward atoms settled and not paid yet. */
  readonly rewardPending: bigint;
  /** Reward atoms ever paid (informational; seven bytes on chain). */
  readonly rewardPaid: bigint;
}

/**
 * A legacy per-market Earn pool (dev only; the Earn vault replaced them). Its
 * cash and inventory are the market trader whose owner is the pool's address.
 */
export interface FuturesPool {
  readonly bump: number;
  readonly market: PublicKey;
  readonly operator: PublicKey;
  /** Shares outstanding. */
  readonly shares: bigint;
  /** Cash ever deposited and withdrawn (informational). */
  readonly deposited: bigint;
  readonly withdrawn: bigint;
  readonly createdTs: bigint;
}

/** A depositor's position in a legacy pool. */
export interface FuturesLpPosition {
  readonly bump: number;
  readonly pool: PublicKey;
  readonly owner: PublicKey;
  readonly shares: bigint;
  /** Cash this owner ever deposited and withdrew (informational). */
  readonly deposited: bigint;
  readonly withdrawn: bigint;
}

/** What a package arc stands for: a resting order, or one tier of one side of one ladder step. */
export type FuturesArcId =
  | { readonly kind: "order"; readonly slot: number }
  | { readonly kind: "maker"; readonly maker: number; readonly step: number; readonly side: number; readonly tier: number };

/** One source of liquidity, as the program verifies packages against it. */
export interface FuturesArc {
  readonly id: FuturesArcId;
  /** 0 long (buys the claim), 1 short. */
  readonly side: number;
  readonly lo: number;
  readonly hi: number;
  /** Premium per lot for the whole interval, atoms. */
  readonly price: bigint;
  /** Lots available. */
  readonly cap: bigint;
  /** FIFO rank: smaller is older. */
  readonly rank: bigint;
}

// ----------------------------------------------------------------- Decoders
function expectMagic(r: ByteReader, magic: string, what: string): void {
  if (!r.matches(0, ascii(magic))) throw new FuturesAccountError(`${what}: bad magic`);
}

/** Decodes the header and checks the magic, version and exact account length (`Header::read`). */
export function decodeHeader(data: Uint8Array): FuturesHeader {
  const r = new ByteReader(data);
  if (r.length < HEADER_LEN) throw new FuturesAccountError("market: too short");
  expectMagic(r, FUTURES_MAGIC, "market");
  if (r.u8(8) !== FUTURES_VERSION) throw new FuturesAccountError("market: unknown version");
  const productId = r.slice(32, 32);
  const status = r.u8(9);
  const h: FuturesHeader = {
    status,
    statusName: STATUS_NAMES[status] ?? "unknown",
    authorityBump: r.u8(10),
    valueDecimals: r.u8(11),
    grid: r.u16(12),
    traderCap: r.u16(14),
    orderCap: r.u16(16),
    makerCap: r.u8(18),
    registryBump: r.u8(19),
    claimCap: r.u32(20),
    claimHint: r.u32(24),
    productId,
    symbol: productSymbol(productId),
    expiryTs: r.u64(64),
    period: r.pubkey(72),
    oracleProgram: r.pubkey(104),
    quoteMint: r.pubkey(136),
    vault: r.pubkey(168),
    creator: r.pubkey(200),
    k: r.u64(232),
    stepValue: r.u64(240),
    refFine: r.u64(248),
    settlementFine: r.u64(256),
    nextSeq: r.u64(264),
    totalFree: r.u64(272),
    totalReserved: r.u64(280),
    backing: r.u64(288),
    custody: r.u64(296),
    trades: r.u64(304),
    volumeLots: r.u64(312),
    volumePremium: r.u128(320),
    lastEntryFine: r.u64(336),
    claimsOpen: r.u64(344),
    totalPaidOut: r.u64(352),
    settledTs: r.u64(360),
    lastTradeTs: r.u64(368),
  };
  if (r.length !== marketLen(h.grid, h.traderCap, h.orderCap, h.makerCap, h.claimCap)) {
    throw new FuturesAccountError("market: bad length");
  }
  return h;
}

/** The one-market-per-index-and-expiry registry account. */
export function decodeRegistry(data: Uint8Array): FuturesRegistry {
  const r = new ByteReader(data);
  if (r.length !== REGISTRY_LEN) throw new FuturesAccountError("registry: bad length");
  expectMagic(r, REGISTRY_MAGIC, "registry");
  return { bump: r.u8(8), market: r.pubkey(16) };
}

/**
 * The Earn vault account (`Vault::read`): `VAULT_LEN` bytes, or `VAULT_REWARDS_LEN` with a reward stream; magic,
 * and at most `VAULT_MARKETS` markets.
 */
export function decodeVault(data: Uint8Array): FuturesVault {
  const r = new ByteReader(data);
  if (r.length !== VAULT_LEN && r.length !== VAULT_REWARDS_LEN) throw new FuturesAccountError("vault: bad length");
  expectMagic(r, VAULT_MAGIC, "vault");
  const count = r.u8(9);
  if (count > VAULT_MARKETS) throw new FuturesAccountError("vault: too many markets");
  return {
    bump: r.u8(8), operator: r.pubkey(16), quoteMint: r.pubkey(48), cashAccount: r.pubkey(80), cash: r.u64(112),
    shares: r.u64(120), deposited: r.u64(128), withdrawn: r.u64(136), createdTs: r.u64(144),
    markets: Array.from({ length: count }, (_, i) => r.pubkey(VAULT_HEAD + 32 * i)),
    rewards: r.length === VAULT_REWARDS_LEN ? decodeRewards(r, VAULT_LEN) : null,
  };
}

function decodeRewards(r: ByteReader, at: number): FuturesVaultRewards {
  if (new TextDecoder().decode(r.slice(at, 8)) !== REWARDS_MAGIC) throw new FuturesAccountError("vault rewards: bad magic");
  return {
    decimals: r.u8(at + 8), mint: r.pubkey(at + 16), account: r.pubkey(at + 48), house: r.pubkey(at + 80),
    startTs: r.u64(at + 112), endTs: r.u64(at + 120), amount: r.u64(at + 128), lastTs: r.u64(at + 136),
    acc: r.u128(at + 144), houseShares: r.u64(at + 160), distributed: r.u64(at + 168), paid: r.u64(at + 176),
  };
}

/** An Earn vault position account (`VaultPosition::read`). */
export function decodeVaultPosition(data: Uint8Array): FuturesVaultPosition {
  const r = new ByteReader(data);
  if (r.length !== VAULT_LP_LEN) throw new FuturesAccountError("vault position: bad length");
  expectMagic(r, VAULT_LP_MAGIC, "vault position");
  return {
    bump: r.u8(8), vault: r.pubkey(16), owner: r.pubkey(48), shares: r.u64(80), deposited: r.u64(88), withdrawn: r.u64(96),
    rewardSnapshot: r.u128(104), rewardPending: r.u64(120), rewardPaid: r.u64(8) >> 8n,
  };
}

/** A legacy Earn pool account (`Pool::read`). */
export function decodePool(data: Uint8Array): FuturesPool {
  const r = new ByteReader(data);
  if (r.length !== POOL_LEN) throw new FuturesAccountError("pool: bad length");
  expectMagic(r, POOL_MAGIC, "pool");
  return {
    bump: r.u8(8), market: r.pubkey(16), operator: r.pubkey(48), shares: r.u64(80), deposited: r.u64(88),
    withdrawn: r.u64(96), createdTs: r.u64(104),
  };
}

/** A legacy pool position account (`LpPosition::read`). */
export function decodeLpPosition(data: Uint8Array): FuturesLpPosition {
  const r = new ByteReader(data);
  if (r.length !== LP_LEN) throw new FuturesAccountError("pool position: bad length");
  expectMagic(r, LP_MAGIC, "pool position");
  return {
    bump: r.u8(8), pool: r.pubkey(16), owner: r.pubkey(48), shares: r.u64(80), deposited: r.u64(88), withdrawn: r.u64(96),
  };
}

/** `getProgramAccounts` filter for the futures program's market accounts. */
export function marketFilters(): GetProgramAccountsFilter[] {
  return [{ memcmp: { offset: 0, bytes: Buffer.from(ascii(FUTURES_MAGIC)).toString("base64"), encoding: "base64" } }];
}

// ----------------------------------------------------------------- Tiers (state.rs `Tiers`)
const tierCount = (t: FuturesTiers): number => Math.max(t.count, 1);

/** Each tier's share of `total` lots, exact (the last takes the rest); `max(count, 1)` entries. */
export function tierSizes(t: FuturesTiers, total: bigint): bigint[] {
  const n = tierCount(t);
  if (n > MAX_TIERS) throw new RangeError(`at most ${MAX_TIERS} tiers`);
  if (n === 1) return [total];
  const w: bigint[] = [100n];
  for (let i = 1; i < n; i += 1) w.push((w[i - 1]! * BigInt(t.ratio)) / 100n);
  let sum = w.reduce((a, b) => a + b, 0n);
  if (sum < 1n) sum = 1n;
  const out: bigint[] = [];
  let used = 0n;
  for (let i = 0; i < n - 1; i += 1) {
    const size = BigInt.asUintN(64, (total * w[i]!) / sum);
    out.push(size);
    used += size;
  }
  out.push(total - used);
  return out;
}

/** Each tier's remaining lots when `remaining` of `total` are left (the best tiers trade first). */
export function tierRemaining(t: FuturesTiers, total: bigint, remaining: bigint, sizes = tierSizes(t, total)): bigint[] {
  const n = tierCount(t);
  if (n === 1) return [remaining];
  if (remaining >= total) return sizes.slice(0, n);
  let filled = total - remaining;
  return sizes.slice(0, n).map((size) => {
    const used = filled < size ? filled : size;
    filled -= used;
    return size - used;
  });
}

/** Tier `i`'s price from a step's best: bids step down (floor 0), asks up (capped at `k`). */
export function tierPrice(t: FuturesTiers, best: number, side: number, i: number, k: bigint): bigint {
  const off = BigInt(t.spread) * BigInt(i);
  if (side === FuturesSide.Long) return BigInt(best) > off ? BigInt(best) - off : 0n;
  const ask = BigInt(best) + off;
  return ask < k ? ask : k;
}

// ----------------------------------------------------------------- The book
/** `ameba_futures_math::rank`: arrival sequence, then a sub-index. */
function rank(seq: bigint, sub: number): bigint {
  return (seq << 32n) | BigInt(sub);
}

/** A ladder arc's sub-index: by step, bid before ask, then tier. */
function makerSub(step: number, side: number, tier: number): number {
  return (step * 2 + side) * MAX_TIERS + tier;
}

/** A maker slot, its head, and every step that quotes lots or holds inventory. */
export interface FuturesLadder {
  readonly maker: FuturesMaker;
  readonly steps: FuturesStep[];
}

/** A decoded market account (mirrors the Rust client's `Book`). Records are read on demand from `data`. */
export class FuturesBook {
  readonly header: FuturesHeader;
  readonly data: Uint8Array;
  private readonly r: ByteReader;

  private constructor(header: FuturesHeader, data: Uint8Array) {
    this.header = header;
    this.data = data;
    this.r = new ByteReader(data);
  }

  static decode(data: Uint8Array): FuturesBook {
    const copy = Uint8Array.from(data);
    return new FuturesBook(decodeHeader(copy), copy);
  }

  private tradersAt(): number {
    return HEADER_LEN;
  }
  private ordersAt(): number {
    return this.tradersAt() + this.header.traderCap * TRADER_LEN;
  }
  private makersAt(): number {
    return this.ordersAt() + this.header.orderCap * ORDER_LEN;
  }
  private claimsAt(): number {
    return this.makersAt() + this.header.makerCap * makerLen(this.header.grid);
  }

  private check(index: number, cap: number, what: string): void {
    if (!Number.isInteger(index) || index < 0 || index >= cap) throw new RangeError(`${what} ${index} is outside 0..${cap}`);
  }

  /** The trader record in `slot` (empty slots have the default owner). */
  trader(slot: number): FuturesTrader {
    this.check(slot, this.header.traderCap, "trader slot");
    const at = this.tradersAt() + slot * TRADER_LEN, r = this.r;
    return {
      slot, owner: r.pubkey(at), free: r.u64(at + 32), reserved: r.u64(at + 40), claims: r.u32(at + 48),
      orders: r.u16(at + 52), maker: r.u8(at + 54), paidOut: r.u64(at + 56),
    };
  }

  order(slot: number): FuturesOrder {
    this.check(slot, this.header.orderCap, "order slot");
    const at = this.ordersAt() + slot * ORDER_LEN, r = this.r;
    const flags = r.u8(at + 11);
    return {
      slot, seq: r.u64(at), trader: r.u16(at + 8), side: r.u8(at + 10), flags, close: (flags & ORDER_CLOSE) !== 0,
      lo: r.u16(at + 12), hi: r.u16(at + 14), lots: r.u64(at + 16), price: r.u64(at + 24), escrowPerLot: r.u64(at + 32),
      claim: r.u32(at + 40), placedLots: r.u64(at + 48), placedTs: r.u64(at + 56),
    };
  }

  claim(slot: number): FuturesClaim {
    this.check(slot, this.header.claimCap, "claim slot");
    const at = this.claimsAt() + slot * CLAIM_LEN, r = this.r;
    return {
      slot, lots: r.u64(at), seq: r.u64(at + 8), trader: r.u16(at + 16), side: r.u8(at + 18), lo: r.u16(at + 20),
      hi: r.u16(at + 22), cost: r.u64(at + 24), reserved: r.u64(at + 32), openedTs: r.u64(at + 40),
    };
  }

  maker(slot: number): FuturesMaker {
    this.check(slot, this.header.makerCap, "maker slot");
    const at = this.makersAt() + slot * makerLen(this.header.grid), r = this.r;
    return {
      slot, seq: r.u64(at), trader: r.u16(at + 8), used: r.u8(at + 10) !== 0, lo: r.u16(at + 12), hi: r.u16(at + 14),
      escrow: r.u64(at + 16), bidSpread: r.u32(at + 24), askSpread: r.u32(at + 28), quotedTs: r.u64(at + 32),
      tiers: { count: r.u8(at + 40), ratio: r.u16(at + 42), spread: r.u32(at + 44) },
      bidTotal: r.u64(at + 48), askTotal: r.u64(at + 56),
    };
  }

  /** Ladder step `[pos, pos + 1)` of maker `slot`. */
  step(slot: number, pos: number): FuturesStep {
    this.check(slot, this.header.makerCap, "maker slot");
    this.check(pos, this.header.grid, "step");
    const at = this.makersAt() + slot * makerLen(this.header.grid) + MAKER_HEAD + pos * STEP_LEN, r = this.r;
    return { pos, ask: r.u32(at), bid: r.u32(at + 4), askLots: r.u64(at + 8), bidLots: r.u64(at + 16), inv: r.i64(at + 24) };
  }

  /** Occupied trader slots. */
  traders(): FuturesTrader[] {
    const out: FuturesTrader[] = [];
    for (let i = 0; i < this.header.traderCap; i += 1) {
      const t = this.trader(i);
      if (!t.owner.equals(DEFAULT_KEY)) out.push(t);
    }
    return out;
  }

  traderOf(owner: Address): FuturesTrader | null {
    const key = toPublicKey(owner);
    return this.traders().find((t) => t.owner.equals(key)) ?? null;
  }

  /** Orders with a sequence (a fully filled order keeps its slot until reused). */
  orders(): FuturesOrder[] {
    const out: FuturesOrder[] = [];
    for (let i = 0; i < this.header.orderCap; i += 1) {
      const o = this.order(i);
      if (o.seq !== 0n) out.push(o);
    }
    return out;
  }

  /** Ladders in use, each with every step that quotes lots or holds inventory. */
  makers(): FuturesLadder[] {
    const out: FuturesLadder[] = [];
    for (let m = 0; m < this.header.makerCap; m += 1) {
      const maker = this.maker(m);
      if (!maker.used) continue;
      const steps: FuturesStep[] = [];
      for (let x = 0; x < this.header.grid; x += 1) {
        const st = this.step(m, x);
        if (st.inv !== 0n || st.bidLots > 0n || st.askLots > 0n) steps.push(st);
      }
      out.push({ maker, steps });
    }
    return out;
  }

  /** Claims with lots. */
  claims(): FuturesClaim[] {
    const out: FuturesClaim[] = [];
    for (let i = 0; i < this.header.claimCap; i += 1) {
      const c = this.claim(i);
      if (c.lots > 0n) out.push(c);
    }
    return out;
  }

  claimsOf(trader: number): FuturesClaim[] {
    return this.claims().filter((c) => c.trader === trader);
  }

  /**
   * Every live order and ladder step tier as package arcs, in canonical order
   * (orders by slot, then ladders by maker, step, bid before ask, best tier
   * first): exactly the list the program verifies a proposal against
   * (`state::book_arcs`).
   */
  arcs(): FuturesArc[] {
    const h = this.header;
    const arcs: FuturesArc[] = [];
    for (let slot = 0; slot < h.orderCap; slot += 1) {
      const o = this.order(slot);
      if (o.seq === 0n || o.lots === 0n) continue;
      arcs.push({
        id: { kind: "order", slot }, side: o.side === 0 ? FuturesSide.Long : FuturesSide.Short, lo: o.lo, hi: o.hi,
        price: o.price, cap: o.lots, rank: rank(o.seq, 0),
      });
    }
    for (let m = 0; m < h.makerCap; m += 1) {
      const mk = this.maker(m);
      if (!mk.used || mk.seq === 0n) continue;
      const n = tierCount(mk.tiers);
      const bidSizes = tierSizes(mk.tiers, mk.bidTotal), askSizes = tierSizes(mk.tiers, mk.askTotal);
      const seq = rank(mk.seq, 0);
      for (let x = mk.lo; x < mk.hi; x += 1) {
        const st = this.step(m, x);
        const sides: [number, bigint[], number][] = [
          [FuturesSide.Long, tierRemaining(mk.tiers, mk.bidTotal, st.bidLots, bidSizes), st.bid],
          [FuturesSide.Short, tierRemaining(mk.tiers, mk.askTotal, st.askLots, askSizes), st.ask],
        ];
        for (const [side, lots, best] of sides) {
          for (let tier = 0; tier < n; tier += 1) {
            const cap = lots[tier]!;
            if (cap === 0n) continue;
            arcs.push({
              id: { kind: "maker", maker: m, step: x, side, tier }, side, lo: x, hi: x + 1,
              price: tierPrice(mk.tiers, best, side, tier, h.k), cap, rank: seq | BigInt(makerSub(x, side, tier)),
            });
          }
        }
      }
    }
    return arcs;
  }

  /**
   * What trader `slot` holds is worth, `[lowest, highest]` atoms over every
   * settlement outcome: free and reserved cash plus everything its claims and
   * ladder inventory pay (the one outcome once the market is final). Null on
   * a `u64` overflow, as `state::trader_value`.
   */
  traderValue(slot: number): readonly [bigint, bigint] | null {
    const h = this.header;
    const tr = this.trader(slot);
    const cash = tr.free + tr.reserved;
    const open = h.status === FuturesStatus.Open;
    const n = h.grid + 1;
    const delta: bigint[] = new Array<bigint>(n + 1).fill(0n);
    let base = 0n, owed = 0n;
    const add = (long: boolean, lo: number, hi: number, lots: bigint) => {
      const sign = long ? 1n : -1n;
      if (!long) base += lots * BigInt(hi - lo);
      delta[lo] = delta[lo]! + sign * lots;
      delta[hi] = delta[hi]! - sign * lots;
      if (!open) {
        const per = long ? longPerLot(lo, hi, h.settlementFine, h.k) : shortPerLot(lo, hi, h.settlementFine, h.k);
        owed += lots * per;
      }
    };
    if (tr.maker > 0) {
      for (let x = 0; x < h.grid; x += 1) {
        const inv = this.step(tr.maker - 1, x).inv;
        if (inv !== 0n) add(inv > 0n, x, x + 1, inv < 0n ? -inv : inv);
      }
    }
    for (let i = 0; i < h.claimCap; i += 1) {
      const c = this.claim(i);
      if (c.lots > 0n && c.trader === slot) add(c.side === FuturesSide.Long, c.lo, c.hi, c.lots);
    }
    const fits = (v: bigint) => v >= 0n && v <= U64;
    if (!open) {
      const v = cash + owed;
      return fits(v) ? [v, v] : null;
    }
    let lo: bigint | null = null, hi: bigint | null = null, d = 0n, slope = 0n;
    for (let x = 0; x < n; x += 1) {
      const v = base + d;
      if (lo === null || v < lo) lo = v;
      if (hi === null || v > hi) hi = v;
      slope += delta[x]!;
      d += slope;
    }
    const low = cash + lo! * h.k, high = cash + hi! * h.k;
    return fits(low) && fits(high) ? [low, high] : null;
  }

  /** `traderValue` of `owner`'s trader, `[0, 0]` when absent (`Book::value_of`). */
  valueOf(owner: Address): readonly [bigint, bigint] {
    const t = this.traderOf(owner);
    return (t && this.traderValue(t.slot)) ?? [0n, 0n];
  }
}

/** Decodes a market account (alias of `FuturesBook.decode`). */
export function decodeMarket(data: Uint8Array): FuturesBook {
  return FuturesBook.decode(data);
}
