/**
 * Little-endian byte helpers for the futures module. Self-contained: the
 * futures subpath has no other runtime dependencies in this package.
 */
import { Buffer } from "buffer";
import { PublicKey } from "@solana/web3.js";
/** Anything a `PublicKey` can be built from without ambiguity. */
export type Address = PublicKey | string;
export declare function toPublicKey(value: Address): PublicKey;
export declare const U8_MAX = 255;
export declare const U16_MAX = 65535;
export declare const U32_MAX = 4294967295;
export declare const U64_MAX: bigint;
export declare const I64_MIN: bigint;
export declare const I64_MAX: bigint;
export declare const U128_MAX: bigint;
export type IntegerLike = bigint | number;
export declare function u8(value: number, name?: string): number;
export declare function u16(value: number, name?: string): number;
export declare function u32(value: number, name?: string): number;
export declare function u64(value: IntegerLike, name?: string): bigint;
export declare function i64(value: IntegerLike, name?: string): bigint;
/** Borsh-style little-endian writer. */
export declare class ByteWriter {
    private readonly parts;
    u8(value: number, name?: string): this;
    u16(value: number, name?: string): this;
    u32(value: number, name?: string): this;
    u64(value: IntegerLike, name?: string): this;
    i64(value: IntegerLike, name?: string): this;
    bytes(value: Uint8Array, length?: number): this;
    pubkey(value: Address): this;
    toBuffer(): Buffer;
}
/** Bounds-checked little-endian reader over account bytes. */
export declare class ByteReader {
    readonly bytes: Uint8Array;
    private readonly view;
    constructor(data: Uint8Array);
    get length(): number;
    private check;
    u8(at: number): number;
    u16(at: number): number;
    u32(at: number): number;
    u64(at: number): bigint;
    i64(at: number): bigint;
    u128(at: number): bigint;
    i128(at: number): bigint;
    slice(at: number, n: number): Uint8Array;
    pubkey(at: number): PublicKey;
    /** True when `bytes[at..at+expected.length]` equals `expected`. */
    matches(at: number, expected: Uint8Array): boolean;
}
export declare function ascii(text: string): Uint8Array;
//# sourceMappingURL=codec.d.ts.map