// FuturesClient against recorded responses of the dev futures API
// (test/fixtures/api/responses.json, read from 127.0.0.1:4500): the
// routes and bodies it sends, the views it returns, its errors, and the
// sponsored prepare → sign → submit flow on a transaction built like the API's.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { Keypair, PublicKey, Transaction } from "@solana/web3.js";
import {
  GovernanceGateBinding, FuturesApiError, FuturesClient, FuturesMarket, decodeProposal, findGovernanceGateAddress,
  findProgramDataAddress, preparedTransaction, computeLimit, heapFrame, reviewPrepared, signPrepared,
} from "../dist/futures/index.js";

const fx = JSON.parse(readFileSync(new URL("./fixtures/api/responses.json", import.meta.url), "utf8"));
const R = fx.responses;

/** A fetch that records requests and answers from a route table. */
function mockFetch(routes) {
  const calls = [];
  const fetch = async (url, init = {}) => {
    const u = new URL(url);
    const call = { method: init.method ?? "GET", path: u.pathname, query: Object.fromEntries(u.searchParams), headers: init.headers,
      body: init.body ? JSON.parse(init.body) : undefined };
    calls.push(call);
    const route = routes[`${call.method} ${call.path}`];
    if (!route) return new Response(JSON.stringify({ error: "no route" }), { status: 404 });
    const { status = 200, body, headers } = typeof route === "function" ? route(call) : { body: route };
    return new Response(JSON.stringify(body), { status, headers });
  };
  return { fetch, calls };
}

const market = R.market.market;
const owner = R.account.owner;

test("reads hit the futures routes and return the API's views", async () => {
  const { fetch, calls } = mockFetch({
    "GET /v1/markets": R.markets,
    [`GET /v1/markets/${market}`]: R.market,
    "GET /v1/markets/RAMX/book": R.book,
    "GET /v1/markets/RAMX/depth": R.depth,
    "GET /v1/markets/RAMX/trades": R.trades,
    "GET /v1/markets/RAMX/candles": R.candles,
    [`GET /v1/accounts/${owner}`]: R.account,
  });
  // The base path is kept (the site's same-origin proxy `/api/futures`); a trailing slash is dropped.
  const proxy = mockFetch({ "GET /api/futures/v1/markets": R.markets });
  assert.deepEqual(await new FuturesClient({ baseUrl: "http://site.test/api/futures/", fetch: proxy.fetch }).markets(), R.markets.markets);
  assert.equal(proxy.calls[0].path, "/api/futures/v1/markets");

  const direct = new FuturesClient({ baseUrl: "http://api.test", fetch, headers: { "x-test": "1" } });
  assert.deepEqual(await direct.markets(), R.markets.markets);
  const m = await direct.market(market);
  assert.equal(m.symbol, "RAMX");
  assert.equal(typeof m.grid, "number");
  assert.equal(typeof m.k, "string");
  assert.deepEqual(Object.keys(m.suggestion).sort(), ["centerValue", "hi", "highValue", "lo", "lowValue", "sigmaValue", "source", "vol"]);
  assert.deepEqual(Object.keys(m.pool).sort(),
    ["address", "deposited", "free", "operator", "reserved", "shares", "valueHigh", "valueLow", "withdrawn"]);
  const book = await direct.book("RAMX");
  assert.ok(book.sources.every((s) => ["order", "maker"].includes(s.source.kind)));
  assert.ok(book.ladders.every((l) => Array.isArray(l.steps) && typeof l.tiers.count === "number"));
  const depth = await direct.depth("RAMX", { lo: 86, hi: 104, max: 50n });
  assert.equal(depth.asks[0].cumulativeLots, 50);
  assert.deepEqual(calls.at(-1).query, { lo: "86", hi: "104", max: "50" });
  assert.equal((await direct.trades("RAMX", { limit: 3 })).length, 3);
  assert.deepEqual(calls.at(-1).query, { limit: "3" });
  assert.equal((await direct.candles("RAMX", { resolution: 3600, from: 1_792_000_000 })).candles.length, 3);
  assert.deepEqual(calls.at(-1).query, { resolution: "3600", from: "1792000000" });
  const account = await direct.account(new PublicKey(owner));
  assert.ok(account.markets[0].claims.every((c) => c.exit === null || typeof c.exit.net === "string"));
  assert.ok(calls.every((c) => c.method === "GET" && c.headers["x-test"] === "1" && c.headers.accept === "application/json"));
  assert.ok(calls.every((c) => c.path === "/v1/markets" || c.path.startsWith("/v1/markets/") || c.path.startsWith("/v1/accounts/")));
});

// The Earn vault's views, shaped as api/API.md describes them (not recorded: the vault is newer than responses.json).
const EARN_VAULT = {
  address: "3A3eqHrnruUkbXHYEwNJ1gJWigHe86GinQE21m9eWcSe", operator: "E8BpgbWXkDmzX3bKnrEyDT59NjRELg1k9ainkzD2YgeF",
  cashAccount: "BdAzteYEAizVxvPbw7FRqpCDNWxVKgPYNfXVzSDxaheM", shares: "40000000", cash: "12000000", deposited: "40000000",
  withdrawn: "0", valueLow: "39100000", valueHigh: "40700000",
  markets: [{
    market, symbol: "RAMX", label: "RAMX Oct 26", expiryTs: 1_793_491_200, status: "open", tradable: true, free: "9000000",
    reserved: "19000000", valueLow: "27100000", valueHigh: "28700000",
  }],
};

test("Earn reads hit the vault routes", async () => {
  const position = {
    address: "Z2BJgXxsXsJXEok7GUF5hyjkrNQsQz2vLnLTRkDpzJy", exists: true, shares: "10000000", valueLow: "9775000",
    valueHigh: "10175000", deposited: "10000000", withdrawn: "0", earningsLow: "-225000", earningsHigh: "175000", share: "0.25",
    withdrawable: "9775000",
  };
  const activity = [{ signature: "sig2", slot: 12, ts: 1_792_100_000, kind: "deposit", amount: "10000000", shares: "10000000" }];
  const { fetch, calls } = mockFetch({
    "GET /v1/earn": { vault: EARN_VAULT, chainTime: 1_792_100_100 },
    [`GET /v1/accounts/${owner}/earn`]: { owner, usdc: "5000000", vault: EARN_VAULT, position, chainTime: 1_792_100_100 },
    [`GET /v1/accounts/${owner}/earn/activity`]: { owner, activity },
  });
  const api = new FuturesClient({ baseUrl: "http://api.test", fetch });
  const earn = await api.earn();
  assert.equal(earn.vault.markets[0].symbol, "RAMX");
  const mine = await api.earnAccount(new PublicKey(owner));
  assert.deepEqual([mine.position.share, mine.position.withdrawable], ["0.25", "9775000"]);
  assert.deepEqual(await api.earnActivity(owner, { limit: 20, before: "sig9" }), activity);
  assert.deepEqual(calls.map((c) => [c.path, c.query]), [
    ["/v1/earn", {}], [`/v1/accounts/${owner}/earn`, {}], [`/v1/accounts/${owner}/earn/activity`, { limit: "20", before: "sig9" }],
  ]);
});

test("quotes send the API's spellings and integers as strings", async () => {
  const { fetch, calls } = mockFetch({ "POST /v1/quote": R.quote });
  const api = new FuturesClient({ baseUrl: "http://api.test", fetch });
  const q = await api.quote({
    market: "RAMX", owner: new PublicKey(owner), side: 0, lo: 86, hi: 104, lots: 3, limit: 180_000n, mode: 1, minFill: 1n,
    close: { slot: 4, seq: 17n },
  });
  assert.equal(q.filled, 3);
  assert.equal(q.pay, "429369");
  assert.deepEqual(calls[0].body, {
    market: "RAMX", owner, side: "long", lo: 86, hi: 104, lots: 3, limit: "180000", mode: "gtc", minFill: "1",
    close: { slot: 4, seq: "17" },
  });
  assert.equal(calls[0].headers["content-type"], "application/json");
  await api.quote({ market: "RAMX", side: "short", lo: 1, hi: 2, lots: 1, mode: "post", limit: undefined });
  assert.deepEqual(calls[1].body, { market: "RAMX", side: "short", lo: 1, hi: 2, lots: 1, mode: "post" });
});

test("API errors surface status, message, logs and Retry-After", async () => {
  const { fetch } = mockFetch({
    "POST /v1/quote": () => fx.errors.quote,
    "GET /v1/markets/NOPE": () => fx.errors.market,
    "POST /v1/tx/submit": () => ({ status: 422, body: { error: "MinFill", logs: ["Program log: x"] } }),
    "GET /v1/markets": () => ({ status: 429, body: { error: "rate limited" }, headers: { "retry-after": "2" } }),
  });
  const api = new FuturesClient({ baseUrl: "http://api.test", fetch });
  await assert.rejects(api.quote({ market: "RAMX", side: "long", lo: 86, hi: 300, lots: 3 }),
    (e) => e instanceof FuturesApiError && e.status === 400 && e.error === fx.errors.quote.body.error);
  await assert.rejects(api.market("NOPE"), (e) => e.status === 404 && /unknown market/u.test(e.message));
  await assert.rejects(api.submit({ id: "x" }, "AAAA"), (e) => e.status === 422 && e.logs[0] === "Program log: x");
  await assert.rejects(api.markets(), (e) => e.status === 429 && e.retryAfter === 2);
});

/** A transaction shaped like the API's `submit` action: compute limit, heap frame, a governed submit; sponsor pays. */
function preparedLike(ownerKey, sponsor) {
  const program = new PublicKey(new Uint8Array(32).fill(80));
  const controller = new PublicKey(new Uint8Array(32).fill(71));
  const m = FuturesMarket.derive(program, new PublicKey(new Uint8Array(32).fill(81)), {
    vault: new PublicKey(new Uint8Array(32).fill(82)), period: new PublicKey(new Uint8Array(32).fill(83)),
    mint: new PublicKey(new Uint8Array(32).fill(84)),
  }, "RAMX", 1_793_491_200n);
  const [gate, bump] = findGovernanceGateAddress(controller, program);
  const data = Buffer.alloc(192);
  data.write("AGVGAT01"); data[8] = 1; data[9] = bump; data[10] = 1;
  PublicKey.findProgramAddressSync([Buffer.from("ameba-governance-v3"), Buffer.from("council")], controller)[0].toBuffer().copy(data, 12);
  program.toBuffer().copy(data, 44);
  findProgramDataAddress(program)[0].toBuffer().copy(data, 76);
  data.writeBigUInt64LE(5n, 108);
  const binding = GovernanceGateBinding.fromAccount(controller, program, gate, controller, data);
  const proposal = Uint8Array.of(0, 1, 0, 1, 86, 18, 3, 3, 1, 0);
  const submit = binding.bind(m.submit(ownerKey, { side: "long", lo: 86, hi: 104, lots: 3n, limit: 180_000n, minFill: 1n, mode: "ioc" },
    proposal));
  const tx = new Transaction({ feePayer: sponsor, blockhash: "11111111111111111111111111111111", lastValidBlockHeight: 9 });
  tx.add(computeLimit(), heapFrame(), submit);
  const message = tx.serializeMessage();
  // The API ships it with every signature zeroed.
  const wire = Buffer.concat([Buffer.of(2), Buffer.alloc(128), message]);
  return { program, message, wire: wire.toString("base64"), proposal };
}

test("prepare, review, sign and submit keep the prepared message exactly", async () => {
  const ownerKey = Keypair.generate();
  const sponsor = Keypair.generate().publicKey;
  const like = preparedLike(ownerKey.publicKey, sponsor);
  const prepared = { id: "prep-1", transaction: like.wire, blockhash: "11111111111111111111111111111111", lastValidBlockHeight: 9,
    summary: { action: "submit", instructions: ["computeBudget", "heapFrame", "submit"] } };
  const { fetch, calls } = mockFetch({
    "POST /v1/tx/prepare": prepared,
    "POST /v1/tx/submit": { signature: "sig", lastValidBlockHeight: 9 },
    "GET /v1/tx/sig/wait": { signature: "sig", status: "confirmed", slot: 1, processedMs: 1, confirmedMs: 2, timedOut: false, waitedMs: 3,
      events: [{ type: "take", market }] },
    "GET /v1/tx/sig": { signature: "sig", status: "confirmed" },
  });
  const api = new FuturesClient({ baseUrl: "http://api.test", fetch });
  const got = await api.prepare(ownerKey.publicKey, "submit", {
    market: "RAMX", side: "long", lo: 86, hi: 104, lots: 3, limit: 180_000n, close: { slot: 2, seq: 5n }, withdraw: true,
  });
  assert.deepEqual(calls[0].body, {
    owner: ownerKey.publicKey.toBase58(), action: "submit",
    params: { market: "RAMX", side: "long", lo: 86, hi: 104, lots: 3, limit: "180000", close: { slot: 2, seq: "5" }, withdraw: true },
  });

  // Review: compute budget and heap frame are other programs; the submit decodes with its governance epoch.
  const tx = preparedTransaction(got);
  assert.equal(Buffer.from(tx.serializeMessage()).toString("hex"), Buffer.from(like.message).toString("hex"));
  const review = reviewPrepared(tx, like.program);
  assert.deepEqual(review.map((r) => r.program), ["other", "other", "futures"]);
  const submit = review[2];
  assert.equal(submit.epoch, 5n);
  assert.equal(submit.instruction.kind, "submit");
  assert.deepEqual([submit.instruction.lo, submit.instruction.hi, submit.instruction.lots, submit.instruction.limit], [86, 104, 3n, 180_000n]);
  // One ladder run of 18 steps (86..104) at 3 lots each.
  assert.equal(decodeProposal(submit.instruction.proposal).fills.length, 18);

  // Sign as the owner: the message bytes stay the prepared ones, the sponsor's signature stays empty.
  const signed = signPrepared(got, ownerKey);
  assert.equal(Buffer.from(signed.serializeMessage()).toString("hex"), Buffer.from(like.message).toString("hex"));
  const { signature } = await api.submit(got, signed);
  assert.equal(signature, "sig");
  const sent = Transaction.from(Buffer.from(calls[1].body.transaction, "base64"));
  assert.equal(calls[1].body.id, "prep-1");
  assert.ok(sent.signatures.find((s) => s.publicKey.equals(ownerKey.publicKey)).signature);
  assert.equal(sent.signatures.find((s) => s.publicKey.equals(sponsor)).signature, null);
  assert.ok(sent.verifySignatures(false));

  const waited = await api.wait("sig", { commitment: "confirmed", timeoutMs: 5000 });
  assert.equal(waited.status, "confirmed");
  assert.deepEqual(calls[2].query, { commitment: "confirmed", timeoutMs: "5000" });
  assert.equal((await api.transaction("sig")).status, "confirmed");

  // send(): prepare → sign callback → submit in one call.
  const sent2 = await api.send(ownerKey.publicKey, "cancel", { market: "RAMX", slot: 1, seq: 9n }, (t) => {
    t.partialSign(ownerKey);
    return t;
  });
  assert.equal(sent2.signature, "sig");
  assert.equal(sent2.prepared.id, "prep-1");
  assert.deepEqual(calls[4].body.params, { market: "RAMX", slot: 1, seq: "9" });

  // Earn vault actions (no market) and the legacy pools' exit take the same flow.
  await api.prepare(ownerKey.publicKey, "earnDeposit", { amount: 5_000_000n });
  await api.prepare(ownerKey.publicKey, "earnWithdraw", { amount: "all" });
  await api.prepare(ownerKey.publicKey, "earnWithdraw", { amount: 1_250_000 });
  await api.prepare(ownerKey.publicKey, "poolWithdraw", { market: "RAMX", shares: "all" });
  assert.deepEqual(calls.slice(-4).map((c) => [c.body.action, c.body.params]), [
    ["earnDeposit", { amount: "5000000" }],
    ["earnWithdraw", { amount: "all" }],
    ["earnWithdraw", { amount: 1_250_000 }],
    ["poolWithdraw", { market: "RAMX", shares: "all" }],
  ]);

  // Finalizing and settling (also the cash-out of a retired market).
  await api.prepare(ownerKey.publicKey, "finalize", { market: "RAMX" });
  await api.prepare(ownerKey.publicKey, "settle", { market: "RAMX" });
  await api.prepare(ownerKey.publicKey, "settle", { market: "RAMX", withdraw: false });
  assert.deepEqual(calls.slice(-3).map((c) => [c.body.action, c.body.params]), [
    ["finalize", { market: "RAMX" }],
    ["settle", { market: "RAMX" }],
    ["settle", { market: "RAMX", withdraw: false }],
  ]);

  // Cash out: USDC from the owner's wallet to another wallet, no market.
  const destination = Keypair.generate().publicKey;
  await api.prepare(ownerKey.publicKey, "cashOut", { destination, amount: 2_500_000n });
  assert.deepEqual([calls.at(-1).body.action, calls.at(-1).body.params], ["cashOut", { destination: destination.toBase58(), amount: "2500000" }]);
});

test("send moves compressed USDC first: every part signed, then submitted in order, each landed before the next", async () => {
  const ownerKey = Keypair.generate();
  const sponsor = Keypair.generate().publicKey;
  const like = preparedLike(ownerKey.publicKey, sponsor);
  const part = (id, action) => ({ id, transaction: like.wire, blockhash: "11111111111111111111111111111111",
    lastValidBlockHeight: 9, summary: { action, instructions: ["computeBudget", action] } });
  const prepared = { ...part("main", "submit"), before: [part("d1", "decompress"), part("d2", "decompress")] };
  const order = [];
  const { fetch } = mockFetch({
    "POST /v1/tx/prepare": prepared,
    "POST /v1/tx/submit": (call) => { order.push(`submit ${call.body.id}`); return { body: { signature: `sig-${call.body.id}` } }; },
    "GET /v1/tx/sig-d1/wait": () => { order.push("wait d1"); return { body: { signature: "sig-d1", status: "processed" } }; },
    "GET /v1/tx/sig-d2/wait": () => { order.push("wait d2"); return { body: { signature: "sig-d2", status: "confirmed" } }; },
  });
  const api = new FuturesClient({ baseUrl: "http://api.test", fetch });
  const signedIds = [];
  const sent = await api.send(ownerKey.publicKey, "submit", { market: "RAMX", side: "long", lo: 86, hi: 104, lots: 3, limit: 180_000n },
    (tx, p) => { signedIds.push(p.id); tx.partialSign(ownerKey); return tx; });
  assert.deepEqual(signedIds, ["d1", "d2", "main"]);
  assert.deepEqual(order, ["submit d1", "wait d1", "submit d2", "wait d2", "submit main"]);
  assert.deepEqual(sent.before, ["sig-d1", "sig-d2"]);
  assert.equal(sent.signature, "sig-main");

  // A part that does not land stops before the main transaction.
  const stuck = mockFetch({
    "POST /v1/tx/prepare": prepared,
    "POST /v1/tx/submit": (call) => ({ body: { signature: `sig-${call.body.id}` } }),
    "GET /v1/tx/sig-d1/wait": () => ({ body: { signature: "sig-d1", status: "failed" } }),
  });
  await assert.rejects(new FuturesClient({ baseUrl: "http://api.test", fetch: stuck.fetch }).send(ownerKey.publicKey, "submit",
    { market: "RAMX", side: "long", lo: 86, hi: 104, lots: 3, limit: 180_000n }, (tx) => { tx.partialSign(ownerKey); return tx; }),
  /did not land: failed/u);
  assert.equal(stuck.calls.filter((c) => c.path === "/v1/tx/submit").length, 1);
});

test("the wallet's classic and compressed USDC have their own reads", async () => {
  const w = "8gtNpuavjwVMjXkf7yVC1yypWTPbXoiK7HjwBfeiYrEx";
  const { fetch, calls } = mockFetch({
    [`GET /v1/wallets/${w}/usdc`]: { owner: w, ata: "7FXo3kwfoTqWEHCDo8SFtFNm1SLH6Qk5SFXoK1ynuAuG", usdcAtoms: "352102", decimals: 6, slot: 1 },
    [`GET /v1/wallets/${w}/compressed-usdc`]: { owner: w, lightAta: "92jqfCtcx541Vvq2V6skWtf6PsBwPYGQGe1xtV2nVSoZ",
      usdcAtoms: "18065063", lightAtaAtoms: "0", compressedAtoms: "18065063", decimals: 6, slot: 2 },
  });
  const api = new FuturesClient({ baseUrl: "http://api.test", fetch });
  assert.equal((await api.walletUsdc(w)).usdcAtoms, "352102");
  assert.equal((await api.walletCompressedUsdc(w)).compressedAtoms, "18065063");
  assert.deepEqual(calls.map((c) => c.path), [`/v1/wallets/${w}/usdc`, `/v1/wallets/${w}/compressed-usdc`]);
});
