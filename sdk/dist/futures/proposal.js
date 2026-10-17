/**
 * The compact proposal a futures `Submit` or `Quote` carries
 * (`ameba_futures_math::codec::Proposal`): the lots filled on each arc of the
 * book, the taker's fill, and one node potential per canonical node (the
 * certificate the program verifies the package with).
 *
 * The SDK does not solve packages: the futures API's `/v1/quote` and
 * `/v1/tx/prepare` do, as does the Rust client's `Book::propose`. This
 * codec reads and writes the bytes exactly, so a prepared transaction's
 * proposal can be checked against the book (`proposalDense`) before
 * signing.
 *
 * Wire format: LEB128 varints. Order fills `(slot delta, lots)`; ladder fills
 * as runs `(maker u8, side + 2·tier u8, step, run length, lots)` of
 * consecutive steps with equal lots; then the taker's fill; then the
 * potentials as zigzag-coded deltas.
 */
import { U64_MAX, u64 } from "./codec.js";
import { MAX_TIERS } from "./program.js";
export class FuturesProposalError extends Error {
    name = "FuturesProposalError";
}
const I64_MIN = -(1n << 63n), I64_MAX = (1n << 63n) - 1n;
const MAX_POTENTIALS = 4096n;
/** Canonical arc order (`ArcId`'s derived `Ord`): orders by slot first, then ladder arcs by maker, step, side, tier. */
export function compareArcIds(a, b) {
    if (a.kind !== b.kind)
        return a.kind === "order" ? -1 : 1;
    if (a.kind === "order" && b.kind === "order")
        return a.slot - b.slot;
    const x = a, y = b;
    return x.maker - y.maker || x.step - y.step || x.side - y.side || x.tier - y.tier;
}
function put(out, value) {
    let v = value;
    for (;;) {
        const byte = Number(v & 0x7fn);
        v >>= 7n;
        if (v === 0n) {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}
const zigzag = (v) => BigInt.asUintN(64, (v << 1n) ^ (v >> 63n));
const unzigzag = (v) => BigInt.asIntN(64, (v >> 1n) ^ -(v & 1n));
function i64(value, name) {
    const v = typeof value === "bigint" ? value : BigInt(value);
    if (v < I64_MIN || v > I64_MAX)
        throw new RangeError(`${name} must be an i64`);
    return v;
}
/** `Proposal::encode`. */
export function encodeProposal(proposal) {
    const out = [];
    const orders = [];
    const steps = [];
    for (const f of proposal.fills) {
        const lots = u64(f.lots, "lots");
        if (lots === 0n)
            throw new RangeError("a fill must have lots");
        if (f.id.kind === "order")
            orders.push([f.id.slot, lots]);
        else {
            if (!Number.isInteger(f.id.tier) || f.id.tier < 0 || f.id.tier >= MAX_TIERS)
                throw new RangeError("tier out of range");
            steps.push([f.id.maker, f.id.side + 2 * f.id.tier, f.id.step, lots]);
        }
    }
    orders.sort((a, b) => a[0] - b[0]);
    put(out, BigInt(orders.length));
    let last = 0n;
    orders.forEach(([slot, lots], i) => {
        if (!Number.isInteger(slot) || slot < 0 || slot >= 0xffff)
            throw new RangeError("order slot out of range");
        if (i > 0 && BigInt(slot) === last)
            throw new RangeError("duplicate order fill");
        put(out, BigInt(slot) - last);
        last = BigInt(slot);
        put(out, lots);
    });
    steps.sort((a, b) => a[0] - b[0] || a[1] - b[1] || a[2] - b[2] || (a[3] < b[3] ? -1 : a[3] > b[3] ? 1 : 0));
    const runs = [];
    for (const [maker, sideByte, step, lots] of steps) {
        const r = runs[runs.length - 1];
        if (r && r[0] === maker && r[1] === sideByte && r[2] + r[3] === step && r[4] === lots)
            r[3] += 1;
        else
            runs.push([maker, sideByte, step, 1, lots]);
    }
    put(out, BigInt(runs.length));
    for (const [maker, sideByte, step, len, lots] of runs) {
        out.push(maker & 0xff, sideByte & 0xff);
        put(out, BigInt(step));
        put(out, BigInt(len));
        put(out, lots);
    }
    put(out, u64(proposal.takerFill, "takerFill"));
    put(out, BigInt(proposal.potentials.length));
    let prev = 0n;
    for (const p of proposal.potentials) {
        const v = i64(p, "potential");
        put(out, zigzag(BigInt.asIntN(64, v - prev)));
        prev = v;
    }
    return Uint8Array.from(out);
}
/** `Proposal::decode`: throws `FuturesProposalError` on anything the program would reject as undecodable. */
export function decodeProposal(bytes) {
    let at = 0;
    const byte = () => {
        if (at >= bytes.length)
            throw new FuturesProposalError("proposal is truncated");
        return bytes[at++];
    };
    const varint = () => {
        let v = 0n;
        for (let shift = 0n; shift < 64n; shift += 7n) {
            const b = byte();
            v |= (BigInt(b & 0x7f) << shift) & U64_MAX;
            if ((b & 0x80) === 0)
                return v;
        }
        throw new FuturesProposalError("varint is too long");
    };
    const fills = [];
    const n = varint();
    let slot = 0n;
    for (let i = 0n; i < n; i += 1n) {
        const d = varint();
        if (i > 0n && d === 0n)
            throw new FuturesProposalError("order fills must have increasing slots");
        slot += d;
        if (slot > U64_MAX)
            throw new FuturesProposalError("order slot overflows");
        const lots = varint();
        if (slot >= 0xffffn)
            throw new FuturesProposalError("order slot out of range");
        if (lots === 0n)
            throw new FuturesProposalError("zero-lot fill");
        fills.push({ id: { kind: "order", slot: Number(slot) }, lots });
    }
    const runs = varint();
    for (let r = 0n; r < runs; r += 1n) {
        const maker = byte();
        const sideByte = byte();
        const side = sideByte & 1, tier = sideByte >> 1;
        if (tier >= MAX_TIERS)
            throw new FuturesProposalError("tier out of range");
        const step = varint(), len = varint();
        if (step > 0xffffn || len > 0xffffn)
            throw new FuturesProposalError("ladder run out of range");
        const lots = varint();
        if (lots === 0n || len === 0n)
            throw new FuturesProposalError("empty ladder run");
        for (let s = 0n; s < len; s += 1n) {
            const at = step + s;
            if (at > 0xffffn)
                throw new FuturesProposalError("ladder run overflows");
            fills.push({ id: { kind: "maker", maker, step: Number(at), side, tier }, lots });
        }
    }
    fills.sort((a, b) => compareArcIds(a.id, b.id));
    for (let i = 1; i < fills.length; i += 1) {
        if (compareArcIds(fills[i - 1].id, fills[i].id) === 0)
            throw new FuturesProposalError("duplicate fill");
    }
    const takerFill = varint();
    const count = varint();
    if (count > MAX_POTENTIALS)
        throw new FuturesProposalError("too many potentials");
    const potentials = [];
    let prev = 0n;
    for (let i = 0n; i < count; i += 1n) {
        prev = BigInt.asIntN(64, prev + unzigzag(varint()));
        potentials.push(prev);
    }
    if (at !== bytes.length)
        throw new FuturesProposalError("proposal has trailing bytes");
    return { fills, takerFill, potentials };
}
/** `Proposal::from_fills`: a solved package over `arcs` (fills parallel to them) as a proposal. */
export function proposalFromFills(arcs, fills, takerFill, potentials) {
    if (fills.length !== arcs.length)
        throw new RangeError("fills must be parallel to arcs");
    const out = [];
    arcs.forEach((a, i) => {
        const lots = u64(fills[i], "fill");
        if (lots > 0n)
            out.push({ id: a.id, lots });
    });
    out.sort((a, b) => compareArcIds(a.id, b.id));
    return { fills: out, takerFill: u64(takerFill, "takerFill"), potentials: potentials.map((p) => i64(p, "potential")) };
}
/**
 * `Proposal::dense`: the proposal's fills parallel to `arcs` (the book's
 * canonical arc list, sorted by id). Throws when a fill names an arc the
 * book does not have.
 */
export function proposalDense(proposal, arcs) {
    const out = new Array(arcs.length).fill(0n);
    let i = 0;
    for (const f of proposal.fills) {
        while (i < arcs.length && compareArcIds(arcs[i].id, f.id) < 0)
            i += 1;
        if (i === arcs.length || compareArcIds(arcs[i].id, f.id) !== 0) {
            throw new FuturesProposalError("proposal fills an arc that is not in the book");
        }
        out[i] = f.lots;
        i += 1;
    }
    return out;
}
/**
 * Premium the taker pays (long, positive) or receives (short) for fills
 * parallel to `arcs`: what resting shorts receive minus what resting longs
 * pay, signed for the taker's side (`taker_premium`).
 */
export function takerPremium(takerSide, arcs, fills) {
    let net = 0n;
    arcs.forEach((a, i) => {
        const m = fills[i] ?? 0n;
        net += (a.side === 0 ? -a.price : a.price) * m;
    });
    return takerSide === 0 ? net : -net;
}
//# sourceMappingURL=proposal.js.map