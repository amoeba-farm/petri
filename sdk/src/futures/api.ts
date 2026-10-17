/**
 * `FuturesClient`: a thin typed client for the futures API's market,
 * account, quote and Earn routes (`api/API.md`, "Futures") and its sponsored
 * transaction flow:
 * prepare, sign as the owner, submit, wait. The API solves every package
 * (quotes and `Submit` proposals) on its cached book with the Rust client's
 * exact solver and pays the fee; the owner only signs.
 *
 * ```ts
 * const api = new FuturesClient({ baseUrl: "https://api.amoeba.farm" });
 * const quote = await api.quote({ market: "RAMX", owner, side: "long", lo: 86, hi: 104, lots: 3 });
 * const prepared = await api.prepare(owner, "submit", { market: "RAMX", side: "long", lo: 86, hi: 104, lots: 3 });
 * const { signature } = await api.submit(prepared, signPrepared(prepared, keypair));
 * await api.wait(signature, { commitment: "confirmed" });
 * ```
 *
 * Amounts are strings of integer atoms, `*Value` fields decimal strings of
 * index values, positions/lots/slots/timestamps JSON numbers, sequences
 * decimal strings (API conventions). Requests accept `bigint` for any integer.
 */
import { Buffer } from "buffer";
import { Transaction, type PublicKey, type Signer } from "@solana/web3.js";
import { toPublicKey, type Address } from "./codec.js";
import { FuturesMode, FuturesSide, type FuturesModeLike, type FuturesSideLike } from "./program.js";
import { decodeGovernanceTail } from "./governance.js";
import { decodeFuturesInstruction, type DecodedFuturesInstruction } from "./instructions.js";

// ----------------------------------------------------------------- Shared
/** Integer atoms (or another u64) as a decimal string; may start with `-` where signed. */
export type Atoms = string;
/** A decimal string such as `"100"` or `"102.9155"`. */
export type DecimalString = string;
/** Integers in requests: numbers, bigints or decimal strings. */
export type IntegerInput = number | bigint | string;
export type FuturesSideName = "long" | "short";
export type FuturesModeName = "ioc" | "gtc" | "post";

export class FuturesApiError extends Error {
  override readonly name = "FuturesApiError";
  /** HTTP status (400 bad input, 403 refused, 404 unknown, 409 maker not indexed yet, 410 prepared id gone, 422 preflight, 429, 500, 502, 503). */
  readonly status: number;
  /** The API's `error`, e.g. `"MinFill: …"` or `"InsufficientFunds: …"`. */
  readonly error: string;
  readonly logs: readonly string[] | null;
  /** Seconds from `Retry-After` on a 429. */
  readonly retryAfter: number | null;

  constructor(status: number, error: string, logs: readonly string[] | null = null, retryAfter: number | null = null) {
    super(`${status}: ${error}`);
    this.status = status;
    this.error = error;
    this.logs = logs;
    this.retryAfter = retryAfter;
  }
}

// ----------------------------------------------------------------- Views
export interface FuturesIndexReading {
  readonly value: DecimalString;
  readonly ts: number;
}

export interface FuturesPeriodState {
  readonly status: string;
  readonly effectiveListingTs: number | null;
  readonly expiryTs: number;
  readonly valueDecimals: number;
  readonly settlementValue: DecimalString | null;
}

/** The ticket's default range: `index * (1 ∓ 2σ√T)` in whole grid positions. */
export interface FuturesSuggestion {
  readonly sigmaValue: DecimalString;
  readonly lowValue: DecimalString;
  readonly highValue: DecimalString;
  readonly lo: number;
  readonly hi: number;
  readonly centerValue: DecimalString;
  readonly vol: DecimalString;
  /** `book`: the spread the maker ladders price (the normal case); else realized index volatility, or the dev fixture. */
  readonly source: "book" | "oracle-index" | "dev-fixture";
}

/** The deployment's treasury trader in a market. */
export interface FuturesMakerSummary {
  readonly owner: string;
  readonly slot: number;
  readonly free: Atoms;
  readonly reserved: Atoms;
  /** Ladder slot, or null. */
  readonly maker: number | null;
  readonly lo: number | null;
  readonly hi: number | null;
  readonly escrow: Atoms;
  readonly bidLots: number;
  readonly askLots: number;
  readonly inventoryLots: number;
  readonly payoutMin: Atoms;
  readonly payoutMax: Atoms;
}

/**
 * A market's legacy Earn pool (dev only: the Earn vault replaced the
 * per-market pools; holders leave with `poolWithdraw`); values are atoms over
 * every settlement outcome.
 */
export interface FuturesPoolSummary {
  readonly address: string;
  readonly operator: string;
  readonly shares: Atoms;
  readonly valueLow: Atoms;
  readonly valueHigh: Atoms;
  readonly free: Atoms;
  readonly reserved: Atoms;
  readonly deposited: Atoms;
  readonly withdrawn: Atoms;
}

export interface MarketView {
  readonly market: string;
  readonly symbol: string;
  /** 64 hex characters. */
  readonly productId: string;
  readonly expiryTs: number;
  readonly label: string;
  readonly status: "open" | "settled" | "failed";
  /** Open and the period tradable at the chain clock (the program's rule); never for a retired market. */
  readonly tradable: boolean;
  /** No longer in the deployment manifest: open to withdrawals and settlement only. */
  readonly retired: boolean;
  readonly period: string;
  readonly periodState: FuturesPeriodState | null;
  readonly vault: string;
  readonly grid: number;
  /** Atoms per lot per grid step. */
  readonly k: Atoms;
  /** Index points per grid step. */
  readonly stepValue: DecimalString;
  readonly valueDecimals: number;
  readonly lotUsdPerPoint: DecimalString;
  /** The failed-period settlement. */
  readonly referenceValue: DecimalString;
  readonly index: FuturesIndexReading | null;
  readonly lastEntryValue: DecimalString | null;
  readonly lastTradeTs: number | null;
  readonly settlementValue: DecimalString | null;
  readonly trades: number;
  readonly volumeLots: number;
  readonly volumePremium: Atoms;
  readonly backing: Atoms;
  readonly custody: Atoms;
  readonly claimsOpen: number;
  readonly volume24hLots: number;
  readonly change24hValue: DecimalString | null;
  readonly suggestion: FuturesSuggestion;
  readonly maker: FuturesMakerSummary | null;
  /** The market's legacy Earn pool (dev only), null when there is none. The Earn vault is `FuturesClient.earn()`. */
  readonly pool: FuturesPoolSummary | null;
  readonly slot: number;
  readonly chainTime: number;
}

export interface OrderView {
  readonly slot: number;
  readonly seq: string;
  /** Null when the trader slot is not found. */
  readonly owner: string | null;
  readonly side: FuturesSideName;
  readonly lo: number;
  readonly hi: number;
  readonly loValue: DecimalString;
  readonly hiValue: DecimalString;
  readonly lots: number;
  readonly placedLots: number;
  /** Premium atoms per lot; a resting order trades at exactly this. */
  readonly price: Atoms;
  readonly entryValue: DecimalString;
  readonly escrow: Atoms;
  readonly close: boolean;
  /** The claim a close order names; `seq` is null once that claim is gone. */
  readonly claim: { readonly slot: number; readonly seq: string | null } | null;
  readonly placedTs: number;
}

export interface LadderStepView {
  readonly pos: number;
  readonly bid: Atoms | null;
  readonly ask: Atoms | null;
  readonly bidValue: DecimalString | null;
  readonly askValue: DecimalString | null;
  readonly bidLots: number;
  readonly askLots: number;
  /** Signed lots: + long, - short. */
  readonly inventory: number;
}

export interface LadderView {
  readonly maker: number;
  readonly owner: string | null;
  readonly seq: string;
  readonly live: boolean;
  readonly lo: number;
  readonly hi: number;
  readonly loValue: DecimalString;
  readonly hiValue: DecimalString;
  readonly escrow: Atoms;
  readonly quotedTs: number;
  readonly tiers: { readonly count: number; readonly ratio: number; readonly spread: number };
  readonly steps: readonly LadderStepView[];
}

export type FuturesSourceId =
  | { readonly kind: "order"; readonly slot: number; readonly seq: string }
  | { readonly kind: "maker"; readonly maker: number; readonly step: number; readonly tier: number; readonly seq: string };

/** One live arc of the solver's source list (`Book::arcs()`). */
export interface SourceView {
  readonly source: FuturesSourceId;
  readonly owner: string | null;
  readonly side: FuturesSideName;
  readonly lo: number;
  readonly hi: number;
  readonly loValue: DecimalString;
  readonly hiValue: DecimalString;
  readonly lots: number;
  readonly price: Atoms;
  readonly entryValue: DecimalString;
}

export interface BookView {
  readonly market: string;
  readonly slot: number;
  readonly orders: readonly OrderView[];
  readonly ladders: readonly LadderView[];
  readonly sources: readonly SourceView[];
}

export interface FuturesDepthLevel {
  /** Signed premium atoms per lot. */
  readonly premium: Atoms;
  readonly entryValue: DecimalString;
  readonly lots: number;
  readonly cumulativeLots: number;
}

/** Executable depth of one claim shape: `asks` buy `[lo, hi)` (entries ascending), `bids` sell it. */
export interface DepthView {
  readonly lo: number;
  readonly hi: number;
  readonly loValue: DecimalString;
  readonly hiValue: DecimalString;
  readonly asks: readonly FuturesDepthLevel[];
  readonly bids: readonly FuturesDepthLevel[];
}

export interface TradeView {
  readonly signature: string;
  readonly eventIndex: number;
  readonly slot: number;
  readonly ts: number;
  /** The taker's: buy = long. */
  readonly side: "buy" | "sell";
  readonly lo: number;
  readonly hi: number;
  readonly loValue: DecimalString;
  readonly hiValue: DecimalString;
  readonly lots: number;
  /** Signed total. */
  readonly premium: Atoms;
  readonly entryValue: DecimalString;
  readonly sources: "orders" | "ladder" | "mixed";
  readonly taker: string;
}

export interface FuturesCandle {
  readonly t: number;
  readonly o: number;
  readonly h: number;
  readonly l: number;
  readonly c: number;
  readonly v: number;
}

export interface FuturesCandles {
  readonly resolution: number;
  readonly candles: readonly FuturesCandle[];
}

export interface ClaimView {
  readonly slot: number;
  readonly seq: string;
  readonly side: FuturesSideName;
  readonly lo: number;
  readonly hi: number;
  readonly loValue: DecimalString;
  readonly hiValue: DecimalString;
  readonly lots: number;
  readonly cost: Atoms;
  readonly reserved: number;
  readonly entryValue: DecimalString;
  readonly openedTs: number;
  readonly maxPayout: Atoms;
  readonly payoutAtIndex: Atoms | null;
  readonly pnlAtIndex: Atoms | null;
  /** The executable sale of the claim now: `lots` fill for `premium` in total; `net` is what free cash gains. */
  readonly exit: { readonly lots: number; readonly premium: Atoms; readonly net: Atoms; readonly entryValue: DecimalString } | null;
}

export interface FuturesAccountMarket {
  readonly market: string;
  readonly symbol: string;
  readonly label: string;
  readonly expiryTs: number;
  readonly status: "open" | "settled" | "failed";
  readonly tradable: boolean;
  /** No longer in the deployment manifest: open to withdrawals and settlement only. */
  readonly retired: boolean;
  readonly markValue: DecimalString | null;
  readonly trader: { readonly slot: number; readonly free: Atoms; readonly reserved: Atoms; readonly paidOut: Atoms } | null;
  readonly orders: readonly OrderView[];
  readonly claims: readonly ClaimView[];
  readonly ladder: LadderView | null;
  /**
   * The owner's position in the market's legacy Earn pool (dev only;
   * `valueLow` is what withdrawing every share pays now). The Earn vault
   * position is `FuturesClient.earnAccount(owner)`.
   */
  readonly lp: {
    readonly pool: string; readonly shares: Atoms; readonly valueLow: Atoms; readonly valueHigh: Atoms;
    readonly deposited: Atoms; readonly withdrawn: Atoms;
  } | null;
}

export interface AccountView {
  readonly owner: string;
  readonly usdc: Atoms;
  readonly usdcAta: string;
  readonly usdcAtaExists: boolean;
  readonly markets: readonly FuturesAccountMarket[];
}

// ----------------------------------------------------------------- Earn
/** One market the Earn vault is in: its trader slot there, valued over every settlement outcome (atoms). */
export interface EarnVaultMarket {
  readonly market: string;
  readonly symbol: string;
  readonly label: string;
  readonly expiryTs: number;
  readonly status: "open" | "settled" | "failed";
  readonly tradable: boolean;
  /** The vault trader's free cash there (withdrawals may draw on it) and what its quotes and orders hold. */
  readonly free: Atoms;
  readonly reserved: Atoms;
  readonly valueLow: Atoms;
  readonly valueHigh: Atoms;
}

/**
 * The Earn vault: one for the whole product. Its value is its idle cash plus
 * its trader's value in each of its markets, over every settlement outcome:
 * deposits buy shares at `valueHigh`, withdrawals pay at `valueLow`.
 */
export interface EarnVault {
  readonly address: string;
  readonly operator: string;
  /** The vault's USDC token account. */
  readonly cashAccount: string;
  /** Shares outstanding. */
  readonly shares: Atoms;
  /** Idle cash, not in any market. */
  readonly cash: Atoms;
  readonly deposited: Atoms;
  readonly withdrawn: Atoms;
  readonly valueLow: Atoms;
  readonly valueHigh: Atoms;
  /** Every market the vault is in, in its order. */
  readonly markets: readonly EarnVaultMarket[];
}

/** `GET /v1/earn`. */
export interface EarnView {
  /** Null while the deployment has no Earn vault. */
  readonly vault: EarnVault | null;
  readonly chainTime: number;
}

/** An owner's Earn vault shares, valued now. */
export interface EarnPosition {
  /** The position account (`findVaultPositionAddress`); `exists` is false before the first deposit. */
  readonly address: string;
  readonly exists: boolean;
  readonly shares: Atoms;
  /** What the shares are worth at the vault's lowest value (what withdrawing them all pays) and its highest. */
  readonly valueLow: Atoms;
  readonly valueHigh: Atoms;
  readonly deposited: Atoms;
  readonly withdrawn: Atoms;
  /** The value less deposits plus withdrawals, at each end; may be negative. */
  readonly earningsLow: Atoms;
  readonly earningsHigh: Atoms;
  /** The shares' fraction of all shares, a decimal string (`"0.25"`). */
  readonly share: DecimalString;
  /** Atoms an `earnWithdraw` can pay now, from the vault's cash on hand. */
  readonly withdrawable: Atoms;
}

/** `GET /v1/accounts/:owner/earn`. */
export interface EarnAccountView {
  readonly owner: string;
  /** The owner's wallet USDC. */
  readonly usdc: Atoms;
  readonly vault: EarnVault | null;
  readonly position: EarnPosition;
  readonly chainTime: number;
}

/** One Earn deposit, withdrawal or (dev) migration of a legacy pool position into the vault. */
export interface EarnActivity {
  readonly signature: string;
  readonly slot: number;
  readonly ts: number;
  readonly kind: "deposit" | "withdraw" | "migrate";
  readonly amount: Atoms;
  readonly shares: Atoms;
}

// ----------------------------------------------------------------- Quotes
export interface FuturesClaimRef {
  readonly slot: number;
  readonly seq: IntegerInput;
}

export interface FuturesQuoteRequest {
  /** Market address or symbol. */
  readonly market: string;
  readonly owner?: Address;
  readonly side: FuturesSideLike;
  readonly lo: number;
  readonly hi: number;
  readonly lots: IntegerInput;
  /** Premium atoms per lot; default the most permissive (`set` for a long, 0 for a short). */
  readonly limit?: IntegerInput;
  readonly mode?: FuturesModeLike;
  readonly minFill?: IntegerInput;
  /** A claim of `owner` the order closes. */
  readonly close?: FuturesClaimRef;
}

export interface FuturesQuote {
  readonly market: string;
  readonly side: FuturesSideName;
  readonly lo: number;
  readonly hi: number;
  readonly loValue: DecimalString;
  readonly hiValue: DecimalString;
  readonly lots: number;
  /** The limit used. */
  readonly limit: Atoms;
  readonly mode: FuturesModeName;
  readonly filled: number;
  /** Signed total. */
  readonly premium: Atoms;
  readonly averageEntryValue: DecimalString | null;
  /** Signed atoms the taker puts in (long: premium; short: set × filled − premium). */
  readonly pay: Atoms;
  readonly maxPayout: Atoms;
  readonly legs: { readonly orders: number; readonly ladderSteps: number };
  /** With `owner`: free cash, USDC, sets the fill releases, net pay, escrow of a resting remainder, deposit needed. */
  readonly free?: Atoms;
  readonly usdc?: Atoms;
  readonly redeemed?: Atoms;
  readonly netPay?: Atoms;
  readonly escrow?: Atoms;
  readonly deposit?: Atoms;
  readonly slot: number;
  /** `NotTradable`, `NoLiquidity`, `WouldCross`, `MinFill`, `InsufficientFunds`, `ClaimNotFound`, `NotComplement`, `ClaimReserved`. */
  readonly error?: string;
}

// ----------------------------------------------------------------- Transactions
/** `params` of each action of `POST /v1/tx/prepare`. */
export interface FuturesActionParams {
  readonly deposit: { readonly market: string; readonly amount: IntegerInput };
  readonly withdraw: { readonly market: string; readonly amount: IntegerInput | "all" };
  readonly submit: {
    readonly market: string; readonly side: FuturesSideLike; readonly lo: number; readonly hi: number; readonly lots: IntegerInput;
    readonly limit?: IntegerInput; readonly minFill?: IntegerInput; readonly mode?: FuturesModeLike; readonly close?: FuturesClaimRef;
    readonly withdraw?: boolean;
  };
  readonly close: {
    readonly market: string; readonly slot: number; readonly seq: IntegerInput; readonly lots?: IntegerInput;
    readonly limit?: IntegerInput; readonly minFill?: IntegerInput; readonly withdraw?: boolean;
  };
  readonly cancel: { readonly market: string; readonly slot: number; readonly seq: IntegerInput; readonly withdraw?: boolean };
  /** Deposits `amount` USDC atoms from the owner's wallet into the Earn vault for shares (no `market`). */
  readonly earnDeposit: { readonly amount: IntegerInput };
  /** Withdraws `amount` USDC atoms (or `"all"` that can be paid now) from the Earn vault to the owner's wallet (no `market`). */
  readonly earnWithdraw: { readonly amount: IntegerInput | "all" };
  /** Legacy pools' exit (dev only): redeems a market's pool shares for cash to the owner's USDC account. */
  readonly poolWithdraw: { readonly market: string; readonly shares: IntegerInput | "all" };
  /** Finalizes the market against its settled period (anyone may). */
  readonly finalize: { readonly market: string };
  /** Settles the owner's claims, orders and ladder (finalizing first if needed), then withdraws all free cash unless `withdraw: false`. */
  readonly settle: { readonly market: string; readonly withdraw?: boolean };
  /** Sends `amount` USDC atoms from the owner's wallet to `destination`'s (no `market`; the sponsor pays any ATA rent). */
  readonly cashOut: { readonly destination: Address; readonly amount: Atoms | bigint };
}
export type FuturesAction = keyof FuturesActionParams;

/** An unsigned sponsored transaction: fee payer the sponsor, every signature zeroed. */
export interface PreparedTransaction {
  readonly id: string;
  /** Base64 bincode legacy `Transaction`. */
  readonly transaction: string;
  readonly blockhash: string;
  readonly lastValidBlockHeight: number;
  /**
   * `{ action, market, symbol, label, owner, feePayer, instructions: [step names], expiresAt, …action fields }`
   * (`cashOut`, `earnDeposit` and `earnWithdraw`: no market fields).
   */
  readonly summary: Readonly<Record<string, unknown>> & { readonly action: string; readonly instructions: readonly string[] };
  /**
   * Transactions that must land first, in order, when the action spends compressed USDC in transactions of its own
   * (API.md, "Spending compressed USDC"): each moves the owner's compressed USDC into a classic USDC account
   * (`summary.action` `"decompress"`). `send` signs and submits them for you.
   */
  readonly before?: readonly PreparedTransaction[];
}

/** `GET /v1/wallets/:owner/usdc`: the owner's classic USDC token account. */
export interface WalletUsdc {
  readonly owner: string;
  readonly ata: string;
  readonly usdcAtoms: string;
  readonly decimals: number;
  readonly slot: number;
}

/**
 * `GET /v1/wallets/:owner/compressed-usdc`: the owner's compressed USDC, their Light token ATA plus their compressed
 * token accounts (`compressedAtoms` is null when those could not be read). Orders and cash-outs spend it.
 */
export interface WalletCompressedUsdc {
  readonly owner: string;
  readonly lightAta: string;
  readonly usdcAtoms: string;
  readonly lightAtaAtoms: string;
  readonly compressedAtoms: string | null;
  readonly decimals: number;
  readonly slot: number;
}

export interface FuturesQuotePrepared {
  readonly quote: FuturesQuote;
  readonly prepared: PreparedTransaction | null;
}

export interface SubmittedTransaction {
  readonly signature: string;
  readonly lastValidBlockHeight: number;
}

export type TxCommitment = "processed" | "confirmed";

export interface TxWaitResult {
  readonly signature: string;
  readonly status: "pending" | "processed" | "confirmed" | "failed" | "expired";
  readonly slot: number | null;
  readonly processedMs: number | null;
  readonly confirmedMs: number | null;
  readonly err?: unknown;
  /** Decoded error name, e.g. `"MinFill"`. */
  readonly error?: string;
  /** The failing step (e.g. `"submit"`), null when unknown; set with `error`. */
  readonly failedInstruction?: string | null;
  /** At confirmed: decoded events (`take`, `fill`, …), each with `type` and `market`. */
  readonly events?: readonly Readonly<Record<string, unknown>>[];
  readonly logs?: readonly string[];
  readonly timedOut: boolean;
  readonly waitedMs: number;
}

export interface TxStatusResult {
  readonly signature: string;
  readonly status: "pending" | "confirmed" | "finalized" | "failed";
  readonly slot?: number | null;
  /** The raw transaction error (tracked transactions). */
  readonly err?: unknown;
  readonly error?: string;
  /** The failing step, null when unknown (tracked transactions, with `error`). */
  readonly failedInstruction?: string | null;
  readonly blockTime?: number | null;
  readonly events?: readonly Readonly<Record<string, unknown>>[];
  readonly logs?: readonly string[];
  /** Tracked transactions: `pending`, `processed`, `confirmed`, `failed` or `expired`. */
  readonly commitment?: "pending" | "processed" | "confirmed" | "failed" | "expired";
  readonly processedMs?: number | null;
  readonly confirmedMs?: number | null;
}

/** The deployment, as `GET /v1/config` describes it (the fields the futures flow uses). */
export interface FuturesApiConfig {
  readonly cluster: string;
  readonly genesisHash: string | null;
  readonly programs: { readonly oracle: string; readonly futures: string | null };
  readonly treasury: string | null;
  readonly governanceController: { readonly program: string; readonly targets: readonly string[] } | null;
  readonly sponsor: string;
  readonly usdcMint: string;
  readonly usdcDecimals: number;
  readonly computeUnitLimit: number;
  readonly preparedTtlSecs: number;
  /** The deployment's public RPC endpoints, null when the manifest names none. */
  readonly rpcUrl: string | null;
  readonly wsUrl: string | null;
  readonly [key: string]: unknown;
}

// ----------------------------------------------------------------- Signing
/** The prepared transaction, ready to sign. Its message bytes are kept exactly (the API checks them at submit). */
export function preparedTransaction(prepared: Pick<PreparedTransaction, "transaction">): Transaction {
  return Transaction.from(Buffer.from(prepared.transaction, "base64"));
}

/**
 * Signs a prepared transaction as the owner with a keypair and returns it
 * for `FuturesClient.submit`. With a wallet, call its `signTransaction` on
 * `preparedTransaction(prepared)` instead. Review it first
 * (`reviewPrepared`): the API prepares, the owner decides.
 */
export function signPrepared(prepared: Pick<PreparedTransaction, "transaction">, signer: Signer): Transaction {
  const tx = preparedTransaction(prepared);
  tx.partialSign(signer);
  return tx;
}

/** One instruction of a prepared transaction: the futures program's decoded, others by program id. */
export type ReviewedInstruction =
  | { readonly program: "futures"; readonly instruction: DecodedFuturesInstruction; readonly epoch: bigint | null }
  | { readonly program: "other"; readonly programId: PublicKey };

/**
 * Decodes every instruction of a prepared transaction for review before
 * signing: futures instructions (with their governance tail, when bound)
 * against `program` (`config().programs.futures`); anything else by its
 * program id (compute budget, token account creation, …).
 */
export function reviewPrepared(tx: Transaction, program: Address): ReviewedInstruction[] {
  const futures = toPublicKey(program);
  return tx.instructions.map((ix): ReviewedInstruction => {
    if (!ix.programId.equals(futures)) return { program: "other", programId: ix.programId };
    try {
      const { payload, epoch } = decodeGovernanceTail(ix.data);
      return { program: "futures", instruction: decodeFuturesInstruction(payload), epoch };
    } catch {
      // An unbound instruction (an ungoverned deployment); throws if it is not a futures instruction either.
      return { program: "futures", instruction: decodeFuturesInstruction(ix.data), epoch: null };
    }
  });
}

// ----------------------------------------------------------------- Client
export interface FuturesClientOptions {
  /** API origin, e.g. `https://api.amoeba.farm` (production) or `http://127.0.0.1:4500` (a dev API), or a site's same-origin proxy `https://<site>/api/futures`. */
  readonly baseUrl: string;
  /** Defaults to the global `fetch`. */
  readonly fetch?: typeof fetch;
  readonly headers?: Readonly<Record<string, string>>;
}

const SIDE_NAMES: Record<FuturesSide, FuturesSideName> = { [FuturesSide.Long]: "long", [FuturesSide.Short]: "short" };
const MODE_NAMES: Record<FuturesMode, FuturesModeName> = { [FuturesMode.Ioc]: "ioc", [FuturesMode.Gtc]: "gtc", [FuturesMode.Post]: "post" };

/** Request bodies: bigints as decimal strings, addresses as base58, sides and modes as the API's names. */
function wire(value: unknown, key = ""): unknown {
  if (typeof value === "bigint") return value.toString();
  if (value && typeof value === "object" && typeof (value as PublicKey).toBase58 === "function") return (value as PublicKey).toBase58();
  if (key === "side" && typeof value === "number") return SIDE_NAMES[value as FuturesSide] ?? value;
  if (key === "mode" && typeof value === "number") return MODE_NAMES[value as FuturesMode] ?? value;
  if (Array.isArray(value)) return value.map((v) => wire(v));
  if (value && typeof value === "object") {
    return Object.fromEntries(Object.entries(value).filter(([, v]) => v !== undefined).map(([k, v]) => [k, wire(v, k)]));
  }
  return value;
}

function query(params: Readonly<Record<string, IntegerInput | undefined>>): string {
  const entries = Object.entries(params).filter((e): e is [string, IntegerInput] => e[1] !== undefined);
  return entries.length ? `?${new URLSearchParams(entries.map(([k, v]) => [k, String(v)])).toString()}` : "";
}

export class FuturesClient {
  readonly baseUrl: string;
  private readonly fetchImpl: typeof fetch;
  private readonly headers: Readonly<Record<string, string>>;

  constructor(options: FuturesClientOptions) {
    this.baseUrl = options.baseUrl.replace(/\/+$/u, "");
    const f = options.fetch ?? (globalThis.fetch as typeof fetch | undefined);
    if (!f) throw new Error("no fetch available: pass options.fetch");
    this.fetchImpl = f;
    this.headers = options.headers ?? {};
  }

  private async request<T>(method: "GET" | "POST", path: string, body?: unknown): Promise<T> {
    const init: RequestInit = { method, headers: { accept: "application/json", ...this.headers } };
    if (body !== undefined) {
      init.headers = { ...init.headers, "content-type": "application/json" };
      init.body = JSON.stringify(wire(body));
    }
    const res = await this.fetchImpl(`${this.baseUrl}${path}`, init);
    const text = await res.text();
    let json: unknown = null;
    try {
      json = text ? JSON.parse(text) : null;
    } catch {
      json = null;
    }
    if (!res.ok) {
      const e = (json && typeof json === "object" ? json : {}) as { error?: unknown; logs?: unknown };
      const retry = res.headers.get("retry-after");
      throw new FuturesApiError(
        res.status,
        typeof e.error === "string" ? e.error : text || res.statusText,
        Array.isArray(e.logs) ? e.logs.filter((l): l is string => typeof l === "string") : null,
        retry !== null && Number.isFinite(Number(retry)) ? Number(retry) : null,
      );
    }
    return json as T;
  }

  private market_(market: string): string {
    return encodeURIComponent(market);
  }

  /** `GET /v1/config`: program ids (`programs.futures`), treasury, sponsor, USDC mint, governance controller. */
  config(): Promise<FuturesApiConfig> {
    return this.request("GET", "/v1/config");
  }

  /** `GET /v1/markets`. */
  async markets(): Promise<MarketView[]> {
    return (await this.request<{ markets: MarketView[] }>("GET", "/v1/markets")).markets;
  }

  /** `GET /v1/markets/:market` (address, or a symbol: its open market with the nearest expiry). */
  market(market: string): Promise<MarketView> {
    return this.request("GET", `/v1/markets/${this.market_(market)}`);
  }

  /** `GET /v1/markets/:market/book`: resting orders, maker ladders and the solver's complete source list. */
  book(market: string): Promise<BookView> {
    return this.request("GET", `/v1/markets/${this.market_(market)}/book`);
  }

  /** `GET /v1/markets/:market/depth`: executable depth of `[lo, hi)` (default: the market's suggestion). */
  depth(market: string, options: { readonly lo?: number; readonly hi?: number; readonly max?: IntegerInput } = {}): Promise<DepthView> {
    return this.request("GET", `/v1/markets/${this.market_(market)}/depth${query(options)}`);
  }

  /** `GET /v1/markets/:market/trades`, newest first. */
  async trades(market: string, options: { readonly limit?: number; readonly before?: string } = {}): Promise<TradeView[]> {
    return (await this.request<{ trades: TradeView[] }>("GET",
      `/v1/markets/${this.market_(market)}/trades${query(options)}`)).trades;
  }

  /** `GET /v1/markets/:market/candles` over price-line trades. */
  candles(market: string, options: { readonly resolution?: number; readonly from?: number; readonly to?: number } = {}): Promise<FuturesCandles> {
    return this.request("GET", `/v1/markets/${this.market_(market)}/candles${query(options)}`);
  }

  /** `GET /v1/accounts/:owner`: cash, orders, claims (with their executable exit) and ladder per market. */
  account(owner: Address): Promise<AccountView> {
    return this.request("GET", `/v1/accounts/${toPublicKey(owner).toBase58()}`);
  }

  /** `GET /v1/earn`: the Earn vault (null while there is none) with its value and markets. */
  earn(): Promise<EarnView> {
    return this.request("GET", "/v1/earn");
  }

  /** `GET /v1/accounts/:owner/earn`: the vault, the owner's shares valued now and what can be withdrawn now. */
  earnAccount(owner: Address): Promise<EarnAccountView> {
    return this.request("GET", `/v1/accounts/${toPublicKey(owner).toBase58()}/earn`);
  }

  /** `GET /v1/accounts/:owner/earn/activity`: the owner's Earn deposits and withdrawals, newest first (`before`: a signature). */
  async earnActivity(owner: Address, options: { readonly limit?: number; readonly before?: string } = {}): Promise<EarnActivity[]> {
    return (await this.request<{ activity: EarnActivity[] }>("GET",
      `/v1/accounts/${toPublicKey(owner).toBase58()}/earn/activity${query(options)}`)).activity;
  }

  /** `POST /v1/quote`: the package an order would fill now (nothing signed). */
  quote(request: FuturesQuoteRequest): Promise<FuturesQuote> {
    return this.request("POST", "/v1/quote", request);
  }

  /** `POST /v1/tx/quote-prepare` with an order: its quote and the `submit` for it together. */
  quotePrepare(request: FuturesQuoteRequest & {
    readonly owner: Address; readonly withdraw?: boolean;
  }): Promise<FuturesQuotePrepared> {
    return this.request("POST", "/v1/tx/quote-prepare", request);
  }

  /** `GET /v1/wallets/:owner/usdc`. */
  walletUsdc(owner: Address): Promise<WalletUsdc> {
    return this.request("GET", `/v1/wallets/${toPublicKey(owner).toBase58()}/usdc`);
  }

  /** `GET /v1/wallets/:owner/compressed-usdc`. */
  walletCompressedUsdc(owner: Address): Promise<WalletCompressedUsdc> {
    return this.request("GET", `/v1/wallets/${toPublicKey(owner).toBase58()}/compressed-usdc`);
  }

  /** `POST /v1/tx/prepare`: a sponsored transaction for one action, for the owner to sign. */
  prepare<A extends FuturesAction>(owner: Address, action: A, params: FuturesActionParams[A]): Promise<PreparedTransaction> {
    return this.request("POST", "/v1/tx/prepare", { owner: toPublicKey(owner), action, params });
  }

  /** `POST /v1/tx/submit`: the owner-signed transaction of a prepared id; the API co-signs as the sponsor and sends it. */
  submit(prepared: Pick<PreparedTransaction, "id">, signed: Transaction | Uint8Array | string): Promise<SubmittedTransaction> {
    // Duck-typed: a wallet may hand back a Transaction of another web3.js copy.
    const transaction = typeof signed === "string" ? signed
      : Buffer.from(signed instanceof Uint8Array ? signed
        : signed.serialize({ requireAllSignatures: false, verifySignatures: false })).toString("base64");
    return this.request("POST", "/v1/tx/submit", { id: prepared.id, transaction });
  }

  /** `GET /v1/tx/:signature/wait`: resolves at `commitment`, on failure or expiry, or after `timeoutMs`. */
  wait(signature: string, options: { readonly commitment?: TxCommitment; readonly timeoutMs?: number } = {}): Promise<TxWaitResult> {
    return this.request("GET", `/v1/tx/${encodeURIComponent(signature)}/wait${query(options)}`);
  }

  /** `GET /v1/tx/:signature`. */
  transaction(signature: string): Promise<TxStatusResult> {
    return this.request("GET", `/v1/tx/${encodeURIComponent(signature)}`);
  }

  /**
   * Prepare, sign and submit one action. `sign` gets each prepared
   * transaction (review it there) and returns it signed by the owner: a
   * wallet's `signTransaction`, or `(tx) => { tx.partialSign(keypair); return tx; }`.
   * When the action spends compressed USDC in transactions of their own
   * (`prepared.before`), every one is signed first, then each is submitted
   * and waited for (`processed`) before the next, and the main one last;
   * their signatures come back as `before`.
   */
  async send<A extends FuturesAction>(owner: Address, action: A, params: FuturesActionParams[A],
    sign: (tx: Transaction, prepared: PreparedTransaction) => Transaction | Promise<Transaction>,
  ): Promise<SubmittedTransaction & { readonly prepared: PreparedTransaction; readonly before: readonly string[] }> {
    const prepared = await this.prepare(owner, action, params);
    const first = prepared.before ?? [];
    const signedFirst: Transaction[] = [];
    for (const part of first) signedFirst.push(await sign(preparedTransaction(part), part));
    const signed = await sign(preparedTransaction(prepared), prepared);
    const before: string[] = [];
    for (const [i, part] of first.entries()) {
      const { signature } = await this.submit(part, signedFirst[i]!);
      const landed = await this.wait(signature, { commitment: "processed" });
      if (landed.status !== "processed" && landed.status !== "confirmed") {
        throw new Error(`moving compressed USDC (${signature}) did not land: ${landed.status}`);
      }
      before.push(signature);
    }
    return { ...(await this.submit(prepared, signed)), prepared, before };
  }
}
