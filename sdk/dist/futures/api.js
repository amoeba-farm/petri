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
import { Transaction } from "@solana/web3.js";
import { toPublicKey } from "./codec.js";
import { FuturesMode, FuturesSide } from "./program.js";
import { decodeGovernanceTail } from "./governance.js";
import { decodeFuturesInstruction } from "./instructions.js";
export class FuturesApiError extends Error {
    name = "FuturesApiError";
    /** HTTP status (400 bad input, 403 refused, 404 unknown, 409 maker not indexed yet, 410 prepared id gone, 422 preflight, 429, 500, 502, 503). */
    status;
    /** The API's `error`, e.g. `"MinFill: …"` or `"InsufficientFunds: …"`. */
    error;
    logs;
    /** Seconds from `Retry-After` on a 429. */
    retryAfter;
    constructor(status, error, logs = null, retryAfter = null) {
        super(`${status}: ${error}`);
        this.status = status;
        this.error = error;
        this.logs = logs;
        this.retryAfter = retryAfter;
    }
}
// ----------------------------------------------------------------- Signing
/** The prepared transaction, ready to sign. Its message bytes are kept exactly (the API checks them at submit). */
export function preparedTransaction(prepared) {
    return Transaction.from(Buffer.from(prepared.transaction, "base64"));
}
/**
 * Signs a prepared transaction as the owner with a keypair and returns it
 * for `FuturesClient.submit`. With a wallet, call its `signTransaction` on
 * `preparedTransaction(prepared)` instead. Review it first
 * (`reviewPrepared`): the API prepares, the owner decides.
 */
export function signPrepared(prepared, signer) {
    const tx = preparedTransaction(prepared);
    tx.partialSign(signer);
    return tx;
}
/**
 * Decodes every instruction of a prepared transaction for review before
 * signing: futures instructions (with their governance tail, when bound)
 * against `program` (`config().programs.futures`); anything else by its
 * program id (compute budget, token account creation, …).
 */
export function reviewPrepared(tx, program) {
    const futures = toPublicKey(program);
    return tx.instructions.map((ix) => {
        if (!ix.programId.equals(futures))
            return { program: "other", programId: ix.programId };
        try {
            const { payload, epoch } = decodeGovernanceTail(ix.data);
            return { program: "futures", instruction: decodeFuturesInstruction(payload), epoch };
        }
        catch {
            // An unbound instruction (an ungoverned deployment); throws if it is not a futures instruction either.
            return { program: "futures", instruction: decodeFuturesInstruction(ix.data), epoch: null };
        }
    });
}
const SIDE_NAMES = { [FuturesSide.Long]: "long", [FuturesSide.Short]: "short" };
const MODE_NAMES = { [FuturesMode.Ioc]: "ioc", [FuturesMode.Gtc]: "gtc", [FuturesMode.Post]: "post" };
/** Request bodies: bigints as decimal strings, addresses as base58, sides and modes as the API's names. */
function wire(value, key = "") {
    if (typeof value === "bigint")
        return value.toString();
    if (value && typeof value === "object" && typeof value.toBase58 === "function")
        return value.toBase58();
    if (key === "side" && typeof value === "number")
        return SIDE_NAMES[value] ?? value;
    if (key === "mode" && typeof value === "number")
        return MODE_NAMES[value] ?? value;
    if (Array.isArray(value))
        return value.map((v) => wire(v));
    if (value && typeof value === "object") {
        return Object.fromEntries(Object.entries(value).filter(([, v]) => v !== undefined).map(([k, v]) => [k, wire(v, k)]));
    }
    return value;
}
function query(params) {
    const entries = Object.entries(params).filter((e) => e[1] !== undefined);
    return entries.length ? `?${new URLSearchParams(entries.map(([k, v]) => [k, String(v)])).toString()}` : "";
}
export class FuturesClient {
    baseUrl;
    fetchImpl;
    headers;
    constructor(options) {
        this.baseUrl = options.baseUrl.replace(/\/+$/u, "");
        const f = options.fetch ?? globalThis.fetch;
        if (!f)
            throw new Error("no fetch available: pass options.fetch");
        this.fetchImpl = f;
        this.headers = options.headers ?? {};
    }
    async request(method, path, body) {
        const init = { method, headers: { accept: "application/json", ...this.headers } };
        if (body !== undefined) {
            init.headers = { ...init.headers, "content-type": "application/json" };
            init.body = JSON.stringify(wire(body));
        }
        const res = await this.fetchImpl(`${this.baseUrl}${path}`, init);
        const text = await res.text();
        let json = null;
        try {
            json = text ? JSON.parse(text) : null;
        }
        catch {
            json = null;
        }
        if (!res.ok) {
            const e = (json && typeof json === "object" ? json : {});
            const retry = res.headers.get("retry-after");
            throw new FuturesApiError(res.status, typeof e.error === "string" ? e.error : text || res.statusText, Array.isArray(e.logs) ? e.logs.filter((l) => typeof l === "string") : null, retry !== null && Number.isFinite(Number(retry)) ? Number(retry) : null);
        }
        return json;
    }
    market_(market) {
        return encodeURIComponent(market);
    }
    /** `GET /v1/config`: program ids (`programs.futures`), treasury, sponsor, USDC mint, governance controller. */
    config() {
        return this.request("GET", "/v1/config");
    }
    /** `GET /v1/markets`. */
    async markets() {
        return (await this.request("GET", "/v1/markets")).markets;
    }
    /** `GET /v1/markets/:market` (address, or a symbol: its open market with the nearest expiry). */
    market(market) {
        return this.request("GET", `/v1/markets/${this.market_(market)}`);
    }
    /** `GET /v1/markets/:market/book`: resting orders, maker ladders and the solver's complete source list. */
    book(market) {
        return this.request("GET", `/v1/markets/${this.market_(market)}/book`);
    }
    /** `GET /v1/markets/:market/depth`: executable depth of `[lo, hi)` (default: the market's suggestion). */
    depth(market, options = {}) {
        return this.request("GET", `/v1/markets/${this.market_(market)}/depth${query(options)}`);
    }
    /** `GET /v1/markets/:market/trades`, newest first. */
    async trades(market, options = {}) {
        return (await this.request("GET", `/v1/markets/${this.market_(market)}/trades${query(options)}`)).trades;
    }
    /** `GET /v1/markets/:market/candles` over price-line trades. */
    candles(market, options = {}) {
        return this.request("GET", `/v1/markets/${this.market_(market)}/candles${query(options)}`);
    }
    /** `GET /v1/accounts/:owner`: cash, orders, claims (with their executable exit) and ladder per market. */
    account(owner) {
        return this.request("GET", `/v1/accounts/${toPublicKey(owner).toBase58()}`);
    }
    /** `GET /v1/earn`: the Earn vault (null while there is none) with its value and markets. */
    earn() {
        return this.request("GET", "/v1/earn");
    }
    /** `GET /v1/accounts/:owner/earn`: the vault, the owner's shares valued now and what can be withdrawn now. */
    earnAccount(owner) {
        return this.request("GET", `/v1/accounts/${toPublicKey(owner).toBase58()}/earn`);
    }
    /** `GET /v1/accounts/:owner/earn/activity`: the owner's Earn deposits and withdrawals, newest first (`before`: a signature). */
    async earnActivity(owner, options = {}) {
        return (await this.request("GET", `/v1/accounts/${toPublicKey(owner).toBase58()}/earn/activity${query(options)}`)).activity;
    }
    /** `POST /v1/quote`: the package an order would fill now (nothing signed). */
    quote(request) {
        return this.request("POST", "/v1/quote", request);
    }
    /** `POST /v1/tx/quote-prepare` with an order: its quote and the `submit` for it together. */
    quotePrepare(request) {
        return this.request("POST", "/v1/tx/quote-prepare", request);
    }
    /** `GET /v1/wallets/:owner/usdc`. */
    walletUsdc(owner) {
        return this.request("GET", `/v1/wallets/${toPublicKey(owner).toBase58()}/usdc`);
    }
    /** `GET /v1/wallets/:owner/compressed-usdc`. */
    walletCompressedUsdc(owner) {
        return this.request("GET", `/v1/wallets/${toPublicKey(owner).toBase58()}/compressed-usdc`);
    }
    /** `POST /v1/tx/prepare`: a sponsored transaction for one action, for the owner to sign. */
    prepare(owner, action, params) {
        return this.request("POST", "/v1/tx/prepare", { owner: toPublicKey(owner), action, params });
    }
    /** `POST /v1/tx/submit`: the owner-signed transaction of a prepared id; the API co-signs as the sponsor and sends it. */
    submit(prepared, signed) {
        // Duck-typed: a wallet may hand back a Transaction of another web3.js copy.
        const transaction = typeof signed === "string" ? signed
            : Buffer.from(signed instanceof Uint8Array ? signed
                : signed.serialize({ requireAllSignatures: false, verifySignatures: false })).toString("base64");
        return this.request("POST", "/v1/tx/submit", { id: prepared.id, transaction });
    }
    /** `GET /v1/tx/:signature/wait`: resolves at `commitment`, on failure or expiry, or after `timeoutMs`. */
    wait(signature, options = {}) {
        return this.request("GET", `/v1/tx/${encodeURIComponent(signature)}/wait${query(options)}`);
    }
    /** `GET /v1/tx/:signature`. */
    transaction(signature) {
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
    async send(owner, action, params, sign) {
        const prepared = await this.prepare(owner, action, params);
        const first = prepared.before ?? [];
        const signedFirst = [];
        for (const part of first)
            signedFirst.push(await sign(preparedTransaction(part), part));
        const signed = await sign(preparedTransaction(prepared), prepared);
        const before = [];
        for (const [i, part] of first.entries()) {
            const { signature } = await this.submit(part, signedFirst[i]);
            const landed = await this.wait(signature, { commitment: "processed" });
            if (landed.status !== "processed" && landed.status !== "confirmed") {
                throw new Error(`moving compressed USDC (${signature}) did not land: ${landed.status}`);
            }
            before.push(signature);
        }
        return { ...(await this.submit(prepared, signed)), prepared, before };
    }
}
//# sourceMappingURL=api.js.map