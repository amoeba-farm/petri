/**
 * `ameba-sdk/futures`: client for Ameba futures (`ameba-futures`): the
 * program's constants and payoffs (`program.ts`), `FuturesBook` and the Earn
 * vault decoders (`accounts.ts`), the proposal codec (`proposal.ts`),
 * `FuturesMarket` and `FuturesEarnVault` (`instructions.ts`) and
 * `FuturesClient` (`api.ts`), with the shared codec, PDAs, unit helpers and
 * governance gate binding.
 *
 * Golden vectors from the Rust client crate pin every encoder and decoder.
 */
export {
  ByteReader, ByteWriter, I64_MAX, I64_MIN, U128_MAX, U64_MAX, toPublicKey,
  type Address, type IntegerLike,
} from "./codec.js";
export * from "./constants.js";
export * from "./pda.js";
export * from "./math.js";
export * from "./governance.js";
export * from "./program.js";
export * from "./accounts.js";
export * from "./proposal.js";
export * from "./instructions.js";
export * from "./api.js";
