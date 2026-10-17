/** Product ids and the program-derived addresses shared by futures markets and the oracle. */
import { PublicKey } from "@solana/web3.js";
import { type Address, type IntegerLike } from "./codec.js";
export type ProgramAddress = readonly [PublicKey, number];
/** Product id: the ASCII symbol zero-padded to 32 bytes (`"RAMX"`, `"NANDX"`). */
export declare function productId(symbol: string): Uint8Array;
/** Inverse of `productId`: the bytes before the first zero. */
export declare function productSymbol(id: Uint8Array): string;
/** Accepts a symbol or a 32-byte id. */
export declare function toProductId(product: string | Uint8Array): Uint8Array;
/** Oracle `Period` account for one product and expiry. */
export declare function findPeriodAddress(oracle: Address, product: string | Uint8Array, expiryTs: IntegerLike): ProgramAddress;
/** Upgradeable-loader ProgramData of a program (the governance gate binds it). */
export declare function findProgramDataAddress(program: Address): ProgramAddress;
//# sourceMappingURL=pda.d.ts.map