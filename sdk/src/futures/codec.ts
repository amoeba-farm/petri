/**
 * Little-endian byte helpers for the futures module. Self-contained: the
 * futures subpath has no other runtime dependencies in this package.
 */
import { Buffer } from "buffer";
import { PublicKey } from "@solana/web3.js";

/** Anything a `PublicKey` can be built from without ambiguity. */
export type Address = PublicKey | string;

export function toPublicKey(value: Address): PublicKey {
  return value instanceof PublicKey ? value : new PublicKey(value);
}

export const U8_MAX = 0xff;
export const U16_MAX = 0xffff;
export const U32_MAX = 0xffff_ffff;
export const U64_MAX = (1n << 64n) - 1n;
export const I64_MIN = -(1n << 63n);
export const I64_MAX = (1n << 63n) - 1n;
export const U128_MAX = (1n << 128n) - 1n;

export type IntegerLike = bigint | number;

function integer(value: IntegerLike, name: string): bigint {
  if (typeof value === "bigint") return value;
  if (typeof value === "number" && Number.isSafeInteger(value)) return BigInt(value);
  throw new RangeError(`${name} must be a safe integer or bigint`);
}

export function u8(value: number, name = "u8"): number {
  if (!Number.isInteger(value) || value < 0 || value > U8_MAX) throw new RangeError(`${name} must be a u8`);
  return value;
}
export function u16(value: number, name = "u16"): number {
  if (!Number.isInteger(value) || value < 0 || value > U16_MAX) throw new RangeError(`${name} must be a u16`);
  return value;
}
export function u32(value: number, name = "u32"): number {
  if (!Number.isInteger(value) || value < 0 || value > U32_MAX) throw new RangeError(`${name} must be a u32`);
  return value;
}
export function u64(value: IntegerLike, name = "u64"): bigint {
  const v = integer(value, name);
  if (v < 0n || v > U64_MAX) throw new RangeError(`${name} must be a u64`);
  return v;
}
export function i64(value: IntegerLike, name = "i64"): bigint {
  const v = integer(value, name);
  if (v < I64_MIN || v > I64_MAX) throw new RangeError(`${name} must be an i64`);
  return v;
}

/** Borsh-style little-endian writer. */
export class ByteWriter {
  private readonly parts: Uint8Array[] = [];

  u8(value: number, name?: string): this {
    this.parts.push(Uint8Array.of(u8(value, name)));
    return this;
  }
  u16(value: number, name?: string): this {
    const b = new Uint8Array(2);
    new DataView(b.buffer).setUint16(0, u16(value, name), true);
    this.parts.push(b);
    return this;
  }
  u32(value: number, name?: string): this {
    const b = new Uint8Array(4);
    new DataView(b.buffer).setUint32(0, u32(value, name), true);
    this.parts.push(b);
    return this;
  }
  u64(value: IntegerLike, name?: string): this {
    const b = new Uint8Array(8);
    new DataView(b.buffer).setBigUint64(0, u64(value, name), true);
    this.parts.push(b);
    return this;
  }
  i64(value: IntegerLike, name?: string): this {
    const b = new Uint8Array(8);
    new DataView(b.buffer).setBigInt64(0, i64(value, name), true);
    this.parts.push(b);
    return this;
  }
  bytes(value: Uint8Array, length?: number): this {
    if (length !== undefined && value.length !== length) throw new RangeError(`expected ${length} bytes`);
    this.parts.push(Uint8Array.from(value));
    return this;
  }
  pubkey(value: Address): this {
    return this.bytes(toPublicKey(value).toBytes(), 32);
  }
  toBuffer(): Buffer {
    return Buffer.concat(this.parts.map((p) => Buffer.from(p.buffer, p.byteOffset, p.byteLength)));
  }
}

/** Bounds-checked little-endian reader over account bytes. */
export class ByteReader {
  readonly bytes: Uint8Array;
  private readonly view: DataView;

  constructor(data: Uint8Array) {
    this.bytes = data;
    this.view = new DataView(data.buffer, data.byteOffset, data.byteLength);
  }
  get length(): number {
    return this.bytes.length;
  }
  private check(at: number, n: number): void {
    if (!Number.isInteger(at) || at < 0 || at + n > this.bytes.length) {
      throw new RangeError(`read of ${n} bytes at ${at} is outside ${this.bytes.length} bytes`);
    }
  }
  u8(at: number): number {
    this.check(at, 1);
    return this.view.getUint8(at);
  }
  u16(at: number): number {
    this.check(at, 2);
    return this.view.getUint16(at, true);
  }
  u32(at: number): number {
    this.check(at, 4);
    return this.view.getUint32(at, true);
  }
  u64(at: number): bigint {
    this.check(at, 8);
    return this.view.getBigUint64(at, true);
  }
  i64(at: number): bigint {
    this.check(at, 8);
    return this.view.getBigInt64(at, true);
  }
  u128(at: number): bigint {
    return this.u64(at) | (this.u64(at + 8) << 64n);
  }
  i128(at: number): bigint {
    const v = this.u128(at);
    return v >= 1n << 127n ? v - (1n << 128n) : v;
  }
  slice(at: number, n: number): Uint8Array {
    this.check(at, n);
    return this.bytes.slice(at, at + n);
  }
  pubkey(at: number): PublicKey {
    return new PublicKey(this.slice(at, 32));
  }
  /** True when `bytes[at..at+expected.length]` equals `expected`. */
  matches(at: number, expected: Uint8Array): boolean {
    if (at + expected.length > this.bytes.length) return false;
    for (let i = 0; i < expected.length; i += 1) if (this.bytes[at + i] !== expected[i]) return false;
    return true;
  }
}

export function ascii(text: string): Uint8Array {
  const out = new Uint8Array(text.length);
  for (let i = 0; i < text.length; i += 1) {
    const c = text.charCodeAt(i);
    if (c > 0x7f) throw new RangeError("expected ASCII");
    out[i] = c;
  }
  return out;
}
