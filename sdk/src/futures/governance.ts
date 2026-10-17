/** Exact shared governance gate ABI; bind current authenticated epochs before signing. */
import { Buffer } from "buffer";
import { PublicKey, TransactionInstruction } from "@solana/web3.js";
import { toPublicKey, u64, type Address, type IntegerLike } from "./codec.js";
import { findProgramDataAddress } from "./pda.js";

export const PRODUCTION_CONTROLLER_PROGRAM_ID = new PublicKey("8fhNi6QHU5TYNhoPDM4vs89ZBztnpxp3LnBXRgkBVKtx");
/** Synthetic fixture identity; deployed localnet controllers use their trusted manifest ID. */
export const TEST_CONTROLLER_PROGRAM_ID = new PublicKey(new Uint8Array(32).fill(71));
/** @deprecated Fixture alias, not a deployed localnet identity. */
export const LOCALNET_CONTROLLER_PROGRAM_ID = TEST_CONTROLLER_PROGRAM_ID;
export const GOVERNANCE_TAIL_LEN = 16;
export const PROTOCOL_GATE_LEN = 192;

export function findGovernanceGateAddress(controller: Address, target: Address): [PublicKey, number] {
  return PublicKey.findProgramAddressSync([
    Buffer.from("ameba-governance-v3"), Buffer.from("gate"), toPublicKey(target).toBuffer(),
  ], toPublicKey(controller));
}

export function encodeGovernanceTail(epoch: IntegerLike): Buffer {
  const bytes = Buffer.alloc(GOVERNANCE_TAIL_LEN);
  bytes.write("AGV1", 0, "ascii");
  bytes[4] = 1;
  bytes.writeBigUInt64LE(u64(epoch, "epoch"), 8);
  return bytes;
}

/**
 * Splits a bound instruction's data into its payload and the governance tail
 * `encodeGovernanceTail` appended (`"AGV1"`, version 1, the signed epoch).
 */
export function decodeGovernanceTail(data: Uint8Array): { readonly payload: Uint8Array; readonly epoch: bigint } {
  if (data.length <= GOVERNANCE_TAIL_LEN) throw new Error("missing governance tail");
  const tail = Buffer.from(data.subarray(data.length - GOVERNANCE_TAIL_LEN));
  if (!tail.subarray(0, 4).equals(Buffer.from("AGV1")) || tail[4] !== 1 || tail.subarray(5, 8).some((v) => v !== 0)) {
    throw new Error("invalid governance tail");
  }
  return { payload: Uint8Array.from(data.subarray(0, data.length - GOVERNANCE_TAIL_LEN)), epoch: tail.readBigUInt64LE(8) };
}

/** Authenticated active gate state, tied to a canonical target and controller. */
export class GovernanceGateBinding {
  private constructor(
    readonly controller: PublicKey,
    readonly target: PublicKey,
    readonly address: PublicKey,
    readonly epoch: bigint,
  ) {}

  static fromAccount(controller: Address, target: Address, address: Address, owner: Address, data: Uint8Array): GovernanceGateBinding {
    const c = toPublicKey(controller), t = toPublicKey(target), a = toPublicKey(address);
    const [expected, bump] = findGovernanceGateAddress(c, t);
    const bytes = Buffer.from(data);
    const config = PublicKey.findProgramAddressSync([
      Buffer.from("ameba-governance-v3"), Buffer.from("council"),
    ], c)[0];
    if (!a.equals(expected) || !toPublicKey(owner).equals(c) || bytes.length !== PROTOCOL_GATE_LEN
      || !bytes.subarray(0, 8).equals(Buffer.from("AGVGAT01")) || bytes[8] !== 1
      || bytes[9] !== bump || bytes[10] !== 1 || bytes[190] !== 0 || bytes[191] !== 0
      || !new PublicKey(bytes.subarray(12, 44)).equals(config)
      || !new PublicKey(bytes.subarray(44, 76)).equals(t)
      || !new PublicKey(bytes.subarray(76, 108)).equals(findProgramDataAddress(t)[0])) {
      throw new Error("invalid governance gate account");
    }
    // Active status requires no proposal or freeze fields.
    if (bytes[11] !== 0 || bytes.subarray(116, 158).some(v => v !== 0)) {
      throw new Error("governance gate is frozen or malformed");
    }
    const epoch = bytes.readBigUInt64LE(108);
    if (epoch === 0n) throw new Error("governance gate epoch must be nonzero");
    return new GovernanceGateBinding(c, t, a, epoch);
  }

  bind(ix: TransactionInstruction): TransactionInstruction {
    if (!ix.programId.equals(this.target) || ix.keys.some(k => k.pubkey.equals(this.address))) {
      throw new Error("invalid governance instruction target or duplicate gate");
    }
    return new TransactionInstruction({ programId: ix.programId,
      keys: [...ix.keys, { pubkey: this.address, isSigner: false, isWritable: false }],
      data: Buffer.concat([ix.data, encodeGovernanceTail(this.epoch)]),
    });
  }
}
