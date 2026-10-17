/** Exact shared governance gate ABI; bind current authenticated epochs before signing. */
import { Buffer } from "buffer";
import { PublicKey, TransactionInstruction } from "@solana/web3.js";
import { type Address, type IntegerLike } from "./codec.js";
export declare const PRODUCTION_CONTROLLER_PROGRAM_ID: PublicKey;
/** Synthetic fixture identity; deployed localnet controllers use their trusted manifest ID. */
export declare const TEST_CONTROLLER_PROGRAM_ID: PublicKey;
/** @deprecated Fixture alias, not a deployed localnet identity. */
export declare const LOCALNET_CONTROLLER_PROGRAM_ID: PublicKey;
export declare const GOVERNANCE_TAIL_LEN = 16;
export declare const PROTOCOL_GATE_LEN = 192;
export declare function findGovernanceGateAddress(controller: Address, target: Address): [PublicKey, number];
export declare function encodeGovernanceTail(epoch: IntegerLike): Buffer;
/**
 * Splits a bound instruction's data into its payload and the governance tail
 * `encodeGovernanceTail` appended (`"AGV1"`, version 1, the signed epoch).
 */
export declare function decodeGovernanceTail(data: Uint8Array): {
    readonly payload: Uint8Array;
    readonly epoch: bigint;
};
/** Authenticated active gate state, tied to a canonical target and controller. */
export declare class GovernanceGateBinding {
    readonly controller: PublicKey;
    readonly target: PublicKey;
    readonly address: PublicKey;
    readonly epoch: bigint;
    private constructor();
    static fromAccount(controller: Address, target: Address, address: Address, owner: Address, data: Uint8Array): GovernanceGateBinding;
    bind(ix: TransactionInstruction): TransactionInstruction;
}
//# sourceMappingURL=governance.d.ts.map