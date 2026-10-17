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
import { PublicKey } from "@solana/web3.js";
import { ByteReader, ascii, toPublicKey } from "./codec.js";
import { productSymbol } from "./pda.js";
import { CLAIM_LEN, FUTURES_MAGIC, FUTURES_VERSION, HEADER_LEN, LP_LEN, LP_MAGIC, MAKER_HEAD, MAX_TIERS, ORDER_CLOSE, ORDER_LEN, POOL_LEN, POOL_MAGIC, REGISTRY_LEN, REGISTRY_MAGIC, STATUS_NAMES, STEP_LEN, TRADER_LEN, VAULT_HEAD, VAULT_LEN, VAULT_LP_LEN, VAULT_REWARDS_LEN, REWARDS_MAGIC, VAULT_LP_MAGIC, VAULT_MAGIC, VAULT_MARKETS, FuturesSide, FuturesStatus, longPerLot, makerLen, marketLen, shortPerLot, } from "./program.js";
/** A futures account that does not decode: wrong magic, version or length. */
export class FuturesAccountError extends Error {
    name = "FuturesAccountError";
}
const U64 = (1n << 64n) - 1n;
const DEFAULT_KEY = PublicKey.default;
// ----------------------------------------------------------------- Decoders
function expectMagic(r, magic, what) {
    if (!r.matches(0, ascii(magic)))
        throw new FuturesAccountError(`${what}: bad magic`);
}
/** Decodes the header and checks the magic, version and exact account length (`Header::read`). */
export function decodeHeader(data) {
    const r = new ByteReader(data);
    if (r.length < HEADER_LEN)
        throw new FuturesAccountError("market: too short");
    expectMagic(r, FUTURES_MAGIC, "market");
    if (r.u8(8) !== FUTURES_VERSION)
        throw new FuturesAccountError("market: unknown version");
    const productId = r.slice(32, 32);
    const status = r.u8(9);
    const h = {
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
export function decodeRegistry(data) {
    const r = new ByteReader(data);
    if (r.length !== REGISTRY_LEN)
        throw new FuturesAccountError("registry: bad length");
    expectMagic(r, REGISTRY_MAGIC, "registry");
    return { bump: r.u8(8), market: r.pubkey(16) };
}
/**
 * The Earn vault account (`Vault::read`): `VAULT_LEN` bytes, or `VAULT_REWARDS_LEN` with a reward stream; magic,
 * and at most `VAULT_MARKETS` markets.
 */
export function decodeVault(data) {
    const r = new ByteReader(data);
    if (r.length !== VAULT_LEN && r.length !== VAULT_REWARDS_LEN)
        throw new FuturesAccountError("vault: bad length");
    expectMagic(r, VAULT_MAGIC, "vault");
    const count = r.u8(9);
    if (count > VAULT_MARKETS)
        throw new FuturesAccountError("vault: too many markets");
    return {
        bump: r.u8(8), operator: r.pubkey(16), quoteMint: r.pubkey(48), cashAccount: r.pubkey(80), cash: r.u64(112),
        shares: r.u64(120), deposited: r.u64(128), withdrawn: r.u64(136), createdTs: r.u64(144),
        markets: Array.from({ length: count }, (_, i) => r.pubkey(VAULT_HEAD + 32 * i)),
        rewards: r.length === VAULT_REWARDS_LEN ? decodeRewards(r, VAULT_LEN) : null,
    };
}
function decodeRewards(r, at) {
    if (new TextDecoder().decode(r.slice(at, 8)) !== REWARDS_MAGIC)
        throw new FuturesAccountError("vault rewards: bad magic");
    return {
        decimals: r.u8(at + 8), mint: r.pubkey(at + 16), account: r.pubkey(at + 48), house: r.pubkey(at + 80),
        startTs: r.u64(at + 112), endTs: r.u64(at + 120), amount: r.u64(at + 128), lastTs: r.u64(at + 136),
        acc: r.u128(at + 144), houseShares: r.u64(at + 160), distributed: r.u64(at + 168), paid: r.u64(at + 176),
    };
}
/** An Earn vault position account (`VaultPosition::read`). */
export function decodeVaultPosition(data) {
    const r = new ByteReader(data);
    if (r.length !== VAULT_LP_LEN)
        throw new FuturesAccountError("vault position: bad length");
    expectMagic(r, VAULT_LP_MAGIC, "vault position");
    return {
        bump: r.u8(8), vault: r.pubkey(16), owner: r.pubkey(48), shares: r.u64(80), deposited: r.u64(88), withdrawn: r.u64(96),
        rewardSnapshot: r.u128(104), rewardPending: r.u64(120), rewardPaid: r.u64(8) >> 8n,
    };
}
/** A legacy Earn pool account (`Pool::read`). */
export function decodePool(data) {
    const r = new ByteReader(data);
    if (r.length !== POOL_LEN)
        throw new FuturesAccountError("pool: bad length");
    expectMagic(r, POOL_MAGIC, "pool");
    return {
        bump: r.u8(8), market: r.pubkey(16), operator: r.pubkey(48), shares: r.u64(80), deposited: r.u64(88),
        withdrawn: r.u64(96), createdTs: r.u64(104),
    };
}
/** A legacy pool position account (`LpPosition::read`). */
export function decodeLpPosition(data) {
    const r = new ByteReader(data);
    if (r.length !== LP_LEN)
        throw new FuturesAccountError("pool position: bad length");
    expectMagic(r, LP_MAGIC, "pool position");
    return {
        bump: r.u8(8), pool: r.pubkey(16), owner: r.pubkey(48), shares: r.u64(80), deposited: r.u64(88), withdrawn: r.u64(96),
    };
}
/** `getProgramAccounts` filter for the futures program's market accounts. */
export function marketFilters() {
    return [{ memcmp: { offset: 0, bytes: Buffer.from(ascii(FUTURES_MAGIC)).toString("base64"), encoding: "base64" } }];
}
// ----------------------------------------------------------------- Tiers (state.rs `Tiers`)
const tierCount = (t) => Math.max(t.count, 1);
/** Each tier's share of `total` lots, exact (the last takes the rest); `max(count, 1)` entries. */
export function tierSizes(t, total) {
    const n = tierCount(t);
    if (n > MAX_TIERS)
        throw new RangeError(`at most ${MAX_TIERS} tiers`);
    if (n === 1)
        return [total];
    const w = [100n];
    for (let i = 1; i < n; i += 1)
        w.push((w[i - 1] * BigInt(t.ratio)) / 100n);
    let sum = w.reduce((a, b) => a + b, 0n);
    if (sum < 1n)
        sum = 1n;
    const out = [];
    let used = 0n;
    for (let i = 0; i < n - 1; i += 1) {
        const size = BigInt.asUintN(64, (total * w[i]) / sum);
        out.push(size);
        used += size;
    }
    out.push(total - used);
    return out;
}
/** Each tier's remaining lots when `remaining` of `total` are left (the best tiers trade first). */
export function tierRemaining(t, total, remaining, sizes = tierSizes(t, total)) {
    const n = tierCount(t);
    if (n === 1)
        return [remaining];
    if (remaining >= total)
        return sizes.slice(0, n);
    let filled = total - remaining;
    return sizes.slice(0, n).map((size) => {
        const used = filled < size ? filled : size;
        filled -= used;
        return size - used;
    });
}
/** Tier `i`'s price from a step's best: bids step down (floor 0), asks up (capped at `k`). */
export function tierPrice(t, best, side, i, k) {
    const off = BigInt(t.spread) * BigInt(i);
    if (side === FuturesSide.Long)
        return BigInt(best) > off ? BigInt(best) - off : 0n;
    const ask = BigInt(best) + off;
    return ask < k ? ask : k;
}
// ----------------------------------------------------------------- The book
/** `ameba_futures_math::rank`: arrival sequence, then a sub-index. */
function rank(seq, sub) {
    return (seq << 32n) | BigInt(sub);
}
/** A ladder arc's sub-index: by step, bid before ask, then tier. */
function makerSub(step, side, tier) {
    return (step * 2 + side) * MAX_TIERS + tier;
}
/** A decoded market account (mirrors the Rust client's `Book`). Records are read on demand from `data`. */
export class FuturesBook {
    header;
    data;
    r;
    constructor(header, data) {
        this.header = header;
        this.data = data;
        this.r = new ByteReader(data);
    }
    static decode(data) {
        const copy = Uint8Array.from(data);
        return new FuturesBook(decodeHeader(copy), copy);
    }
    tradersAt() {
        return HEADER_LEN;
    }
    ordersAt() {
        return this.tradersAt() + this.header.traderCap * TRADER_LEN;
    }
    makersAt() {
        return this.ordersAt() + this.header.orderCap * ORDER_LEN;
    }
    claimsAt() {
        return this.makersAt() + this.header.makerCap * makerLen(this.header.grid);
    }
    check(index, cap, what) {
        if (!Number.isInteger(index) || index < 0 || index >= cap)
            throw new RangeError(`${what} ${index} is outside 0..${cap}`);
    }
    /** The trader record in `slot` (empty slots have the default owner). */
    trader(slot) {
        this.check(slot, this.header.traderCap, "trader slot");
        const at = this.tradersAt() + slot * TRADER_LEN, r = this.r;
        return {
            slot, owner: r.pubkey(at), free: r.u64(at + 32), reserved: r.u64(at + 40), claims: r.u32(at + 48),
            orders: r.u16(at + 52), maker: r.u8(at + 54), paidOut: r.u64(at + 56),
        };
    }
    order(slot) {
        this.check(slot, this.header.orderCap, "order slot");
        const at = this.ordersAt() + slot * ORDER_LEN, r = this.r;
        const flags = r.u8(at + 11);
        return {
            slot, seq: r.u64(at), trader: r.u16(at + 8), side: r.u8(at + 10), flags, close: (flags & ORDER_CLOSE) !== 0,
            lo: r.u16(at + 12), hi: r.u16(at + 14), lots: r.u64(at + 16), price: r.u64(at + 24), escrowPerLot: r.u64(at + 32),
            claim: r.u32(at + 40), placedLots: r.u64(at + 48), placedTs: r.u64(at + 56),
        };
    }
    claim(slot) {
        this.check(slot, this.header.claimCap, "claim slot");
        const at = this.claimsAt() + slot * CLAIM_LEN, r = this.r;
        return {
            slot, lots: r.u64(at), seq: r.u64(at + 8), trader: r.u16(at + 16), side: r.u8(at + 18), lo: r.u16(at + 20),
            hi: r.u16(at + 22), cost: r.u64(at + 24), reserved: r.u64(at + 32), openedTs: r.u64(at + 40),
        };
    }
    maker(slot) {
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
    step(slot, pos) {
        this.check(slot, this.header.makerCap, "maker slot");
        this.check(pos, this.header.grid, "step");
        const at = this.makersAt() + slot * makerLen(this.header.grid) + MAKER_HEAD + pos * STEP_LEN, r = this.r;
        return { pos, ask: r.u32(at), bid: r.u32(at + 4), askLots: r.u64(at + 8), bidLots: r.u64(at + 16), inv: r.i64(at + 24) };
    }
    /** Occupied trader slots. */
    traders() {
        const out = [];
        for (let i = 0; i < this.header.traderCap; i += 1) {
            const t = this.trader(i);
            if (!t.owner.equals(DEFAULT_KEY))
                out.push(t);
        }
        return out;
    }
    traderOf(owner) {
        const key = toPublicKey(owner);
        return this.traders().find((t) => t.owner.equals(key)) ?? null;
    }
    /** Orders with a sequence (a fully filled order keeps its slot until reused). */
    orders() {
        const out = [];
        for (let i = 0; i < this.header.orderCap; i += 1) {
            const o = this.order(i);
            if (o.seq !== 0n)
                out.push(o);
        }
        return out;
    }
    /** Ladders in use, each with every step that quotes lots or holds inventory. */
    makers() {
        const out = [];
        for (let m = 0; m < this.header.makerCap; m += 1) {
            const maker = this.maker(m);
            if (!maker.used)
                continue;
            const steps = [];
            for (let x = 0; x < this.header.grid; x += 1) {
                const st = this.step(m, x);
                if (st.inv !== 0n || st.bidLots > 0n || st.askLots > 0n)
                    steps.push(st);
            }
            out.push({ maker, steps });
        }
        return out;
    }
    /** Claims with lots. */
    claims() {
        const out = [];
        for (let i = 0; i < this.header.claimCap; i += 1) {
            const c = this.claim(i);
            if (c.lots > 0n)
                out.push(c);
        }
        return out;
    }
    claimsOf(trader) {
        return this.claims().filter((c) => c.trader === trader);
    }
    /**
     * Every live order and ladder step tier as package arcs, in canonical order
     * (orders by slot, then ladders by maker, step, bid before ask, best tier
     * first): exactly the list the program verifies a proposal against
     * (`state::book_arcs`).
     */
    arcs() {
        const h = this.header;
        const arcs = [];
        for (let slot = 0; slot < h.orderCap; slot += 1) {
            const o = this.order(slot);
            if (o.seq === 0n || o.lots === 0n)
                continue;
            arcs.push({
                id: { kind: "order", slot }, side: o.side === 0 ? FuturesSide.Long : FuturesSide.Short, lo: o.lo, hi: o.hi,
                price: o.price, cap: o.lots, rank: rank(o.seq, 0),
            });
        }
        for (let m = 0; m < h.makerCap; m += 1) {
            const mk = this.maker(m);
            if (!mk.used || mk.seq === 0n)
                continue;
            const n = tierCount(mk.tiers);
            const bidSizes = tierSizes(mk.tiers, mk.bidTotal), askSizes = tierSizes(mk.tiers, mk.askTotal);
            const seq = rank(mk.seq, 0);
            for (let x = mk.lo; x < mk.hi; x += 1) {
                const st = this.step(m, x);
                const sides = [
                    [FuturesSide.Long, tierRemaining(mk.tiers, mk.bidTotal, st.bidLots, bidSizes), st.bid],
                    [FuturesSide.Short, tierRemaining(mk.tiers, mk.askTotal, st.askLots, askSizes), st.ask],
                ];
                for (const [side, lots, best] of sides) {
                    for (let tier = 0; tier < n; tier += 1) {
                        const cap = lots[tier];
                        if (cap === 0n)
                            continue;
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
    traderValue(slot) {
        const h = this.header;
        const tr = this.trader(slot);
        const cash = tr.free + tr.reserved;
        const open = h.status === FuturesStatus.Open;
        const n = h.grid + 1;
        const delta = new Array(n + 1).fill(0n);
        let base = 0n, owed = 0n;
        const add = (long, lo, hi, lots) => {
            const sign = long ? 1n : -1n;
            if (!long)
                base += lots * BigInt(hi - lo);
            delta[lo] = delta[lo] + sign * lots;
            delta[hi] = delta[hi] - sign * lots;
            if (!open) {
                const per = long ? longPerLot(lo, hi, h.settlementFine, h.k) : shortPerLot(lo, hi, h.settlementFine, h.k);
                owed += lots * per;
            }
        };
        if (tr.maker > 0) {
            for (let x = 0; x < h.grid; x += 1) {
                const inv = this.step(tr.maker - 1, x).inv;
                if (inv !== 0n)
                    add(inv > 0n, x, x + 1, inv < 0n ? -inv : inv);
            }
        }
        for (let i = 0; i < h.claimCap; i += 1) {
            const c = this.claim(i);
            if (c.lots > 0n && c.trader === slot)
                add(c.side === FuturesSide.Long, c.lo, c.hi, c.lots);
        }
        const fits = (v) => v >= 0n && v <= U64;
        if (!open) {
            const v = cash + owed;
            return fits(v) ? [v, v] : null;
        }
        let lo = null, hi = null, d = 0n, slope = 0n;
        for (let x = 0; x < n; x += 1) {
            const v = base + d;
            if (lo === null || v < lo)
                lo = v;
            if (hi === null || v > hi)
                hi = v;
            slope += delta[x];
            d += slope;
        }
        const low = cash + lo * h.k, high = cash + hi * h.k;
        return fits(low) && fits(high) ? [low, high] : null;
    }
    /** `traderValue` of `owner`'s trader, `[0, 0]` when absent (`Book::value_of`). */
    valueOf(owner) {
        const t = this.traderOf(owner);
        return (t && this.traderValue(t.slot)) ?? [0n, 0n];
    }
}
/** Decodes a market account (alias of `FuturesBook.decode`). */
export function decodeMarket(data) {
    return FuturesBook.decode(data);
}
//# sourceMappingURL=accounts.js.map