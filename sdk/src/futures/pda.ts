/** Product ids and the program-derived addresses shared by futures markets and the oracle. */
import { PublicKey } from "@solana/web3.js";
import { ascii, toPublicKey, u64, type Address, type IntegerLike } from "./codec.js";
import { BPF_LOADER_UPGRADEABLE_PROGRAM_ID, PERIOD_NAMESPACE, PERIOD_SEED } from "./constants.js";

export type ProgramAddress = readonly [PublicKey, number];

/** Product id: the ASCII symbol zero-padded to 32 bytes (`"RAMX"`, `"NANDX"`). */
export function productId(symbol: string): Uint8Array {
  const bytes = ascii(symbol);
  if (bytes.length > 32) throw new RangeError("product symbol exceeds 32 bytes");
  const id = new Uint8Array(32);
  id.set(bytes);
  return id;
}

/** Inverse of `productId`: the bytes before the first zero. */
export function productSymbol(id: Uint8Array): string {
  if (id.length !== 32) throw new RangeError("product id must be 32 bytes");
  const end = id.indexOf(0);
  return String.fromCharCode(...id.subarray(0, end < 0 ? 32 : end));
}

/** Accepts a symbol or a 32-byte id. */
export function toProductId(product: string | Uint8Array): Uint8Array {
  if (typeof product === "string") return productId(product);
  if (product.length !== 32) throw new RangeError("product id must be 32 bytes");
  return Uint8Array.from(product);
}

/** `expiry_ts` little-endian. */
function le64(value: IntegerLike): Uint8Array {
  const out = new Uint8Array(8);
  new DataView(out.buffer).setBigUint64(0, u64(value, "expiry_ts"), true);
  return out;
}

/** Oracle `Period` account for one product and expiry. */
export function findPeriodAddress(oracle: Address, product: string | Uint8Array, expiryTs: IntegerLike): ProgramAddress {
  return PublicKey.findProgramAddressSync(
    [ascii(PERIOD_NAMESPACE), ascii(PERIOD_SEED), toProductId(product), le64(expiryTs)],
    toPublicKey(oracle),
  );
}

/** Upgradeable-loader ProgramData of a program (the governance gate binds it). */
export function findProgramDataAddress(program: Address): ProgramAddress {
  return PublicKey.findProgramAddressSync([toPublicKey(program).toBytes()], BPF_LOADER_UPGRADEABLE_PROGRAM_ID);
}
