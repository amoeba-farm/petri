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
import { type IntegerLike } from "./codec.js";
import type { FuturesArc, FuturesArcId } from "./accounts.js";
export declare class FuturesProposalError extends Error {
    readonly name = "FuturesProposalError";
}
export interface FuturesProposalFill {
    readonly id: FuturesArcId;
    readonly lots: bigint;
}
export interface FuturesProposal {
    /** Sorted by arc id (orders by slot, then maker, step, side, tier), no duplicates, no zero lots. */
    readonly fills: readonly FuturesProposalFill[];
    readonly takerFill: bigint;
    /** One per canonical node (i64). */
    readonly potentials: readonly bigint[];
}
/** Canonical arc order (`ArcId`'s derived `Ord`): orders by slot first, then ladder arcs by maker, step, side, tier. */
export declare function compareArcIds(a: FuturesArcId, b: FuturesArcId): number;
/** `Proposal::encode`. */
export declare function encodeProposal(proposal: FuturesProposal): Uint8Array;
/** `Proposal::decode`: throws `FuturesProposalError` on anything the program would reject as undecodable. */
export declare function decodeProposal(bytes: Uint8Array): FuturesProposal;
/** `Proposal::from_fills`: a solved package over `arcs` (fills parallel to them) as a proposal. */
export declare function proposalFromFills(arcs: readonly FuturesArc[], fills: readonly IntegerLike[], takerFill: IntegerLike, potentials: readonly IntegerLike[]): FuturesProposal;
/**
 * `Proposal::dense`: the proposal's fills parallel to `arcs` (the book's
 * canonical arc list, sorted by id). Throws when a fill names an arc the
 * book does not have.
 */
export declare function proposalDense(proposal: FuturesProposal, arcs: readonly FuturesArc[]): bigint[];
/**
 * Premium the taker pays (long, positive) or receives (short) for fills
 * parallel to `arcs`: what resting shorts receive minus what resting longs
 * pay, signed for the taker's side (`taker_premium`).
 */
export declare function takerPremium(takerSide: number, arcs: readonly FuturesArc[], fills: readonly bigint[]): bigint;
//# sourceMappingURL=proposal.d.ts.map