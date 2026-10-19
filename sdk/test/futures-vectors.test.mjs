// Golden vectors from tools/futures-vectors (`futures.json`, from the Rust
// futures client crate and the futures program's state types): PDAs,
// every FuturesMarket and FuturesEarnVault instruction, the market account and
// its arcs and values, the Earn vault accounts, solved proposals, tiers,
// payoffs and error names.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { PublicKey } from "@solana/web3.js";
import {
  FuturesAccountError, GovernanceGateBinding, ERROR_CODES, HEAP_FRAME, FuturesBook, FuturesEarnVault, FuturesMarket,
  FuturesProposalError, FuturesInstructionTag, VAULT_LEN, VAULT_LP_LEN, VAULT_MARKETS,
  decodeHeader, decodeFuturesInstruction, decodeProposal, decodeRegistry, encodeFuturesInstruction,
  encodeProposal, findAuthorityAddress, findLpAddress, findPoolAddress, findRegistryAddress, findVaultAddress,
  findVaultPositionAddress, decodeLpPosition, decodePool, decodeVault, decodeVaultPosition, operated, averageEntry, closeArgs,
  VAULT_REWARDS_LEN,
  computeLimit, contribution, entryOf, futuresErrorName, fineToValue, heapFrame, longPerLot,
  marketFilters, marketSpace, positionValue, premiumAt, proposalDense, proposalFromFills,
  setValue, settleFine, shortPerLot, takerPremium, tierPrice, tierRemaining, tierSizes,
  transactionErrorName, findPeriodAddress, productId, productSymbol,
} from "../dist/futures/index.js";

const v = JSON.parse(readFileSync(new URL("./fixtures/futures/futures.json", import.meta.url), "utf8")).vectors;
const bytes = (b64) => Uint8Array.from(Buffer.from(b64, "base64"));
const hex = (b) => Buffer.from(b).toString("hex");
const fromHex = (h) => Uint8Array.from(Buffer.from(h, "hex"));

/** Normalizes decoded values (PublicKey, bigint, Uint8Array) to the fixture's JSON forms. */
function plain(value) {
  if (typeof value === "bigint") return value.toString();
  if (value instanceof Uint8Array) return hex(value);
  if (value && typeof value.toBase58 === "function") return value.toBase58();
  if (Array.isArray(value)) return value.map(plain);
  if (value && typeof value === "object") return Object.fromEntries(Object.entries(value).map(([k, x]) => [k, plain(x)]));
  return value;
}

function ixJson(ix) {
  return {
    programId: ix.programId.toBase58(),
    data: hex(ix.data),
    accounts: ix.keys.map((k) => ({ pubkey: k.pubkey.toBase58(), isSigner: k.isSigner, isWritable: k.isWritable })),
  };
}

test("futures PDAs match the Rust client", () => {
  for (const c of v.pdas.registries) {
    const [registry, bump] = findRegistryAddress(v.pdas.program, c.symbol, BigInt(c.expiryTs));
    assert.deepEqual([registry.toBase58(), bump], [c.registry, c.registryBump], `${c.symbol} ${c.expiryTs}`);
    assert.equal(hex(productId(c.symbol)), c.productId);
    assert.equal(productSymbol(fromHex(c.productId)), c.symbol);
    const [period, periodBump] = findPeriodAddress(v.pdas.oracle, c.symbol, BigInt(c.expiryTs));
    assert.deepEqual([period.toBase58(), periodBump], [c.period, c.periodBump], `period ${c.symbol} ${c.expiryTs}`);
  }
  for (const c of v.pdas.authorities) {
    const [authority, bump] = findAuthorityAddress(v.pdas.program, c.market);
    assert.deepEqual([authority.toBase58(), bump], [c.authority, c.authorityBump]);
    assert.equal(FuturesMarket.authorityOf(v.pdas.program, c.market).toBase58(), c.authority);
  }
  for (const c of v.pdas.pools) {
    const [pool, bump] = findPoolAddress(v.pdas.program, c.market, c.operator);
    assert.deepEqual([pool.toBase58(), bump], [c.pool, c.poolBump]);
    for (const p of c.positions) {
      const [lp, lb] = findLpAddress(v.pdas.program, pool, p.owner);
      assert.deepEqual([lp.toBase58(), lb], [p.lp, p.lpBump]);
    }
  }
  for (const c of v.pdas.vaults) {
    const [vault, bump] = findVaultAddress(v.pdas.program, c.operator);
    assert.deepEqual([vault.toBase58(), bump], [c.vault, c.vaultBump]);
    assert.equal(FuturesEarnVault.address(v.pdas.program, c.operator).toBase58(), c.vault);
    for (const p of c.positions) {
      const [position, pb] = findVaultPositionAddress(v.pdas.program, vault, p.owner);
      assert.deepEqual([position.toBase58(), pb], [p.position, p.positionBump]);
    }
  }
});

function market() {
  const m = v.instructions.market;
  return FuturesMarket.derive(m.program, m.market, { vault: m.vault, period: m.period, mint: m.mint },
    v.instructions.productSymbol, BigInt(v.instructions.expiryTs));
}

function ref(r) {
  return { slot: r.slot, seq: BigInt(r.seq) };
}

function ladder(l) {
  return { ...l, bidLots: BigInt(l.bidLots), askLots: BigInt(l.askLots) };
}

function earnVault() {
  const e = v.instructions.earnVault;
  return FuturesEarnVault.derive(e.program, e.operator, { cash: e.cash, mint: e.mint });
}

/** A drawn market from its vector: only its market, token vault and authority matter. */
function drawn(d) {
  return { market: new PublicKey(d.market), vault: new PublicKey(d.vault), authority: new PublicKey(d.authority) };
}

function build(m, c) {
  const a = c.args;
  const earn = earnVault();
  switch (c.method) {
    case "create": {
      const p = a.params;
      const params = {
        productId: p.symbol, expiryTs: BigInt(p.expiryTs), grid: p.grid, k: BigInt(p.k), stepValue: BigInt(p.stepValue),
        valueDecimals: p.valueDecimals, refValue: BigInt(p.refValue), traderCap: p.traderCap, orderCap: p.orderCap,
        makerCap: p.makerCap, claimCap: p.claimCap,
      };
      assert.equal(marketSpace(params), a.space);
      return m.create(a.creator, a.oracleProgram, params);
    }
    case "deposit": return m.deposit(a.owner, a.token, BigInt(a.amount));
    case "withdraw": return a.amount === "18446744073709551615" ? m.withdraw(a.owner, a.token) : m.withdraw(a.owner, a.token, BigInt(a.amount));
    case "submit": return m.submit(a.owner, {
      side: a.side, lo: a.lo, hi: a.hi, lots: BigInt(a.lots), limit: BigInt(a.limit), minFill: BigInt(a.minFill), mode: a.mode,
      close: a.close && ref(a.close),
    }, fromHex(a.proposal));
    case "cancel": return m.cancel(a.owner, a.slot, BigInt(a.seq));
    case "quote": return m.quote(a.owner, ladder(a.ladder), a.knots, fromHex(a.proposal));
    case "quoteCore": return m.quoteCore(a.owner, ladder(a.ladder), a.core, a.knots, fromHex(a.proposal));
    case "cancelQuote": return m.cancelQuote(a.owner);
    case "split": return m.split(a.owner, ref(a.claim), a.at);
    case "merge": return m.merge(a.owner, ref(a.keep), ref(a.absorb));
    case "redeem": return m.redeem(a.owner, ref(a.long), ref(a.short), BigInt(a.lots));
    case "transfer": return m.transfer(a.owner, ref(a.claim), BigInt(a.lots), new PublicKey(a.to));
    case "finalize": return m.finalize();
    case "settle": return m.settle(a.claims);
    case "settleMaker": return m.settleMaker(a.maker);
    case "prune": return m.prune(a.orders);
    case "heapFrame": return heapFrame(a.bytes);
    case "computeLimit": return computeLimit(a.units);
    case "poolWithdraw": return a.shares === "18446744073709551615" ? m.poolWithdraw(a.pool, a.owner, a.token)
      : m.poolWithdraw(a.pool, a.owner, a.token, BigInt(a.shares));
    case "operated": {
      assert.equal(m.pool(a.operator).toBase58(), a.pool);
      return operated(m.cancel(a.pool, a.slot, BigInt(a.seq)), a.operator);
    }
    case "createVault": return earn.create(a.payer);
    case "vaultDeposit": return earn.deposit(a.owner, a.token, BigInt(a.amount), a.payer, a.markets);
    case "vaultWithdraw": return earn.withdraw(a.owner, a.token, BigInt(a.shares), a.markets, a.draws.map(drawn));
    case "vaultFund": return earn.fund(m, BigInt(a.amount));
    case "vaultDefund": return a.amount === "18446744073709551615" ? earn.defund(m) : earn.defund(m, BigInt(a.amount));
    case "poolMigrate": return m.poolMigrate(a.pool, a.holder, earn, a.payer, a.markets);
    case "operatedVault": {
      assert.equal(earn.vault.toBase58(), a.vault);
      return operated(m.cancel(a.vault, a.slot, BigInt(a.seq)), a.operator);
    }
    default: throw new Error(`no builder for ${c.method}`);
  }
}

test("every FuturesMarket builder matches the Rust client byte for byte", () => {
  const m = market();
  for (const [k, want] of Object.entries(v.instructions.market)) assert.equal(m[k].toBase58(), want, k);
  assert.equal(HEAP_FRAME, v.instructions.heapFrame);
  assert.equal(hex(heapFrame().data), v.instructions.cases.find((c) => c.name === "heapFrame").data);
  for (const c of v.instructions.cases) {
    const ix = build(m, c);
    assert.deepEqual(ixJson(ix), { programId: c.programId, data: c.data, accounts: c.accounts }, c.name);
    if (c.programId !== m.program.toBase58()) continue;
    // Decode, re-encode: the same bytes.
    const decoded = decodeFuturesInstruction(fromHex(c.data));
    assert.equal(hex(encodeFuturesInstruction(decoded)), c.data, `${c.name} roundtrip`);
    assert.throws(() => decodeFuturesInstruction(Buffer.concat([fromHex(c.data), Buffer.of(0)])), RangeError);
    if (c.data.length > 2) assert.throws(() => decodeFuturesInstruction(fromHex(c.data.slice(0, -2))), RangeError);
  }
  const submit = decodeFuturesInstruction(fromHex(v.instructions.cases.find((c) => c.name === "submitClose").data));
  assert.deepEqual(plain(submit.close), { slot: 4, seq: "17" });
  assert.equal(decodeFuturesInstruction(fromHex(v.instructions.cases.find((c) => c.name === "submitIoc").data)).close, null);
  // CouncilSettle (tag 18): the council's value for a market the oracle can't settle.
  const value = Buffer.alloc(8);
  value.writeBigUInt64LE(101_250_000n);
  const council = Buffer.concat([Buffer.of(18), value]);
  assert.deepEqual(decodeFuturesInstruction(council), { kind: "councilSettle", value: 101_250_000n });
  assert.equal(hex(encodeFuturesInstruction({ kind: "councilSettle", value: 101_250_000n })), hex(council));
  assert.throws(() => decodeFuturesInstruction(Uint8Array.of(25)), RangeError);
  // Retired tags still decode (marked), so an old transaction reviews; nothing builds them.
  const retired = Buffer.concat([Buffer.of(FuturesInstructionTag.PoolDeposit), Buffer.from("404b4c0000000000", "hex")]);
  assert.deepEqual(decodeFuturesInstruction(Uint8Array.of(15)), { kind: "createPool", retired: true });
  assert.deepEqual(decodeFuturesInstruction(retired), { kind: "poolDeposit", amount: 5_000_000n, retired: true });
  assert.equal(hex(encodeFuturesInstruction(decodeFuturesInstruction(retired))), hex(retired));
  assert.equal(m.createPool, undefined);
  assert.equal(m.poolDeposit, undefined);
  assert.deepEqual([FuturesInstructionTag.CreateVault, FuturesInstructionTag.PoolMigrate], [19, 24]);
  // Its accounts as the Rust client's `council_settle`: market, period, authority (signer), program, ProgramData.
  const authority = new PublicKey("11111111111111111111111111111112");
  const programData = PublicKey.findProgramAddressSync(
    [m.program.toBytes()], new PublicKey("BPFLoaderUpgradeab1e11111111111111111111111"))[0];
  assert.deepEqual(m.councilSettle(authority, 101_250_000n).keys.map((k) => [k.pubkey.toBase58(), k.isSigner, k.isWritable]), [
    [m.market.toBase58(), false, true], [m.period.toBase58(), false, false], [authority.toBase58(), true, false],
    [m.program.toBase58(), false, false], [programData.toBase58(), false, false],
  ]);
  // The Earn vault from its account; no draws leave every market readonly.
  const earn = earnVault();
  const fromAccount = FuturesEarnVault.fromAccount(earn.program, decodeVault(bytes(v.accounts.vault.vault.data)));
  assert.deepEqual([fromAccount.vault, fromAccount.operator, fromAccount.cash, fromAccount.mint].map((k) => k.toBase58()),
    [earn.vault, earn.operator, earn.cash, earn.mint].map((k) => k.toBase58()));
  assert.ok(earn.withdraw(PublicKey.default, PublicKey.default, 1n, [m.market]).keys.slice(6).every((k) => !k.isWritable));
  // Wire widths are checked, not truncated.
  assert.throws(() => earn.deposit(PublicKey.default, PublicKey.default, 1n << 64n, PublicKey.default, []), RangeError);
  assert.throws(() => m.cancel(PublicKey.default, 65_536, 1n), RangeError);
  assert.throws(() => m.deposit(PublicKey.default, PublicKey.default, -1n), RangeError);
  assert.throws(() => m.submit(PublicKey.default, { side: 2, lo: 0, hi: 1, lots: 1n, limit: 0n, minFill: 0n, mode: "ioc" },
    new Uint8Array()), RangeError);
  assert.throws(() => m.submit(PublicKey.default, { side: "long", lo: 0, hi: 1, lots: 1n, limit: 0n, minFill: 0n, mode: "fok" },
    new Uint8Array()), RangeError);
});

test("a governed futures instruction binds and reviews like the Rust client's", () => {
  const g = v.instructions.governance;
  const m = market();
  const binding = GovernanceGateBinding.fromAccount(g.controller, m.program, g.gate, g.controller, fromHex(g.gateData));
  const raw = m.cancel(g.raw.accounts[1].pubkey, 4, 17n);
  assert.deepEqual(ixJson(raw), { programId: g.raw.programId, data: g.raw.data, accounts: g.raw.accounts });
  const bound = binding.bind(raw);
  assert.deepEqual(ixJson(bound), { programId: g.bound.programId, data: g.bound.data, accounts: g.bound.accounts });
  assert.deepEqual(plain(decodeFuturesInstruction(bound.data, { governed: true })), { kind: "cancel", slot: 4, seq: "17" });
  assert.throws(() => decodeFuturesInstruction(bound.data), RangeError);
  assert.throws(() => decodeFuturesInstruction(raw.data, { governed: true }));
});

const HEADER_KEYS = ["status", "authorityBump", "valueDecimals", "grid", "traderCap", "orderCap", "makerCap", "registryBump",
  "claimCap", "claimHint", "productId", "expiryTs", "period", "oracleProgram", "quoteMint", "vault", "creator", "k", "stepValue",
  "refFine", "settlementFine", "nextSeq", "totalFree", "totalReserved", "backing", "custody", "trades", "volumeLots",
  "volumePremium", "lastEntryFine", "claimsOpen", "totalPaidOut", "settledTs", "lastTradeTs"];

function header(h) {
  return Object.fromEntries(HEADER_KEYS.map((k) => [k, plain(h[k])]));
}

test("the market account decodes every record as the program stores it", () => {
  const a = v.accounts.open;
  const data = bytes(a.data);
  const book = FuturesBook.decode(data);
  assert.deepEqual(header(book.header), a.header);
  assert.deepEqual([book.header.symbol, book.header.statusName], ["RAMX", "open"]);
  assert.deepEqual(header(decodeHeader(data)), a.header);
  a.traders.forEach((t) => assert.deepEqual(plain(book.trader(t.slot)), t));
  a.orders.forEach((o) => {
    const { close, ...got } = plain(book.order(o.slot));
    assert.deepEqual(got, o);
    assert.equal(close, (o.flags & 1) === 1);
  });
  a.claims.forEach((c) => assert.deepEqual(plain(book.claim(c.slot)), c));
  a.makers.forEach((mk) => {
    const { steps, ...head } = mk;
    assert.deepEqual(plain(book.maker(mk.slot)), head);
    steps.forEach((st) => assert.deepEqual(plain(book.step(mk.slot, st.pos)), st));
  });
  // The client's filtered views.
  assert.deepEqual(book.traders().map((t) => t.slot), a.book.traders);
  assert.deepEqual(book.orders().map((o) => o.slot), a.book.orders);
  assert.deepEqual(book.claims().map((c) => c.slot), a.book.claims);
  assert.deepEqual(book.claimsOf(0).map((c) => c.slot), a.book.claimsOf0);
  assert.deepEqual(book.makers().map((l) => ({ slot: l.maker.slot, steps: l.steps.map((s) => s.pos) })), a.book.makers);
  for (const o of a.book.owners) {
    assert.equal(book.traderOf(o.owner)?.slot ?? null, o.trader);
    assert.deepEqual(plain(book.valueOf(o.owner)), o.value);
  }
  // The complete arc list the program verifies against.
  assert.deepEqual(plain(book.arcs()), a.arcs);
  assert.equal(book.arcs().filter((x) => x.id.kind === "maker").length, a.ladderArcs);
  for (const t of a.traderValues) assert.deepEqual(plain(book.traderValue(t.slot)), t.value);
  // Loader checks: magic, version and exact length.
  assert.throws(() => FuturesBook.decode(data.subarray(0, data.length - 1)), FuturesAccountError);
  for (const at of [0, 8]) {
    const bad = Uint8Array.from(data);
    bad[at] ^= 1;
    assert.throws(() => FuturesBook.decode(bad), FuturesAccountError);
  }
  assert.throws(() => book.trader(a.header.traderCap), RangeError);
  assert.throws(() => book.step(0, a.header.grid), RangeError);
  assert.equal(Buffer.from(marketFilters()[0].memcmp.bytes, "base64").toString(), "AFRANGE1");
});

test("final markets value holdings at the settlement", () => {
  for (const key of ["settled", "failed"]) {
    const a = v.accounts[key];
    const book = FuturesBook.decode(bytes(a.data));
    assert.deepEqual(header(book.header), a.header, key);
    assert.equal(book.header.statusName, key);
    for (const t of a.traderValues) assert.deepEqual(plain(book.traderValue(t.slot)), t.value, `${key} ${t.slot}`);
    for (const o of a.owners) assert.deepEqual(plain(book.valueOf(o.owner)), o.value);
  }
  const pools = v.accounts.pools;
  const { data: poolData, ...pool } = pools.pool;
  assert.deepEqual(plain(decodePool(bytes(poolData))), pool);
  const { data: lpData, ...lp } = pools.lp;
  assert.deepEqual(plain(decodeLpPosition(bytes(lpData))), lp);
  assert.throws(() => decodePool(bytes(lpData)), FuturesAccountError);
  assert.throws(() => decodeLpPosition(bytes(poolData)), FuturesAccountError);
  // The Earn vault and a position in it.
  const { data: vaultData, ...vault } = v.accounts.vault.vault;
  assert.deepEqual(plain(decodeVault(bytes(vaultData))), vault);
  const { data: positionData, ...position } = v.accounts.vault.position;
  assert.deepEqual(plain(decodeVaultPosition(bytes(positionData))), position);
  assert.deepEqual([bytes(vaultData).length, bytes(positionData).length, VAULT_MARKETS], [VAULT_LEN, VAULT_LP_LEN, 13]);
  assert.throws(() => decodeVault(bytes(vaultData).subarray(1)), FuturesAccountError);
  assert.throws(() => decodeVaultPosition(bytes(lpData)), FuturesAccountError);
  assert.throws(() => decodeLpPosition(bytes(positionData)), FuturesAccountError);
  // With an AMBA stream: the same vault, grown by the rewards section.
  const rewarded = decodeVault(bytes(v.accounts.vault.rewarded.data));
  assert.deepEqual(plain(rewarded.rewards), v.accounts.vault.rewarded.rewards);
  assert.deepEqual(plain({ ...rewarded, rewards: null }), vault);
  assert.equal(bytes(v.accounts.vault.rewarded.data).length, VAULT_REWARDS_LEN);
  assert.throws(() => decodeVault(bytes(v.accounts.vault.rewarded.data).subarray(1)), FuturesAccountError);
  const crowded = bytes(vaultData);
  crowded[9] = VAULT_MARKETS + 1;
  assert.throws(() => decodeVault(crowded), FuturesAccountError);
  crowded[9] = VAULT_MARKETS;
  assert.equal(decodeVault(crowded).markets.length, VAULT_MARKETS);
  const r = v.accounts.registry;
  assert.deepEqual(plain(decodeRegistry(bytes(r.data))), { bump: r.bump, market: r.market });
  assert.throws(() => decodeRegistry(bytes(r.data).subarray(1)), FuturesAccountError);
  const m = FuturesMarket.fromHeader(v.accounts.program, v.accounts.address, FuturesBook.decode(bytes(v.accounts.open.data)).header);
  assert.equal(m.vault.toBase58(), v.accounts.open.header.vault);
  assert.equal(m.registry.toBase58(), findRegistryAddress(v.accounts.program, "RAMX", BigInt(v.accounts.open.header.expiryTs))[0].toBase58());
});

test("solved proposals decode, densify over the book's arcs and re-encode exactly", () => {
  const book = FuturesBook.decode(bytes(v.accounts.open.data));
  const arcs = book.arcs();
  for (const p of v.proposals) {
    const decoded = decodeProposal(fromHex(p.proposal));
    assert.deepEqual(plain(decoded.fills), p.decoded);
    assert.equal(decoded.takerFill.toString(), p.takerFill);
    assert.deepEqual(plain(decoded.potentials), p.potentials);
    const dense = proposalDense(decoded, arcs);
    assert.deepEqual(plain(dense), p.fills);
    assert.equal(takerPremium(p.taker.side, arcs, dense).toString(), p.premium);
    assert.equal(hex(encodeProposal(decoded)), p.proposal);
    const rebuilt = proposalFromFills(arcs, p.fills.map(BigInt), BigInt(p.takerFill), p.potentials.map(BigInt));
    assert.equal(hex(encodeProposal(rebuilt)), p.proposal);
  }
  // A fill of an arc the book does not have.
  assert.throws(() => proposalDense({ fills: [{ id: { kind: "order", slot: 1 }, lots: 1n }], takerFill: 0n, potentials: [] }, arcs),
    FuturesProposalError);
});

test("the proposal codec matches futures-math's, including its rejections", () => {
  for (const c of v.codec.good) {
    const decoded = decodeProposal(fromHex(c.hex));
    assert.deepEqual(plain(decoded), { fills: c.fills, takerFill: c.takerFill, potentials: c.potentials });
    assert.equal(hex(encodeProposal(decoded)), c.hex);
    // Fills in any order encode canonically.
    assert.equal(hex(encodeProposal({ ...decoded, fills: [...decoded.fills].reverse() })), c.hex);
  }
  for (const b of v.codec.bad) assert.throws(() => decodeProposal(fromHex(b)), FuturesProposalError, b);
  assert.throws(() => encodeProposal({ fills: [{ id: { kind: "order", slot: 0 }, lots: 0n }], takerFill: 0n, potentials: [] }), RangeError);
  assert.throws(() => encodeProposal({ fills: [], takerFill: 0n, potentials: [1n << 63n] }), RangeError);
});

test("ladder tiers split and price as the program does", () => {
  for (const c of v.tiers) {
    const t = { count: c.count, ratio: c.ratio, spread: 250 };
    const total = BigInt(c.total);
    assert.deepEqual(plain(tierSizes(t, total)), c.sizes, JSON.stringify(c));
    for (const r of c.remaining) assert.deepEqual(plain(tierRemaining(t, total, BigInt(r.remaining))), r.lots);
    assert.deepEqual(c.bidPrices, c.sizes.map((_, i) => tierPrice(t, 9_800, 0, i, 10_000n).toString()));
    assert.deepEqual(c.askPrices, c.sizes.map((_, i) => tierPrice(t, 9_800, 1, i, 10_000n).toString()));
  }
});

test("payoffs, settlement and entries are exact", () => {
  const m = v.math;
  for (const c of m.payoffs) {
    assert.equal(setValue(c.lo, c.hi, BigInt(c.k)).toString(), c.set);
    assert.equal(longPerLot(c.lo, c.hi, BigInt(c.s), BigInt(c.k)).toString(), c.long, JSON.stringify(c));
    assert.equal(shortPerLot(c.lo, c.hi, BigInt(c.s), BigInt(c.k)).toString(), c.short);
  }
  for (const c of m.settleFine) assert.equal(settleFine(BigInt(c.value), BigInt(c.stepValue), BigInt(c.k)).toString(), c.fine);
  for (const c of m.contributions) {
    assert.equal(contribution(c.side, c.lo, c.hi, BigInt(c.price), BigInt(c.k)).toString(), c.contribution);
  }
  for (const c of m.entries) {
    const premium = premiumAt(c.lo, c.hi, BigInt(c.entryFine), BigInt(c.k));
    assert.equal(premium.toString(), c.premium);
    assert.equal(entryOf(c.lo, premium, BigInt(c.k)).toString(), c.entryOf);
  }
  // Units: dev step 1 index point = 1_000_000 value units at 6 decimals; k = 10_000.
  assert.equal(positionValue(98, 1_000_000n, 6), "98");
  assert.equal(fineToValue(1_029_155n, 1_000_000n, 10_000n, 6), "102.9155");
  assert.equal(averageEntry(84, 13_240_850n, 70n, 10_000n), 1_029_155n);
  assert.equal(averageEntry(84, -5n, 70n, 10_000n), 840_000n);
});

test("closing a claim is the opposite side on its interval, naming it", () => {
  const long = { slot: 3, seq: 9n, side: 0, lo: 90, hi: 110, lots: 5n, reserved: 2n };
  assert.deepEqual(plain(closeArgs(long, 10_000n)),
    { side: 1, lo: 90, hi: 110, lots: "3", limit: "0", minFill: "1", mode: 0, close: { slot: 3, seq: "9" } });
  const short = { ...long, side: 1 };
  assert.deepEqual(plain(closeArgs(short, 10_000n, { lots: 1n, limit: 150_000n })),
    { side: 0, lo: 90, hi: 110, lots: "1", limit: "150000", minFill: "1", mode: 0, close: { slot: 3, seq: "9" } });
  assert.equal(closeArgs(short, 10_000n).limit, 200_000n);
  const m = market();
  const ix = m.close(PublicKey.default, long, 10_000n, Uint8Array.of(0, 0, 0, 0));
  assert.deepEqual(plain(decodeFuturesInstruction(ix.data)), {
    kind: "submit", side: 1, lo: 90, hi: 110, lots: "3", limit: "0", minFill: "1", mode: 0, close: { slot: 3, seq: "9" },
    proposal: "00000000",
  });
});

test("futures error names match the client", () => {
  for (const c of v.errors) assert.equal(futuresErrorName(c.code), c.name, String(c.code));
  assert.equal(ERROR_CODES.MinFill, 0x46520012);
  assert.equal(ERROR_CODES.VaultCapacity, 0x46520000 + 31);
  assert.equal(ERROR_CODES.NeedsRecovery, 0x46520000 + 33);
  assert.equal(transactionErrorName(0x46520007), "InsufficientFunds");
  assert.equal(transactionErrorName(3108), "GovernanceGateFrozen");
  assert.equal(transactionErrorName(7), null);
});
