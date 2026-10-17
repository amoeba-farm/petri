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
import { PublicKey, TransactionInstruction } from "@solana/web3.js";
import { type Address, type IntegerLike } from "./codec.js";
import { type FuturesModeLike, type FuturesSideLike } from "./program.js";
import type { FuturesClaim, FuturesHeader, FuturesVault } from "./accounts.js";
/** Enum tags of `FuturesInstruction`. */
export declare const FuturesInstructionTag: {
    readonly CreateMarket: 0;
    readonly Deposit: 1;
    readonly Withdraw: 2;
    readonly Submit: 3;
    readonly Cancel: 4;
    readonly Quote: 5;
    readonly CancelQuote: 6;
    readonly Split: 7;
    readonly Merge: 8;
    readonly Redeem: 9;
    readonly Transfer: 10;
    readonly Finalize: 11;
    readonly Settle: 12;
    readonly SettleMaker: 13;
    readonly Prune: 14;
    /** Retired: always `InvalidInstruction` (kept so later tags hold). */
    readonly CreatePool: 15;
    /** Retired: always `InvalidInstruction`. */
    readonly PoolDeposit: 16;
    /** Legacy pools' exit (dev builds only). */
    readonly PoolWithdraw: 17;
    readonly CouncilSettle: 18;
    readonly CreateVault: 19;
    readonly VaultDeposit: 20;
    readonly VaultWithdraw: 21;
    readonly VaultFund: 22;
    readonly VaultDefund: 23;
    /** Legacy pools into the vault (dev builds only). */
    readonly PoolMigrate: 24;
    /** The operator sets the Earn vault's reward stream (AMBA for Earn). */
    readonly VaultRewards: 25;
    /** Pays a position's rewards into its owner's account (anyone may send it). */
    readonly VaultRewardsClaim: 26;
    /** The operator takes back the reward account's surplus. */
    readonly VaultRewardsWithdraw: 27;
};
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
export declare function marketSpace(p: Pick<CreateFuturesMarketParams, "grid" | "traderCap" | "orderCap" | "makerCap" | "claimCap">): number;
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
    readonly tiers: {
        readonly count: number;
        readonly ratio: number;
        readonly spread: number;
    };
}
export type FuturesInstruction = ({
    readonly kind: "createMarket";
} & CreateFuturesMarketParams) | {
    readonly kind: "deposit";
    readonly amount: IntegerLike;
} | {
    readonly kind: "withdraw";
    readonly amount: IntegerLike;
} | ({
    readonly kind: "submit";
    readonly proposal: Uint8Array;
} & FuturesSubmitArgs) | {
    readonly kind: "cancel";
    readonly slot: number;
    readonly seq: IntegerLike;
} | ({
    readonly kind: "quote";
    readonly knots: readonly FuturesKnot[];
    readonly coreLo: number;
    readonly coreHi: number;
    readonly proposal: Uint8Array;
} & FuturesLadderArgs) | {
    readonly kind: "cancelQuote";
} | {
    readonly kind: "split";
    readonly claim: number;
    readonly seq: IntegerLike;
    readonly at: number;
} | {
    readonly kind: "merge";
    readonly keep: number;
    readonly keepSeq: IntegerLike;
    readonly absorb: number;
    readonly absorbSeq: IntegerLike;
} | {
    readonly kind: "redeem";
    readonly long: number;
    readonly longSeq: IntegerLike;
    readonly short: number;
    readonly shortSeq: IntegerLike;
    readonly lots: IntegerLike;
} | {
    readonly kind: "transfer";
    readonly claim: number;
    readonly seq: IntegerLike;
    readonly lots: IntegerLike;
    readonly to: Address;
} | {
    readonly kind: "finalize";
} | {
    readonly kind: "settle";
    readonly claims: readonly number[];
} | {
    readonly kind: "settleMaker";
    readonly maker: number;
} | {
    readonly kind: "prune";
    readonly orders: readonly number[];
}
/** Retired (the program refuses it); encodable only so decoded data round-trips. */
 | {
    readonly kind: "createPool";
    readonly retired?: true;
}
/** Retired (the program refuses it); encodable only so decoded data round-trips. */
 | {
    readonly kind: "poolDeposit";
    readonly amount: IntegerLike;
    readonly retired?: true;
} | {
    readonly kind: "poolWithdraw";
    readonly shares: IntegerLike;
} | {
    readonly kind: "councilSettle";
    readonly value: IntegerLike;
} | {
    readonly kind: "createVault";
} | {
    readonly kind: "vaultDeposit";
    readonly amount: IntegerLike;
} | {
    readonly kind: "vaultWithdraw";
    readonly shares: IntegerLike;
} | {
    readonly kind: "vaultFund";
    readonly amount: IntegerLike;
} | {
    readonly kind: "vaultDefund";
    readonly amount: IntegerLike;
} | {
    readonly kind: "poolMigrate";
} | {
    readonly kind: "vaultRewards";
    readonly startTs: IntegerLike;
    readonly endTs: IntegerLike;
    readonly amount: IntegerLike;
    readonly house: Address;
} | {
    readonly kind: "vaultRewardsClaim";
} | {
    readonly kind: "vaultRewardsWithdraw";
    readonly amount: IntegerLike;
};
/** Borsh bytes of one futures instruction (without the governance tail). */
export declare function encodeFuturesInstruction(ix: FuturesInstruction): Buffer;
/** Decoded instruction with integers normalized (u64 as bigint, keys as `PublicKey`). */
export type DecodedFuturesInstruction = {
    kind: "createMarket";
    productId: Uint8Array;
    symbol: string;
    expiryTs: bigint;
    grid: number;
    k: bigint;
    stepValue: bigint;
    valueDecimals: number;
    refValue: bigint;
    traderCap: number;
    orderCap: number;
    makerCap: number;
    claimCap: number;
} | {
    kind: "deposit";
    amount: bigint;
} | {
    kind: "withdraw";
    amount: bigint;
} | {
    /** 0 long, 1 short (as on the wire; the program rejects anything else). */
    kind: "submit";
    side: number;
    lo: number;
    hi: number;
    lots: bigint;
    limit: bigint;
    minFill: bigint;
    /** 0 ioc, 1 gtc, 2 post (as on the wire). */
    mode: number;
    close: {
        slot: number;
        seq: bigint;
    } | null;
    proposal: Uint8Array;
} | {
    kind: "cancel";
    slot: number;
    seq: bigint;
} | {
    kind: "quote";
    lo: number;
    hi: number;
    knots: FuturesKnot[];
    bidSpread: number;
    askSpread: number;
    bidLots: bigint;
    askLots: bigint;
    keep: boolean;
    tiers: {
        count: number;
        ratio: number;
        spread: number;
    };
    coreLo: number;
    coreHi: number;
    proposal: Uint8Array;
} | {
    kind: "cancelQuote";
} | {
    kind: "split";
    claim: number;
    seq: bigint;
    at: number;
} | {
    kind: "merge";
    keep: number;
    keepSeq: bigint;
    absorb: number;
    absorbSeq: bigint;
} | {
    kind: "redeem";
    long: number;
    longSeq: bigint;
    short: number;
    shortSeq: bigint;
    lots: bigint;
} | {
    kind: "transfer";
    claim: number;
    seq: bigint;
    lots: bigint;
    to: PublicKey;
} | {
    kind: "finalize";
} | {
    kind: "settle";
    claims: number[];
} | {
    kind: "settleMaker";
    maker: number;
} | {
    kind: "prune";
    orders: number[];
}
/** Tag 15, retired: the program refuses it. */
 | {
    kind: "createPool";
    retired: true;
}
/** Tag 16, retired: the program refuses it. */
 | {
    kind: "poolDeposit";
    amount: bigint;
    retired: true;
} | {
    kind: "poolWithdraw";
    shares: bigint;
} | {
    kind: "councilSettle";
    value: bigint;
} | {
    kind: "createVault";
} | {
    kind: "vaultDeposit";
    amount: bigint;
} | {
    kind: "vaultWithdraw";
    shares: bigint;
} | {
    kind: "vaultFund";
    amount: bigint;
} | {
    kind: "vaultDefund";
    amount: bigint;
} | {
    kind: "poolMigrate";
} | {
    kind: "vaultRewards";
    startTs: bigint;
    endTs: bigint;
    amount: bigint;
    house: PublicKey;
} | {
    kind: "vaultRewardsClaim";
} | {
    kind: "vaultRewardsWithdraw";
    amount: bigint;
};
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
export declare function decodeFuturesInstruction(data: Uint8Array, options?: DecodeFuturesInstructionOptions): DecodedFuturesInstruction;
/** `ComputeBudgetInstruction::RequestHeapFrame`; `Submit` and `Quote` need `HEAP_FRAME` (the client's `heap_frame`). */
export declare function heapFrame(bytes?: number): TransactionInstruction;
/** `ComputeBudgetInstruction::SetComputeUnitLimit` (the client's `compute_limit`); the API uses 1.4M for every transaction. */
export declare function computeLimit(units?: number): TransactionInstruction;
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
export declare function closeArgs(claim: Pick<FuturesClaim, "slot" | "seq" | "side" | "lo" | "hi" | "lots" | "reserved">, k: IntegerLike, options?: {
    readonly lots?: IntegerLike;
    readonly limit?: IntegerLike;
    readonly minFill?: IntegerLike;
}): FuturesSubmitArgs;
/** One market's addresses and its instructions (mirrors the Rust client's `Market`). */
export declare class FuturesMarket implements MarketFields {
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
    private constructor();
    /** Addresses of `market` (its vault is the authority's token account; pass it in). */
    static derive(program: Address, market: Address, accounts: {
        readonly vault: Address;
        readonly period: Address;
        readonly mint: Address;
    }, product: string | Uint8Array, expiryTs: IntegerLike): FuturesMarket;
    /** From a decoded market account (`decodeHeader` / `FuturesBook.header`) and its address. */
    static fromHeader(program: Address, market: Address, header: Pick<FuturesHeader, "productId" | "expiryTs" | "vault" | "period" | "quoteMint">): FuturesMarket;
    /** The vault authority PDA of a market. */
    static authorityOf(program: Address, market: Address): PublicKey;
    private ix;
    private trading;
    private owned;
    /**
     * `CreateMarket`, once the market account exists (zeroed, owned by the
     * program, `marketSpace(params)` bytes) and the vault is the
     * authority's. `creator` is the program's upgrade authority (with the
     * council's authorization on a governed deployment).
     */
    create(creator: Address, oracleProgram: Address, params: CreateFuturesMarketParams): TransactionInstruction;
    /** Deposits `amount` atoms from `token` (the owner's USDC account) into the owner's free cash. */
    deposit(owner: Address, token: Address, amount: IntegerLike): TransactionInstruction;
    /** Pays free cash only; `amount` defaults to `u64::MAX`, all of it. */
    withdraw(owner: Address, token: Address, amount?: IntegerLike): TransactionInstruction;
    /**
     * `Submit`: trades one claim shape against the book with the verified
     * package `proposal`, then (`gtc`/`post`) rests the remainder. Send it with
     * `heapFrame()` and a 1.4M compute limit.
     */
    submit(owner: Address, args: FuturesSubmitArgs, proposal: Uint8Array): TransactionInstruction;
    /** Closes (part of) a claim: `submit` with `closeArgs(claim, k, options)`. */
    close(owner: Address, claim: Pick<FuturesClaim, "slot" | "seq" | "side" | "lo" | "hi" | "lots" | "reserved">, k: IntegerLike, proposal: Uint8Array, options?: {
        readonly lots?: IntegerLike;
        readonly limit?: IntegerLike;
        readonly minFill?: IntegerLike;
    }): TransactionInstruction;
    /** Cancels the owner's resting order `(slot, seq)`, releasing its escrow or claim reservation. */
    cancel(owner: Address, slot: number, seq: IntegerLike): TransactionInstruction;
    /** Publishes the owner's maker ladder (post-only: `proposal` carries the book's potentials). Send it with `heapFrame()`. */
    quote(owner: Address, ladder: FuturesLadderArgs, knots: readonly FuturesKnot[], proposal: Uint8Array): TransactionInstruction;
    /** A ladder whose steps outside `core` quote only their last (widest) tier. */
    quoteCore(owner: Address, ladder: FuturesLadderArgs, core: {
        readonly lo: number;
        readonly hi: number;
    }, knots: readonly FuturesKnot[], proposal: Uint8Array): TransactionInstruction;
    cancelQuote(owner: Address): TransactionInstruction;
    /** Splits a claim at `at` into `[lo, at)` and `[at, hi)`, same lots. */
    split(owner: Address, claim: FuturesRef, at: number): TransactionInstruction;
    /** Merges two of the owner's claims on one side: identical intervals, or adjacent ones with equal lots. */
    merge(owner: Address, keep: FuturesRef, absorb: FuturesRef): TransactionInstruction;
    /** Redeems `lots` of an exact complement pair (same interval) for its backing. */
    redeem(owner: Address, long: FuturesRef, short: FuturesRef, lots: IntegerLike): TransactionInstruction;
    /** Gives `lots` of a claim to `to` unchanged. */
    transfer(owner: Address, claim: FuturesRef, lots: IntegerLike, to: Address): TransactionInstruction;
    /** Permissionless: records the period's settlement. */
    finalize(): TransactionInstruction;
    /**
     * `CouncilSettle`: settles a market whose oracle period failed, or is unsettled at or after expiry, at `value`
     * (oracle units at the market's decimals). `authority` signs; on a governed program append the council config and
     * three seats (as for `create`).
     */
    councilSettle(authority: Address, value: IntegerLike): TransactionInstruction;
    /** Permissionless after finalization: pays claims (by slot) into free balances. */
    settle(claims: readonly number[]): TransactionInstruction;
    /** Permissionless after finalization: pays a maker's ladder inventory and releases its escrow. */
    settleMaker(maker: number): TransactionInstruction;
    /** Permissionless after finalization: releases resting orders' escrow. */
    prune(orders: readonly number[]): TransactionInstruction;
    /** The legacy Earn pool `operator` runs in this market (dev only). */
    pool(operator: Address): PublicKey;
    /** `owner`'s position account in a legacy `pool`. */
    lp(pool: Address, owner: Address): PublicKey;
    /**
     * `PoolMigrate` (dev builds): moves the legacy `pool`, whose shares `holder`
     * owns entirely, into the Earn vault. The pool's trader slot becomes the
     * vault's in this market and the holder gets vault shares; the holder and
     * the vault's operator sign, `payer` funds the holder's vault position the
     * first time. `markets`: the vault's, in its order (`FuturesVault.markets`).
     */
    poolMigrate(pool: Address, holder: Address, vault: FuturesEarnVault, payer: Address, markets: readonly Address[]): TransactionInstruction;
    /**
     * `PoolWithdraw` (legacy pools' exit, dev builds): redeems `shares`
     * (default `u64::MAX`, all) at the pool's lowest value, paid from its free cash.
     */
    poolWithdraw(pool: Address, owner: Address, token: Address, shares?: IntegerLike): TransactionInstruction;
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
export declare class FuturesEarnVault {
    readonly program: PublicKey;
    /** The vault's address, `["ameba-range-v1", "earn", operator]`. */
    readonly vault: PublicKey;
    readonly operator: PublicKey;
    /** Its idle cash: a token account of `mint` owned by `vault`. */
    readonly cash: PublicKey;
    /** The quote (USDC) mint. */
    readonly mint: PublicKey;
    private constructor();
    /** The vault `operator` runs, with `cash` as its token account (the client's `EarnVault::new`). */
    static derive(program: Address, operator: Address, accounts: {
        readonly cash: Address;
        readonly mint: Address;
    }): FuturesEarnVault;
    /** From the decoded vault account (`decodeVault`; the client's `EarnVault::of`). */
    static fromAccount(program: Address, vault: Pick<FuturesVault, "operator" | "cashAccount" | "quoteMint">): FuturesEarnVault;
    /** The address of the vault `operator` runs. */
    static address(program: Address, operator: Address): PublicKey;
    /** `owner`'s position account. */
    position(owner: Address): PublicKey;
    private ix;
    /**
     * `CreateVault`, once `cash` (an empty token account of `mint` owned by the
     * vault's address) exists. The operator signs; `payer` funds the rent.
     */
    create(payer: Address): TransactionInstruction;
    /**
     * `VaultDeposit`: `amount` atoms from `token` for shares priced at the
     * vault's highest value over every settlement outcome (1:1 into an empty
     * vault). `markets` are the vault's, in its order (`FuturesVault.markets`);
     * `payer` funds the position account the first time.
     */
    deposit(owner: Address, token: Address, amount: IntegerLike, payer: Address, markets: readonly Address[]): TransactionInstruction;
    /**
     * `VaultWithdraw`: redeems `shares` (`U64_MAX`: all) at the vault's lowest
     * value over every settlement outcome, paid to `token` from the idle cash,
     * then from the vault's free cash in `draws` (markets of `markets`, in that
     * order). `markets` are the vault's, in its order; those drawn from are
     * writable. The vault's last shares wait while it may still pay more
     * (`NotSettled`).
     */
    withdraw(owner: Address, token: Address, shares: IntegerLike, markets: readonly Address[], draws?: readonly FuturesVaultMarket[]): TransactionInstruction;
    /** `VaultFund`: the operator moves `amount` of idle cash into the vault's trader slot in `m` (adding `m` to its markets). */
    fund(m: FuturesVaultMarket, amount: IntegerLike): TransactionInstruction;
    /**
     * `VaultDefund`: the operator moves `amount` (default `u64::MAX`, all) of
     * the vault's free cash in `m` back to idle cash; a market where the vault
     * then holds nothing leaves its markets.
     */
    defund(m: FuturesVaultMarket, amount?: IntegerLike): TransactionInstruction;
}
/**
 * `ix` built for the Earn vault (or a legacy pool) as its owner (`submit`,
 * `cancel`, `quote`, `cancelQuote`, `split`, `merge` or `redeem`), signed by
 * its operator instead: the vault is no signer and the operator signs as the
 * last account (the client's `operated`). The market must be one of the
 * vault's. Withdrawals and transfers never act for the vault: its cash moves
 * only between the vault and its markets and leaves only to depositors.
 */
export declare function operated(ix: TransactionInstruction, operator: Address): TransactionInstruction;
export {};
//# sourceMappingURL=instructions.d.ts.map