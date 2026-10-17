import { PublicKey, type GetProgramAccountsFilter } from "@solana/web3.js";
import { type Address } from "./codec.js";
import { type FuturesStatusName } from "./program.js";
/** A futures account that does not decode: wrong magic, version or length. */
export declare class FuturesAccountError extends Error {
    readonly name = "FuturesAccountError";
}
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
export type FuturesArcId = {
    readonly kind: "order";
    readonly slot: number;
} | {
    readonly kind: "maker";
    readonly maker: number;
    readonly step: number;
    readonly side: number;
    readonly tier: number;
};
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
/** Decodes the header and checks the magic, version and exact account length (`Header::read`). */
export declare function decodeHeader(data: Uint8Array): FuturesHeader;
/** The one-market-per-index-and-expiry registry account. */
export declare function decodeRegistry(data: Uint8Array): FuturesRegistry;
/**
 * The Earn vault account (`Vault::read`): `VAULT_LEN` bytes, or `VAULT_REWARDS_LEN` with a reward stream; magic,
 * and at most `VAULT_MARKETS` markets.
 */
export declare function decodeVault(data: Uint8Array): FuturesVault;
/** An Earn vault position account (`VaultPosition::read`). */
export declare function decodeVaultPosition(data: Uint8Array): FuturesVaultPosition;
/** A legacy Earn pool account (`Pool::read`). */
export declare function decodePool(data: Uint8Array): FuturesPool;
/** A legacy pool position account (`LpPosition::read`). */
export declare function decodeLpPosition(data: Uint8Array): FuturesLpPosition;
/** `getProgramAccounts` filter for the futures program's market accounts. */
export declare function marketFilters(): GetProgramAccountsFilter[];
/** Each tier's share of `total` lots, exact (the last takes the rest); `max(count, 1)` entries. */
export declare function tierSizes(t: FuturesTiers, total: bigint): bigint[];
/** Each tier's remaining lots when `remaining` of `total` are left (the best tiers trade first). */
export declare function tierRemaining(t: FuturesTiers, total: bigint, remaining: bigint, sizes?: bigint[]): bigint[];
/** Tier `i`'s price from a step's best: bids step down (floor 0), asks up (capped at `k`). */
export declare function tierPrice(t: FuturesTiers, best: number, side: number, i: number, k: bigint): bigint;
/** A maker slot, its head, and every step that quotes lots or holds inventory. */
export interface FuturesLadder {
    readonly maker: FuturesMaker;
    readonly steps: FuturesStep[];
}
/** A decoded market account (mirrors the Rust client's `Book`). Records are read on demand from `data`. */
export declare class FuturesBook {
    readonly header: FuturesHeader;
    readonly data: Uint8Array;
    private readonly r;
    private constructor();
    static decode(data: Uint8Array): FuturesBook;
    private tradersAt;
    private ordersAt;
    private makersAt;
    private claimsAt;
    private check;
    /** The trader record in `slot` (empty slots have the default owner). */
    trader(slot: number): FuturesTrader;
    order(slot: number): FuturesOrder;
    claim(slot: number): FuturesClaim;
    maker(slot: number): FuturesMaker;
    /** Ladder step `[pos, pos + 1)` of maker `slot`. */
    step(slot: number, pos: number): FuturesStep;
    /** Occupied trader slots. */
    traders(): FuturesTrader[];
    traderOf(owner: Address): FuturesTrader | null;
    /** Orders with a sequence (a fully filled order keeps its slot until reused). */
    orders(): FuturesOrder[];
    /** Ladders in use, each with every step that quotes lots or holds inventory. */
    makers(): FuturesLadder[];
    /** Claims with lots. */
    claims(): FuturesClaim[];
    claimsOf(trader: number): FuturesClaim[];
    /**
     * Every live order and ladder step tier as package arcs, in canonical order
     * (orders by slot, then ladders by maker, step, bid before ask, best tier
     * first): exactly the list the program verifies a proposal against
     * (`state::book_arcs`).
     */
    arcs(): FuturesArc[];
    /**
     * What trader `slot` holds is worth, `[lowest, highest]` atoms over every
     * settlement outcome: free and reserved cash plus everything its claims and
     * ladder inventory pay (the one outcome once the market is final). Null on
     * a `u64` overflow, as `state::trader_value`.
     */
    traderValue(slot: number): readonly [bigint, bigint] | null;
    /** `traderValue` of `owner`'s trader, `[0, 0]` when absent (`Book::value_of`). */
    valueOf(owner: Address): readonly [bigint, bigint];
}
/** Decodes a market account (alias of `FuturesBook.decode`). */
export declare function decodeMarket(data: Uint8Array): FuturesBook;
//# sourceMappingURL=accounts.d.ts.map