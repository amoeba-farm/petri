# Ameba futures API (dev)

This is the source of truth for the HTTP API that the frontend, SDK and CLI
use for Ameba's futures (the program contract is
[`futures/docs/interfaces.md`](../futures/docs/interfaces.md)). It is kept in
sync with [`api/`](api) (binary `ameba-futures-api`). **[added]** and
**[adjusted]** mark where the service grew past or changed its first
contract.

The service runs on `127.0.0.1:4500`. The browser reaches it on the same
origin through the frontend at `/api/futures/<path>`, which the Next.js
rewrite sends to `http://127.0.0.1:4500/<path>`.

## Conventions

* **Amounts** are strings of integer atoms. USDC has 6 decimals, so
  `"10000000000"` is 10,000 USDC. Signed amounts such as `premium`, `pay`
  and `pnlAtIndex` may start with `-`.
* **`*Value`** fields are index values as decimal strings with no trailing
  zeros, for example `"100"` or `"100.2"` (see "Futures", Units).
* **`*Usd`** fields are decimal strings.
* **Numbers [adjusted].** Grid positions, lots, counts, slots, indexes and
  timestamps are JSON numbers. Lots per record are at most 2^40, so they are
  exact in JavaScript. Premiums, sequences and cumulative atom totals are
  strings.
* **Times [adjusted].** Times are unix seconds of the *chain* clock (the
  Clock sysvar), not wall time. That is the clock the programs use for
  period tradability and expiry. On localnet it can lag wall time by hours.
  `MarketView.chainTime` and `/health` (`chainTime`, `wallTime`) expose
  it, so the UI can compute "time to expiry" and "24h" windows consistently.
  Event times (`ts` of trades and history, `t` of candles and index points)
  are shown in wall time: when the service saw the event live, else its
  block time shifted by the clock offset (wall minus chain, to the minute).
* **Sides.** In trades a side is `"buy"` or `"sell"` (the taker's: buy =
  long). Orders, claims and ladders use `"long"` or `"short"`. Requests
  accept `"buy"`, `"sell"`, `"long"`, `"short"`, `"bid"`, `"ask"`, `0` or `1`.
* **Errors** return a non-2xx status with
  `{ "error": string, "logs"?: string[] }`:

  | Status | Meaning |
  |---|---|
  | 400 | Bad input, or the program would refuse it (`"<name>: <reason>"`, for example `"NoLiquidity: …"`) |
  | 401 | A signature doesn't verify (bot hub, oracle observations) **[added]** |
  | 403 | Refused to sign, or a reserved owner |
  | 404 | Unknown market, or the faucet is disabled |
  | 409 | The bot id is another author's (`POST /v1/bots`) **[added 2026-10-26]** |
  | 410 | The prepared id is unknown, expired or already used |
  | 413 | The body or bundle is past its protection bound **[added]** |
  | 422 | The node rejected a submitted transaction outright; `error` is `"<step>: <error name>"` and `logs` is included |
  | 429 | Rate limited; a `Retry-After` header is set |
  | 500 | Internal error, for example the sponsor failed to sign |
  | 502 | RPC failure |
  | 503 | The database or market state is not ready yet |

  Program errors are named: the futures program's (base `0x46520000`:
  `InsufficientFunds`, `MinFill`, `WouldCross`, `NotBestPrice`,
  `ClaimReserved`, …), the governance gate's (`GovernanceGateFrozen`,
  `GovernanceGateEpochMismatch`, …) and the token program's
  (`TokenInsufficientFunds` and so on).

## Renamed from range (2026-10-15) [adjusted]

Range-composed futures are the only futures product, so nothing in the API
is named for "range" any more; the word is left only for a claim's price
interval (`lo`, `hi`, the ticket's default range). The old names are not
accepted: the old routes are 404, the old actions 400 `unknown action`, and
a manifest `range` section is ignored.

| Was | Now |
|---|---|
| `/v1/range/markets…` | `/v1/markets…` |
| `/v1/range/accounts/:owner` | `/v1/accounts/:owner` (`/v1/accounts/:owner/history` is unchanged) |
| `/v1/range/quote` | `/v1/quote` |
| actions `rangeDeposit`, `rangeWithdraw`, `rangeSubmit`, `rangeClose`, `rangeCancel`, `rangePoolDeposit`, `rangePoolWithdraw`, `rangeFinalize`, `rangeSettle` | `deposit`, `withdraw`, `submit`, `close`, `cancel`, `poolDeposit`, `poolWithdraw`, `finalize`, `settle` (`cashOut` is unchanged) |
| event types (history, `/v1/tx/:signature` and `/wait` `events`) `rangeDeposit`, `rangeWithdraw`, `rangeTake`, `rangeFill`, `rangeLadderFill`, `rangePlace`, `rangeCancel`, `rangeQuote`, `rangeRedeem`, `rangeTransfer`, `rangeFinalize`, `rangeSettle`, `rangeSettleMaker`, `rangeMarket`, `rangePool`, `rangePoolDeposit`, `rangePoolWithdraw` | `deposit`, `withdraw`, `take`, `fill`, `ladderFill`, `place`, `cancel`, `quote`, `redeem`, `transfer`, `finalize`, `settle`, `settleMaker`, `market`, `pool`, `poolDeposit`, `poolWithdraw`; the stored history takes the new names too |
| stream events `rangeBook`, `rangeTrade`, `rangeAccount` | `book`, `trade`, `account` |
| `/v1/config` `programs.range`, `rangeTreasury` | `programs.futures`, `treasury` |
| `/health` `rangeMarkets`, `retiredRangeMarkets`, `logs.range` | gone (the same as `markets`), `retiredMarkets`, `logs.futures` |
| `RangeMarketView` | `MarketView` |
| manifest section `range` (`program`, `treasury`, `treasuryFunding`, `markets`), `accounts.rangeTreasury` | `futures` (the same fields), `accounts.treasury` |
| tables `futures.range_events`, `range_trades`, `range_cursors`, `range_markets` and their indexes; `futures.meta` key `range_decoder_version` | `futures.events`, `trades`, `cursors`, `markets`; `futures_decoder_version` (not `decoder_version`, the retired clearing indexer's key): renamed in place at startup, every row kept (see "Events and indexing") |

The program's on-chain identity does not change: its id, account magics,
PDA seeds (`"ameba-range-v1"`, `"pool"`, `"lp"`, …), event tags (`AR*`) and
error codes (base `0x46520000`).

## Reads

### `GET /v1/config`

```json
{
  "cluster": "localnet",
  "rpcUrl": "http://127.0.0.1:8899",
  "wsUrl": "ws://127.0.0.1:8900",
  "genesisHash": "…",
  "programs": { "oracle": "…", "futures": "…" },
  "treasury": "…",
  "usdcMint": "GSUW4ohqFyJnJvC7GiQPwsseFEYXY6uLkPjYUYQNRmbZ",
  "usdcDecimals": 6,
  "sponsor": "…",
  "faucet": { "enabled": true, "amount": "10000000000", "perHour": 3 },
  "computeUnitLimit": 1400000,
  "preparedTtlSecs": 90,
  "sponsorship": { "researcherBps": 2000, "retentionBps": 2000,
                   "lookupTables": { "RAMX-1796083200": "<address lookup table>" } },
  "evidence": { "webProof": {
    "notary": "/oracle/notary",
    "policy": "amoeba-webproof-verifier-policy-v1/dev",
    "policyDigest": "…64 hex…",
    "notaryKeys": [{ "name": "…", "alg": "secp256k1", "key": "…66 hex…", "trust": "development" }],
    "backendRevision": "5f67389de3f8962227fddffd71d78a852acc9b01" } }
}
```

`faucet` is there only in a dev build (the `dev-faucet` feature, which
`ops/install.sh` enables): a production build has no faucet and no `faucet`
key.

`rpcUrl` and `wsUrl` are the deployment manifest's public endpoints, `null`
when the manifest names none. The service's own `FUTURES_RPC_URL` and
`FUTURES_WS_URL` (which may carry a provider key) are never served.

**[adjusted]** `programs` names only the futures program and the oracle: the
retired clearing and matcher programs are gone (no `clearing` or `matcher`
keys).

**[added]** `oracleReleaseTag` reports the oracle manifest release profile (or `unreleased`).

**[added]** `governanceController` is the optional manifest object `{ program, targets }`: a pinned controller ID and explicit gated program IDs. Any deployed futures program must be listed (and the oracle program of a governed oracle build; the dev oracle, release `localnet-dev`, may be omitted).
When present, transaction preparation binds every futures instruction to the
futures program's cached gate epoch (its gate account and signed suffix,
interfaces.md §6). Missing, malformed or frozen gates refuse preparation. Gate
accounts use websocket subscriptions and periodic cache resync; preparation
adds no gate RPC calls. Startup checks the actual RPC genesis once; a custom
development controller is rejected on any other chain. An absent field
preserves the ungated transaction encoding for development deployments;
governed programs always reject that encoding.

**[added]** `wsUrl`, `genesisHash`, `faucet.perHour`, `computeUnitLimit`
and `preparedTtlSecs`.

**[added]** `sponsorship`: what a sponsored find's bounty pays its
researcher (`researcherBps`) and how much of that their reserve holds
(`retentionBps`); the sponsor gets the rest. 2,000 and 2,000 (the sponsor
80%, the researcher 16% and their reserve 4%) on every cluster but
`mainnet-beta`, whose deployed oracle predates that split and still takes
the researcher's half (5,000 and 2,000) until it is upgraded.
`FUTURES_SPONSOR_RESEARCHER_BPS` (2,000 or 5,000) overrides the cluster's
share: Mainnet's API sets 2,000 once its oracle runs the 80/20 build, and
petri (from v0.1.16) and the web follow this field.
**[added 2026-10-21]** `sponsorship.lookupTables`: each month's sponsorship
address lookup table, by series key (`<SYMBOL>-<expiry>`), as the manifest's
`oracle.sponsorLookupTables` names it (`{}` without any). A prepared
sponsorship's activation resolves through it (v0); petri sends the same.

**[added 2026-10-20]** `evidence.webProof`: how to make web proofs this
service accepts ("Oracle evidence"). `notary` is the WebSocket path to the
API host's notary; `policy` and `policyDigest` name the verifier policy the
service checks bundles under (`amoeba-webproof-verifier-policy-v1/dev` when
the deployment is the dev chain, genesis
`A5ZLaudxEFvFScV6RzNtNN7fzshRxQgXhC6JB6dishHX` in the manifest and at the
RPC, else `…/production`); `notaryKeys` are the notaries it lists, each
`trust` `production` or `development` (compressed secp256k1 keys, hex; an
empty list means no proof verifies yet); `backendRevision` is the TLSNotary
commit prover and notary must run.

**[added]** `programs.futures` (the futures program) and `treasury` (its
dev maker's trader key) are `null` until the deployment manifest has a
`futures` section (see "Futures").

**[adjusted]** `programs.oracle` is the oracle program (the manifest's
`oracle.program`) once the deployment manifest has an `oracle` section, and
until then the manifest's `programs.oracle`. It always equals
`protocol.programId` of the `/v1/oracle/*` documents.

#### The deployment manifest

The service reads `FUTURES_DEPLOYMENT` (`futures/.localnet/deployment.json`
on dev): `cluster`, `rpcUrl`, `wsUrl`, `genesisHash`, `programs.oracle`,
`usdcMint`, `usdcDecimals`, `governanceController`, `oracle` (see "Oracle"),
`futures` (see "Futures"), `accounts` (reserved owners),
`keys.sponsor` (a key file path or a Cloud KMS key version; see "The
sponsor's key") and `keys.mintAuthority` (a key file path, dev builds only).
Any other field is ignored, present or absent: the retired clearing
deployment's `programs.clearing`, `programs.matcher`, `config`, `markets`,
artifacts and maker keys among them.

#### The sponsor's key [added]

The sponsor (`sponsor` above) pays the fee of every sponsored transaction
and is the only key that co-signs them (see submit). Its key is a Solana
keypair file, as on dev, or a Google Cloud KMS key, as in production:

| Setting | Default | What |
|---|---|---|
| `FUTURES_SPONSOR_SIGNER` | the manifest's `keys.sponsor` | `kms:projects/<project>/locations/<location>/keyRings/<ring>/cryptoKeys/<key>/cryptoKeyVersions/<n>`, or a keypair file's path (`file:` optional). `keys.sponsor` takes the same values, and its relative paths are the manifest folder's. |
| `FUTURES_SPONSOR_PUBKEY` | the manifest's `accounts.sponsor` | The sponsor address a KMS key must be. A KMS signer needs one of the two; when both are set they must agree. |
| `FUTURES_ALLOW_FILE_SIGNER_ON_MAINNET` | off | `1` lets a keypair file sign on mainnet. |
| `GOOGLE_APPLICATION_CREDENTIALS` | unset | Google's Application Default Credentials file (below). |

With a KMS key, the service does two things at start, and doesn't start if
either fails:

1. **It gets an access token** from Application Default Credentials, in
   Google's order:
   * the JSON file `GOOGLE_APPLICATION_CREDENTIALS` names (a service account
     key or gcloud user credentials);
   * gcloud's `~/.config/gcloud/application_default_credentials.json`
     (`gcloud auth application-default login`);
   * the metadata server of the Google Cloud host it runs on (Compute
     Engine, Cloud Run, GKE). A Compute Engine VM needs the
     `cloud-platform` access scope.

   Workload identity federation and impersonation files are refused.
   Credentials are never in the repository or the manifest.
2. **It reads the key version's public key** (`getPublicKey`). The version
   must be `EC_SIGN_ED25519` with software or HSM protection, and its key
   must be the configured sponsor address. A disabled or destroyed version
   fails here.

Each co-signature is then one `asymmetricSign` request over the exact
message bytes. KMS returns a raw 64-byte Ed25519 signature, and the service
verifies it against the sponsor's address before it sends anything (a
keypair file's signatures are checked the same way). If signing fails or the
signature doesn't verify, submit answers 500 `sponsor signing failed` and
sends nothing; the cause is in the service log. Nothing is retried. A
request to Google may take up to 10 s, the old backend's limit. A token is
reused until a minute before it expires.

The service refuses to start with a keypair-file sponsor on mainnet, unless
`FUTURES_ALLOW_FILE_SIGNER_ON_MAINNET=1`. It treats the deployment as
mainnet when any of these holds:

* the manifest's `cluster` is `mainnet-beta` (or `mainnet`);
* the manifest's `genesisHash` is mainnet's
  (`5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d`);
* the RPC node's genesis is mainnet's.

The dev faucet's mint authority (`keys.mintAuthority`) is always a keypair
file. Only a dev build (`dev-faucet`) loads it.

To point production at a KMS key version:

1. In Cloud KMS, create an asymmetric signing key with the purpose
   `ASYMMETRIC_SIGN` and the algorithm `EC_SIGN_ED25519`, protection
   `SOFTWARE` or `HSM`.
2. Its Solana address is the base58 of the last 32 bytes of the public
   key's DER:

   ```bash
   gcloud kms keys versions get-public-key <n> --project <project> \
     --location <location> --keyring <ring> --key <key> --output-file sponsor.pem
   openssl pkey -pubin -in sponsor.pem -outform DER | tail -c 32 | python3 -I -c '
   import sys; b = sys.stdin.buffer.read(); n = int.from_bytes(b, "big"); s = ""
   A = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"
   while n: n, r = divmod(n, 58); s = A[r] + s
   print("1" * (len(b) - len(b.lstrip(b"\0"))) + s)'
   ```

   Put it in the manifest's `accounts.sponsor` (or `FUTURES_SPONSOR_PUBKEY`)
   and fund it with SOL for fees and rent.
3. Grant the service's identity `roles/cloudkms.signerVerifier` on that key.
   The role holds `cloudkms.cryptoKeyVersions.useToSign` and
   `…viewPublicKey`, which is all the service needs.
4. Start the service with
   `FUTURES_SPONSOR_SIGNER=kms:projects/<project>/locations/<location>/keyRings/<ring>/cryptoKeys/<key>/cryptoKeyVersions/<n>`
   (or put the same value in `keys.sponsor`). The log line
   `sponsor <address> (Cloud KMS …)` confirms it.

A new key version is a new address. To rotate, fund the new address, then
change both settings together.

### `GET /v1/accounts/:owner/history?limit=100&before=<signature>&types=take,fill`

The owner's indexed market events, newest first, across every market (the
manifest's and the retired ones). An event is included when the owner is
its primary party or its counterparty (the taker of a resting order's fill,
the recipient of a transfer, the pool of an Earn pool deposit or
withdrawal).

```text
{ owner, items: [{ signature, eventIndex, slot, ts, instrument (the market), symbol, label,
                   type, role ("maker"|"taker" for fill and ladderFill, "out"|"in" for transfer, else null),
                   data }] }
```

* `type` and the fields of `data` are those of "Futures", Events and
  indexing (`deposit`, `withdraw`, `take`, `fill`, `ladderFill`, `place`,
  `cancel`, `quote`, `redeem`, `transfer`, `finalize`, `settle`,
  `settleMaker`, `market`, `pool`, `poolDeposit`, `poolWithdraw`). Events
  with an interval get `loValue`/`hiValue` (and fills `entryValue`).
* `instrument` is the market's address; `symbol` and `label` are its
  own (a retired market's from the registry), `null` for a market the
  service does not know.
* `types` filters by a comma-separated list. All types are returned by
  default.
* `before` pages to events older than that transaction. `limit` is clamped
  to 1..1000.
* **[adjusted]** Only the futures program's events are listed: the
  retired clearing markets' events are gone. **[adjusted]** The types lost
  their `range` prefix (see "Renamed from range"), in the stored history
  as well.

## Fast trading path (summary) [added]

The service keeps every account a quote or prepare needs in memory. The
cache is fed by websocket subscriptions at `processed` commitment, and the
blockhash is refreshed every 400 ms while it is in use (every 5 s after a
quiet minute, and at once on the next use). As a result, quotes and prepares make no
RPC calls and never simulate. Submission skips preflight and is rebroadcast
until it lands. The UI flow should be:

1. While the user edits the ticket, call `POST /v1/tx/quote-prepare` with
   the order (on change, or about every 1 to 2 s). Each response carries
   the quote and a ready-to-sign `submit`.
2. On click, sign the latest `prepared.transaction` and
   `POST /v1/tx/submit` it. The response returns the signature at once.
3. `GET /v1/tx/:sig/wait?commitment=processed` resolves when the
   transaction lands, which is the moment to tell the user. Then
   `…/wait?commitment=confirmed` returns its events. You can instead
   listen to `/v1/stream?owner=` for `tx` events.

Each response carries `Server-Timing: app;dur=<ms>` with the service's own
time. `ops/latency.py` times every stage of both flows (`fast`: the above;
`classic`: quote, prepare, sign, submit, poll).

### `GET /v1/wallets/:owner/usdc` [added]

```json
{ "owner": "G493x1FY…", "ata": "7WfSP3Hc…", "usdcAtoms": "10000000000", "decimals": 6, "slot": 6125806 }
```

The owner's classic SPL token account (ATA) of the dev USDC mint. `usdcAtoms`
is `"0"` when the ATA does not exist. The first request for an owner reads
the ATA once and subscribes to it; later reads come from the account cache
(`processed`), and `slot` is the cache's slot. Any valid pubkey is accepted.
For live updates use the `wallet` event of `/v1/stream?owner=`.

### `GET /v1/wallets/:owner/compressed-usdc` [added]

```json
{ "owner": "8gtNpuav…", "lightAta": "92jqfCtc…", "usdcAtoms": "18065063", "lightAtaAtoms": "0",
  "compressedAtoms": "18065063", "decimals": 6, "slot": 457603226 }
```

The owner's compressed USDC: the Light Protocol tokens of the USDC mint that
wallets funded by the earlier product still hold, beside their ATA. It is a
separate read from `/v1/wallets/:owner/usdc` (which it never delays), and
the sum of two forms:

* `lightAtaAtoms`: the owner's Light token ATA, the compressed token
  program's (`cTokenmWW8bLPjZEBAUgYy3zKxQZW6VKi7bqNFEVv3m`) account at
  `[owner, program, mint]`, read and then followed like the ATA (`"0"` when
  it does not exist);
* `compressedAtoms`: their compressed token accounts, from Photon
  (`getCompressedTokenBalancesByOwnerV2` on `FUTURES_PHOTON_URL`). A reading
  answers for 15 s. `null` when Photon cannot be read; `usdcAtoms` is then
  the Light ATA's alone.

`usdcAtoms` is their sum and `slot` the later of the cache's and Photon's.
The futures program spends only the ATA, so `cashOut` and the market orders
move these tokens out first when the ATA falls short (see "Spending
compressed USDC"). For live updates use the `walletCompressed` event of
`/v1/stream?owner=`.

### Spending compressed USDC [added]

`cashOut` and the orders that deposit (`submit`, `close`) spend the wallet's
compressed USDC when its ATA holds less than they need. The ATA pays first;
the shortfall comes from the Light ATA, then from the fewest compressed
token accounts that cover it (largest first). They are moved with the
compressed token program's `Transfer2`, which the owner signs like the rest:

* `decompress`: up to 8 compressed token accounts in one instruction, with
  their validity proof (`getCompressedTokenAccountsByOwnerV2` and
  `getValidityProofV2` on `FUTURES_PHOTON_URL`), decompressing exactly the
  amount needed from the mint's SPL pool into a classic USDC account; what
  they hold beyond it stays the owner's as one compressed account.
* `decompressLightAta`: moves the amount out of the Light ATA into a classic
  USDC account through the same pool.

`cashOut` decompresses straight into the destination's ATA; an order
decompresses into the owner's ATA ahead of its `deposit`. These steps go in
the prepared transaction itself when everything fits (1232 bytes). Otherwise
each batch of accounts gets a transaction of its own, listed in order in
the prepare response's `before`: each is a full prepare response
(`summary.action` `"decompress"`, with `destinationAta` and the `amount` it
moves). Sign them with the main transaction, then submit them in order,
waiting until each lands (`/v1/tx/:sig/wait?commitment=processed`), before
submitting the main one. The first one also creates the account it fills
when that does not exist yet. Each is co-signed under the usual rules.
**[added 2026-10-19]** Owner-paid (`payer: "owner"`), each is owner-paid
too and the owner sends them in the same order (see "Owner-paid
transactions").

Photon is read only on these paths and only when the ATA falls short: an
order the ATA covers makes no compressed read, and an owner whose last
compressed reading was 0 is not read again to price one. After a
transaction that spent compressed USDC lands, the owner's leaves are read
again once Photon has indexed it, and `walletCompressed` follows.

## Transactions (sponsored)

### `POST /v1/tx/quote-prepare` [added]

Request: the `/v1/quote` body plus `owner` (required) and the order's
`mode`, `minFill` and `withdraw` (see "Futures"), and **[added
2026-10-19]** `payer` (`"sponsor"`, the default, or `"owner"`) beside them
at the top level of the body: the order's fields are quote-prepare's
`params`.

```json
{ "market": "RAMX", "owner": "…", "side": "long", "lo": 98, "hi": 120, "lots": 1, "payer": "owner" }
```

Response:

```text
{ quote: <the /v1/quote response>, prepared: null | <the /v1/tx/prepare response for action "submit"> }
```

* `prepared` is the `submit` for this quote (`[deposit]`, `submit`,
  `[withdraw]`); **[added 2026-10-19]** with `payer: "owner"` it is the
  owner-paid prepare response (no `id`; see "Owner-paid transactions").
  It is null when `quote.error` is set, and **[added]** when
  the order needs the owner's compressed USDC (`quote.decompress` is set):
  `/v1/tx/prepare` builds that one (see "Spending compressed USDC").
* A retired market (see "Retired markets") is refused with 400
  `RetiredMarket`.
* **[adjusted]** A body without `market` (the retired clearing markets'
  `instrument` takes) is refused with 400.
* Refresh it while the user edits. Only the latest unsigned prepare per
  owner matters; every prepared message stays usable until it expires.
* The blockhash is at most about 400 ms old (5 s for the first prepare
  after a quiet minute), so a prepared transaction stays valid for roughly
  60 s.

### `POST /v1/tx/prepare`

Request:

```json
{ "owner": "…", "action": "submit", "params": { … } }
```

Response:

```text
{ id, transaction (base64, fee payer = sponsor, owner signature empty), blockhash, lastValidBlockHeight, summary,
  before? }
```

**[added]** `before`, present only when compressed USDC is spent in
transactions of their own: the prepare responses of the transactions that
must land first, in order (see "Spending compressed USDC").

* **[adjusted]** Prepare makes no RPC call. The market account, its
  period, its legacy Earn pool, the Earn vault, the owner's positions and
  USDC ATA come from the in-memory cache. The blockhash is the cached latest confirmed one,
  refreshed every 400 ms while in use (every 5 s when idle). The first request for a never-seen owner reads
  that owner's ATA once and then subscribes to it.
  `GET /v1/accounts/:owner` warms this too.
* `transaction` is a bincode-serialized legacy `Transaction` with every
  signature zeroed.
* The owner signs the message (`transaction.message.serialize()`), for
  example with `wallet.signTransaction`. The service adds the sponsor
  signature at submit time.
* Every transaction starts with `SetComputeUnitLimit(1_400_000)`
  (**[added 2026-10-20]** `earnClaim`'s with `SetComputeUnitLimit(200_000)`,
  see "Earn transactions").
* **[adjusted 2026-10-19]** Every action takes `params.payer: "owner"`
  (`"sponsor"` or none: sponsored, as above), and is then built for the
  owner to pay and send instead (see "Owner-paid transactions").
* `summary` is
  `{ action, owner, feePayer, instructions: [step names], expiresAt, …action fields }`.
  The step names are `computeBudget`, `heapFrame`, `createAta`, `deposit`,
  `withdraw`, `submit`, `cancel`, `poolWithdraw`, `finalize`, `settle`,
  `settleMaker`, `prune`, `vaultDeposit`, `vaultWithdraw`,
  **[added 2026-10-20]** `createAmbaAta` and `vaultRewardsClaim`
  (`earnClaim`), for `cashOut` `createDestinationAta` and `cashOut`, and **[added]**
  `decompress` and `decompressLightAta` (see "Spending compressed USDC"). Market actions' summaries
  carry `market`, `symbol` and `label`; `cashOut`'s and the Earn actions'
  do not (the Earn actions' carry `vault`).

Actions and their `params`:

| action | params | instructions |
|---|---|---|
| `deposit`, `withdraw`, `submit`, `close`, `cancel`, `poolWithdraw`, `finalize`, `settle` **[added]** | `market, …` | see "Futures", Market transactions |
| `earnDeposit`, `earnWithdraw` **[added]** | `amount` (no `market`) | see "Earn" |
| `earnClaim` **[added 2026-10-20]** | none | see "Earn" |
| `stake`, `startUnstake`, `cancelUnstake`, `unstake`, `stakeClaim` **[added]** | `amount` (`startUnstake`, `unstake`: or `"all"`/omitted; `cancelUnstake`, `stakeClaim`: none) | see "Stake" |
| `cashOut` **[added]** | `destination, amount` (no `market`) | [createDestinationAta], [decompressLightAta], [decompress], [cashOut]: a `TransferChecked` of up to `amount` USDC atoms from the owner's ATA to the destination's ATA, the rest decompressed from the owner's compressed USDC into it |

* **[adjusted]** The market actions lost their `range` prefix (see
  "Renamed from range"); the old names are refused with 400 `unknown
  action`. So are the retired clearing markets' actions without a futures
  counterpart (`take`, `place`, `exit`, `transfer`, `lpDeposit`,
  `lpWithdraw`, `lpRedeem`, `lpClaimFees`, `withdrawAll`); `deposit`,
  `withdraw` and `cancel` are the futures actions and need `market`.
  `poolDeposit` went with the per-market Earn pools (2026-10-16): it is
  refused with 400 `unknown action`; Earn is `earnDeposit`.
* Steps in brackets are added only when needed. `createAta` is an
  idempotent creation of the owner's USDC ATA, paid by the sponsor (the
  owner, owner-paid), added when the ATA does not exist.
* **[added]** `cashOut` sends USDC from the owner's wallet to another
  wallet. The sponsor (the owner, owner-paid) pays the fee and, when the
  destination's ATA does not exist yet, its rent (`createDestinationAta`,
  an idempotent creation).
  **[adjusted]** Past the ATA's balance, the owner's compressed USDC pays
  the rest, decompressed straight into the destination's ATA (see
  "Spending compressed USDC"); without a part from the ATA there is no
  `cashOut` step. It is refused with 400 when:
  * `amount` is not a canonical decimal string of atoms (digits only, no
    leading zero, at most 20 digits), is `"0"`, or exceeds the owner's ATA
    balance (from the account cache) plus their compressed USDC;
  * `destination` is not a canonical base58 address on the ed25519 curve
    (program-derived addresses are refused), is the default key or the USDC
    mint, is the owner or the sponsor, or is an account that is not a plain
    system account;
  * the destination's ATA exists but is not an initialized USDC account of
    the destination;
  * the owner is not a plain system account.

  The summary is `{ destination, destinationAta, createDestinationAta, usdcAta,
  amount, balance, usdcAmount, compressedAmount, transactionAmount, … }`:
  `balance` is the ATA's, `usdcAmount` the part it pays, `compressedAmount`
  the part compressed USDC pays and `transactionAmount` what this
  transaction moves (the rest moves in `before`). Submit it with
  `/v1/tx/submit` as usual (owner-paid: the owner sends it).
* A transaction larger than 1232 bytes is refused with 400.

### Owner-paid transactions [added 2026-10-19]

Wallets such as Phantom, Solflare and Backpack change the transactions
they sign (Lighthouse assertions, priority fees), so a sponsored one fails
("Your wallet changed the transaction while signing, so it was not sent").
The user decided (2026-10-19, "Unsponsored staking now", then for every
action) that wallet-adapter wallets pay for and send their own
transactions; Magic (Google) wallets stay sponsored. Every `/v1/tx/prepare`
action (the market, Earn and stake actions and `cashOut`) takes
`params.payer`, and `/v1/tx/quote-prepare` takes `payer` at the top level
of its body, beside the order's fields:

* `"sponsor"` or none: the sponsored transaction, unchanged.
* `"owner"`: owner-paid, below.
* Anything else: 400 `` `payer` must be "owner" or "sponsor" ``.

With `"payer": "owner"` the instructions are the sponsored ones with the
owner in the sponsor's place:

* the owner is the fee payer and the only signer of the transaction and of
  every `before` one, and pays every rent the sponsor would: `createAta`,
  `createAmbaAta`, `createDestinationAta` (`cashOut`), the Earn position
  (`earnDeposit`'s first), the stake position and its growth (`stake`,
  `startUnstake`), and, as the payer named by `decompress` and
  `decompressLightAta`, any rent top-up the Light ATA asks for;
* the sponsor is not among any of the transactions' accounts;
* the compute limit is the sponsored one, `SetComputeUnitLimit(1_400_000)`,
  except for the stake actions: `SetComputeUnitLimit(200_000)` (a stake
  measured about 32,000 CU on mainnet; a wallet's priority fee is priced
  per unit of the limit, so the lower limit keeps it small), and
  **[added 2026-10-20]** `earnClaim`, whose limit is 200,000 either way. The
  API adds no `SetComputeUnitPrice`;
* with one signer fewer, an owner-paid transaction is at least 96 bytes
  smaller than the sponsored one of the same steps, so the API splits off
  `before` transactions exactly where it would for the sponsored one.

Response:

```text
{ transaction (base64 legacy transaction, unsigned), blockhash, lastValidBlockHeight, summary, before? }
```

Each `before` entry is an owner-paid response of its own (`summary.action`
`"decompress"`); quote-prepare's `prepared` is the owner-paid response for
`submit`. No `id` and nothing stored: the sponsor never signs them and
`/v1/tx/submit` does not take them. The owner's wallet signs them, may
change them, and sends them itself (for example `sendTransaction` of the
wallet adapter, or signed and relayed through `/rpc/sendTransaction`, which
refuses only sponsor-paid packets): the `before` ones first, in order, each
landed (`/v1/tx/:sig/wait?commitment=processed`) before the next. Follow
each with `/v1/tx/:sig/wait`. `summary` is the sponsored one's with
`feePayer` the owner and no `expiresAt` (the blockhash's
`lastValidBlockHeight` is the limit). Compressed USDC an owner-paid
transaction spends counts as spent once Photon indexes it (a sponsored
submit counts it as soon as it is confirmed), so prepare a further spend
only after the earlier one has landed.

### `POST /v1/tx/submit`

Request:

```json
{ "id": "…", "transaction": "base64 (owner-signed)" }
```

Response:

```json
{ "signature": "…", "lastValidBlockHeight": 6041234 }
```

The service co-signs only when all of these hold:

* The id was prepared less than 90 s ago and has not been used.
* The message bytes are identical to the prepared ones, or the message is
  the prepared one with only wallet guard instructions added (below).
* The fee payer is the sponsor.
* Every required signature other than the sponsor's verifies over the
  submitted message, including the owner's.

**Wallet guard instructions.** Phantom inserts Lighthouse assertions
(program `L2TExMFKdjpN9kozasaurPirfHy9P8sbXoAN1qA3S95`) into transactions
that change token balances, and re-compiles the message. Such a legacy
message is accepted when: the blockhash and the set and number of required
signers are the prepared ones, with the sponsor as account 0; every prepared
account keeps its signer and writable status, and any added account is
read-only and unsigned; the prepared instructions are all there, in order,
with the same program, account metas and data; every added instruction is a
Lighthouse one given no signer (it may name an account the prepared
instructions already write); and the transaction fits 1232 bytes. So no
compute budget instruction (a priority fee) or any other program can be
added. The sponsor then signs the submitted message. In `/v1/tx/:sig` and
`wait`, an added instruction's step label is `walletGuard`; the others keep
their prepared labels. Any other change is refused (403, "the transaction
message differs from the prepared one"), and the service logs the program
ids of the instructions that differ.

**[adjusted]** It then signs as the sponsor and sends once with
`skipPreflight: true` and `maxRetries: 0`, returning as soon as the node
accepts it.

* A tracker resends it every 300 ms until it is seen (processed) or its
  blockhash expires.
* Failures do not come back from submit. They surface through
  `/v1/tx/:sig`, `/v1/tx/:sig/wait` and the owner's SSE `tx` events, with
  the decoded error name. 422 means only that the node rejected the
  transaction outright.
* Each id can be submitted once.

### `GET /v1/tx/:signature/wait?commitment=processed|confirmed&timeoutMs=30000` [added]

Resolves as soon as the signature reaches `commitment`, fails, or its
blockhash expires. It also resolves when `timeoutMs` passes (default 30000,
max 120000).

```text
{ signature, status ("pending"|"processed"|"confirmed"|"failed"|"expired"),
  slot, processedMs, confirmedMs (since submit), err? (raw TransactionError), error? (decoded name),
  failedInstruction?, events? (decoded events), logs?,
  timedOut (bool), waitedMs }
```

* The tracker follows each submitted signature with a batched
  `getSignatureStatuses` against the local RPC every 20 ms.
* The futures program's processed log notifications (`logsSubscribe`)
  resolve `processed` waits at once with the events decoded from the logs (each
  with `type` and the prepared `market`), without fetching the transaction.
* At confirmed it stores those events at once (fetching the transaction
  once when no notification came), so `/v1/markets/:market/trades`,
  `/v1/accounts/:owner/history` and the SSE `trade` events need not wait
  for the indexer. A transaction that does not call the futures program (a
  `cashOut`) gets no logs notification, so its confirmed result comes
  from one `getTransaction` right away.
* A failed transaction resolves `processed` waits immediately, with `error`
  (for example `"MinFill"`) and `failedInstruction` (the step's name, for
  example `"submit"`). `confirmed` waits on a failure resolve once its `logs`
  are available.
* Unknown signatures, such as ones submitted elsewhere, are followed from
  the first wait, without rebroadcast.

### `GET /v1/tx/:signature`

```text
{ signature, status ("pending"|"confirmed"|"finalized"|"failed"), slot?, error?, blockTime? (number|null), events?, logs? }
```

* Once the transaction has landed, `events` lists the decoded events (each
  with `type` and `market`, as in history) and `logs` is set.
* **[added]** For signatures submitted through this service, the answer
  comes from the tracker:
  * `status` keeps the values above: processed counts as `pending`, and
    expired counts as `failed` with `error: "BlockhashExpired"`.
  * `commitment` (`pending`, `processed`, `confirmed`, `failed` or
    `expired`), `processedMs` and `confirmedMs` are added; `slot`,
    `processedMs` and `confirmedMs` are `null` until known.
  * A failure also carries `err` (the raw transaction error, once landed)
    and, with `error`, `failedInstruction` (the step's name, or `null`).

### `POST /v1/faucet` (dev only)

Request:

```json
{ "owner": "…" }
```

Response:

```json
{ "signature": "…", "amount": "10000000000", "usdcAta": "…" }
```

* Mints test USDC from the dev mint authority to the owner's ATA, creating
  the ATA if missing. The sponsor pays fees and rent.
* Confirmed before the response is returned.
* Enabled only on a dev cluster with the mint authority key present.
* Compiled only into a dev build (the `dev-faucet` feature, which
  `ops/install.sh` enables). A production build has no such route (404) and
  never loads a mint authority; `scripts/check.sh api` fails a release binary
  that carries it.
* Limited to 3 per owner per hour (`FUTURES_FAUCET_PER_HOUR`); no limit
  across owners.
* **[added]** `usdcAta`.

## Futures [added]

The futures program (`ameba-futures`, design in
[`futures/docs/futures.md`](../futures/docs/futures.md)): one market
account per index and expiry holding every trader's cash, resting
order, maker ladder and claim. Everything here is off until the deployment
manifest has a `futures` section; then `/v1/config` names the program.

### Units

* **Positions.** `lo`, `hi` and `pos` are grid positions (numbers, `0..=grid`).
  A position's index value is `pos * stepValue`; on dev one step is one index
  point, so position 100 is `"100"`.
* **Claims.** A long on `[lo, hi)` pays `clamp(S - lo, 0, hi - lo)` steps of
  `k` atoms per lot at settlement `S`; a short pays the rest of the set,
  `set = (hi - lo) * k` atoms per lot. On dev `k` is 10,000 atoms, so a lot is
  $0.01 a point (`lotUsdPerPoint`).
* **Premiums** are atoms per lot (strings), what the long pays. A long's cost
  is `premium * lots`; a short's is `(set - premium) * lots`.
* **Entries** are in fine units of `1/k` step: `lo * k + premium`. Their
  index value is `fine * stepValue / k`. `*Value` entries are rounded to 4
  decimals (exact on dev) without trailing zeros. A trade's or quote's average
  entry is `lo * k + max(premium, 0) / lots`, as the program records its last
  entry.
* **Times** (`openedTs`, `placedTs`, `lastTradeTs`, `expiryTs`) are chain
  clock; trades, candles and index points are shown in wall time (see
  Conventions).
* **Sequences** (`seq`) are decimal strings. An order is named by `(slot,
  seq)`, a claim by `(slot, seq)`; a claim's `seq` changes whenever it is
  split, merged, transferred or partly used.
* `:market` is the market address, or a symbol (the open market of that
  symbol with the nearest expiry). **[added]** It may also be a label
  (`RAMX Oct 26`, any case) or a series key (`RAMX-1793491200`); the
  manifest's markets are matched first. A retired market (see "Retired
  markets") resolves by address, label or series key, never by symbol.

Reads come from the websocket account cache (each market account, its
oracle period and its Earn pool account are subscribed; an owner's Earn
pool positions are followed from their first request on, like their USDC
ATA); quotes and prepares solve packages on that
cached account with the client's exact solver (`Book::propose`), the same
complete arc list the program verifies. Nothing simulates.

### Reference index

`index` (in the market view) is the product's reference index, sampled every
30 s (`FUTURES_INDEX_INTERVAL_SECS`) from an oracle month:
`round(base * (10000 + deltaBps) / 10000)`, 6 decimals. During Game,
`deltaBps` is the sum of each bucket's immutable product weight times its
median accepted standing-source change from baseline (integer bps,
truncation toward zero per bucket). Pending or challenged claims do not move
this reference until adjudication accepts a value. A complete verified
source inventory and 10,000 bps of bucket weights are required; a failed
read retains the previous sample. After Game it uses the month's final index
delta. This indicative reference is separate from the temporal median used
for settlement. The months are the manifest's `oracle.months` (owned by
`oracle.program`); when a product has several, the one in Game (live) with
the nearest expiry is used, else the nearest expiry. Without an `oracle`
section there is no index; the service logs which source is in use at
start. The samples are kept in `futures.index_samples` (see
`/v1/markets/:market/index`).

### `GET /v1/markets?all=1` → `{ markets: MarketView[] }`

The manifest's markets. **[added]** With `all=1` the retired ones follow
them (see "Retired markets").

### `GET /v1/markets/:market` → `MarketView`

```text
MarketView = {
  market, symbol, productId (hex), expiryTs, label, status ("open"|"settled"|"failed"),
  tradable (open and the period tradable at the chain clock, the program's rule),
  period (address), periodState: null | { status ("forming"|"live"|"settled"|"failed"), plannedListingTs,
                                          effectiveListingTs, expiryTs, valueDecimals, settlementValue },
  vault, grid, k (atoms per lot per step), stepValue (index points per step), valueDecimals,
  lotUsdPerPoint ("0.01" on dev), referenceValue (the listing reference; never a settlement),
  index: null | { value, ts },
  lastEntryValue | null, lastTradeTs | null, settlementValue | null,
  trades, volumeLots, volumePremium, backing, reserved, custody, claimsOpen,
  volume24hLots, change24hValue | null,
  suggestion: { sigmaValue, lowValue, highValue, lo, hi, centerValue, vol,
                source ("book"|"oracle-index"|"dev-fixture") },
  maker: null | { owner, slot, free, reserved, maker (ladder slot|null), lo|null, hi|null, escrow,
                  bidLots, askLots, inventoryLots, payoutMin, payoutMax },
  pool: null | { address, operator, shares, valueLow, valueHigh, free, reserved, deposited, withdrawn },
  credit: null | { owner, limit, debt, exposure, makerCash, lends, lendsUntil, settled, shortfall, recoveryAmba },
  slot, chainTime
}
```

* `tradable` is whether the market takes orders now. A market can be listed
  on a Forming oracle period, ahead of its month; it takes no trade until
  the oracle lists the period (`periodState.status` `"live"`), no earlier
  than `periodState.plannedListingTs` (`effectiveListingTs` is 0 until
  then), and the program refuses its orders with `NotTradable` (the user,
  2026-10-15, preflight Decisions 5). The oracle fails only Forming
  periods, so a market that traded never ends on a failed period.
* `index` is the product's reference index (see "Reference index").
* `lastEntryValue` is the last trade on the price line (see candles: its
  range contains the index), else the account's last trade; `lastTradeTs` is
  the account's (both `null` before the first trade). `settlementValue` is the settlement once the market is
  settled or failed (failed: the value the council set).
* `reserved` is the escrow of every resting order and maker ladder (atoms):
  the book's depth in USDC, what the web shows as Liquidity. The Earn
  vault's part is borrowed beyond its cash where its credit covers it
  (settlements from 80 to 120, `docs/futures.md` "Band funding").
* `change24hValue` is the last entry minus the entry of the last price-line
  trade before 24 hours ago (else the first one in the window); `null`
  without trades. `volume24hLots` sums the lots filled in the last 24 hours.
* `suggestion` is the ticket's default range: `index -/+ 2 sigma` in whole
  grid positions (`lo`, `hi`) clamped to `[1, grid - 1]` and at least one
  step wide, then to the span the live maker ladders quote (when that span
  still contains the index), so the default range always has executable
  depth. `sigmaValue` is one standard deviation of the settlement in index
  points:
  * with a live maker ladder, the one the book prices (`source: "book"`):
    half the distance between the settlements the book prices at an 84.1%
    and a 15.9% chance of being exceeded (one standard deviation either side
    for a normal). Each ladder step quotes the one-step claim `[x, x + 1)`;
    its mid premium over `k` (the middle of the best bid and best ask across
    the ladders) is about the chance the index settles above `x + 1/2`.
    Steps below every quote count as certain, steps above as never, an
    unquoted step between keeps the price of the step below, and the chance
    never rises going up. `vol` is the annualized volatility that spread
    implies, `sigma / (index * sqrt(T))` with `T` the time to expiry in years;
  * without one, `index * vol * sqrt(T)`, where `vol` is the index's
    annualized realized log-return volatility over the sampler's last 30
    days of `futures.index_samples` when there are at least 20 samples
    spanning at least a day and the index moved (`source: "oracle-index"`,
    refreshed every minute), otherwise the dev fixture, 40% a year
    (`source: "dev-fixture"`).

  Without an index reading the range is centred on the last entry, else the
  reference value (`centerValue`).
* `maker` is the treasury maker's trader in this market: the Earn vault's
  (see "Earn") where it has one, else the manifest's `futures.treasury`
  trader (its own, from before the vault): its
  cash, its ladder's bounds and escrow (`lo`/`hi` are `null` when no ladder
  is live), the lots its steps quote, its gross one-step inventory
  (`inventoryLots`, the sum of `|inventory|`), and the least and most that
  inventory pays (atoms) over every grid settlement point `S = x * k`,
  `x = 0..=grid`. `null` when neither has a trader slot here.
* `pool` is the market's **legacy Earn pool** (dev only; Earn is the one
  vault since 2026-10-16, see "Earn", and new markets have none): the pool
  account the manifest's `futures.treasury` operates (PDA `["ameba-range-v1", "pool", market,
  treasury]`; the deployed program's seeds keep their name), `null` while
  that account does not exist (or without a treasury). Its cash and
  inventory are the market trader whose owner is `address`; `free` and
  `reserved` are that trader's (`"0"` before the first deposit).
  `valueLow`/`valueHigh` are what the pool holds is worth, the least and
  most over every settlement outcome (free + reserved + what its claims and
  ladder inventory pay; the settlement once final), the program's own
  valuation. `shares` are the shares outstanding;
  `deposited`/`withdrawn` the cash ever paid in and out (atoms, all decimal
  strings). A deposit mints `amount` shares into an empty pool, else
  `floor(amount * shares / valueHigh)`; withdrawing pays
  `floor(shares * valueLow / pool shares)` from `free`.

* `credit` **[added 2026-10-17]** is what backs the maker's side here
  beyond its own cash: the stake-backed credit its stake pool lent the Earn
  vault (`owner`) in this market (see "Stake"), `null` without any.
  `limit` is the most its position may cost the stakers; `debt` the cash it
  borrowed (custody + debt = free + reserved + backing); `exposure` what its
  position could cost them at worst over every fill of its resting orders
  and ladder (the bound the program keeps within `limit`); `makerCash` its own cash (free and
  reserved less the debt); `lends` whether it may borrow more now (open,
  before `lendsUntil`, the expiry less the pool's window); `settled`,
  `shortfall` and `recoveryAmba` after `CreditSettle` (a shortfall's winners
  are paid that much less USDC, in AMBA instead). Atoms as decimal strings.

### `GET /v1/markets/:market/book`

```text
{ market, slot, orders: OrderView[], ladders: LadderView[], sources: SourceView[] }

OrderView  = { slot, seq, owner, side ("long"|"short"), lo, hi, loValue, hiValue, lots, placedLots,
               price (premium atoms/lot), entryValue, escrow, close (bool), claim: null | { slot, seq|null },
               placedTs }
LadderView = { maker (slot), owner, seq, live (bool), lo, hi, loValue, hiValue, escrow, quotedTs,
               tiers: { count, ratio, spread },
               steps: [{ pos, bid|null, ask|null, bidValue|null, askValue|null, bidLots, askLots,
                         inventory (signed lots: + long, - short) }] }
SourceView = { source: { kind: "order", slot, seq }
                       | { kind: "maker", maker (slot), step, tier, seq },
               owner, side ("long"|"short"), lo, hi, loValue, hiValue,
               lots, price (premium atoms/lot), entryValue }
```

* Orders are listed by slot. A resting order trades at exactly its `price`;
  `escrow` is what it reserves (`lots * price` for a long, `lots * (set -
  price)` for a short, nothing for a `close` order, which reserves its
  `claim` instead). A `close` order's `claim.seq` is `null` once that claim
  is gone.
* `owner` (on orders, ladders and sources) is `null` when the trader slot is
  not found.
* A ladder step `pos` quotes the one-step claim `[pos, pos + 1)`: a bid
  (the maker buys it) and an ask (it sells it). `steps` lists only steps that
  quote lots or hold inventory. A ladder with `live: false` was cancelled and
  keeps only inventory.
* Depth `tiers`: each step quotes `count` prices per side, `spread` atoms
  apart going away from the mid (`bid`/`ask` are tier 0, the best), each tier
  holding `ratio` percent of the lots of the one before; `bidLots`/`askLots`
  are what remains over all tiers, and the best tiers trade first. The depth
  endpoint therefore shows one level per tier for a ladder-supplied range.
* `sources` is the complete live source list used by the package solver
  (`Book::arcs()`), from the same cached account and slot as `orders` and
  `ladders`. Every nonempty resting order and nonempty maker step/side/tier
  appears once; cancelled ladders and inventory without an active quote do
  not. A maker source's `lots` is that tier's actual remaining quantity and
  `price` its exact premium, including after cheaper tiers partially fill or
  exhaust. The source identity includes the top-level `side`; maker tiers
  start at 0. There is no range parameter or source-count truncation.
* Source `entryValue` converts `lo * k + price` fine units into index points. These
  entries can be displayed together, but different intervals are different
  claims: quantities may be combined only for the same side, exact bounds
  and exact premium, never merely because rounded `entryValue` matches.
  Unlike-range bids above asks do not imply a crossed market or a universal
  spread. `sources` describes quoted supply, not executable depth of an
  arbitrary range; use `/depth` or `/quote` for the ticket's claim. Market
  `tradable` still governs whether orders can execute.

### `GET /v1/markets/:market/depth?lo=&hi=&max=`

```text
{ lo, hi, loValue, hiValue, asks: Level[], bids: Level[] }
Level = { premium (signed atoms/lot), entryValue, lots, cumulativeLots }
```

The executable depth of one claim shape, from the exact solver: `asks` are
the successive premiums of buying `[lo, hi)` (entries ascending), `bids` of
selling it (entries descending), each level one more cheapest package
increment across every order and ladder step (a composite route included),
up to `max` lots (default 1,000,000,000). `0 <= lo < hi <= grid`, else 400.
Without `lo` and `hi` the market's `suggestion` range is used. A premium
can be negative when resting prices are inconsistent in the taker's favour.

### `GET /v1/markets/:market/trades?limit=100&before=<signature>`

```text
{ trades: [{ signature, eventIndex, slot, ts, side ("buy"|"sell", the taker: buy = long), lo, hi,
             loValue, hiValue, lots (filled), premium (signed total), entryValue (average),
             sources ("orders"|"ladder"|"mixed"), taker }] }
```

One entry per `Submit` that filled (`ARTAKE01` with `filled > 0`), newest
first, ordered by (block time, slot, event index). `sources` says what
filled it (the `ARFILL01` and `ARLADR01` lines of the same `Submit`).
`before` pages to trades older than that transaction; `limit` is clamped to
1..1000.

### `GET /v1/markets/:market/candles?resolution=300&from&to`

`{ resolution, candles: [{ t, o, h, l, c, v }] }` over the trades' average
entry values (index values as numbers) and filled lots. Empty without trades
(the site falls back to the index).

* `resolution` is one of 60, 300, 900, 3600, 14400 or 86400.
* Buckets with no trades are omitted. The client carries the previous
  close forward.
* By default `to` is now and `from` is `to - 500 * resolution`. Any window
  may be asked for; every trade in it is counted.

Only trades on the **price line** count: those whose range contains the
oracle index (current reading, in grid positions). A claim far from the index
trades near its own end (a 150-172 claim near 150 when the index is 100), so
its entry says nothing about the market's level. Volume, the trade tape and
accounts still include every trade.

### `GET /v1/markets/:market/index?from&to&limit` [added]

```text
{ market, symbol, expiryTs, month | null, points: [{ t, value }] }
```

The reference index of the market's symbol over time (see "Reference
index"): the samples, oldest first, taken about every 30 s; `value` is a
number in index points. `from`/`to` default to the last 24 hours of wall
time; `limit` (default 5000, at most 20,000) keeps the newest points in the
range. `month` is the oracle month the market settles on: the canonical
month PDA of its period (`[ameba-oracle-v1, g3-oracle_month, period,
expiry_le]`).

### `GET /v1/accounts/:owner`

```text
{ owner, usdc, usdcAta, usdcAtaExists,
  markets: [{ market, symbol, label, expiryTs, status, tradable, markValue | null,
              trader: null | { slot, free, reserved, paidOut },
              orders: OrderView[], claims: ClaimView[], ladder: LadderView | null,
              lp: null | { pool, shares, valueLow, valueHigh, deposited, withdrawn } }] }

ClaimView = { slot, seq, side, lo, hi, loValue, hiValue, lots, cost, reserved, entryValue, openedTs,
              maxPayout (lots * set), payoutAtIndex | null, pnlAtIndex | null (payoutAtIndex - cost),
              exit: null | { lots, premium (total), net, entryValue } }
```

* Only markets where the owner has a trader slot or an Earn pool position
  are listed. **[added]** Retired markets are listed too, after the
  manifest's, with `retired: true` (every entry carries `retired`); they are
  never `tradable` and their claims have no `exit`. `trader` is `null` (with no orders, claims or ladder) where
  they have only a position. `usdc` is the owner's USDC ATA (followed from
  the first request on).
* `lp` is the owner's position in the market's legacy Earn pool (`pool`
  in the market view), `null` without a position account or while the pool
  does not exist. Earn positions are `GET /v1/accounts/:owner/earn`. `shares` are theirs; `valueLow`/`valueHigh` their share of the
  pool's values, `floor(shares * pool valueLow / pool shares)` and likewise:
  `valueLow` is what withdrawing every share pays now. `deposited` and
  `withdrawn` are the cash they ever paid in and took out (atoms). A
  position whose shares were all withdrawn stays listed with `shares` `"0"`.
* A claim's entry: a long's `lo + cost / (lots * k)`, a short's
  `lo + (set - cost / lots) / k`.
* `payoutAtIndex` values the claim at `markValue`: the reference index while
  the market is open, the settlement once it is final; `null` without an
  index reading.
* `exit` is the executable sale of the claim now, while the market is
  tradable: the opposite side on the claim's interval for its unreserved
  lots at the most permissive limit (selling a long: a short at limit 0;
  buying back a short: a long at limit `set`), solved on the cached book.
  `lots` fill for `premium` in total; `net` is what the owner's free cash
  gains (selling a long: the premium; buying back a short: the released sets
  less the premium). `null` when nothing fills.

### `POST /v1/quote`

Request:

```json
{ "market": "…", "owner": "…", "side": "long", "lo": 98, "hi": 120, "lots": 3, "limit": "220000",
  "mode": "ioc", "close": { "slot": 4, "seq": "17" } }
```

Response:

```text
{ market, side, lo, hi, loValue, hiValue, lots, limit (the limit used), mode,
  filled, premium (signed total), averageEntryValue | null,
  pay (signed atoms the taker puts in: long = premium, short = set * filled - premium; negative = receives),
  maxPayout (set * filled), legs: { orders, ladderSteps },
  free?, usdc?, redeemed?, netPay?, escrow?, deposit?   (with `owner`)
  compressedUsdc?, decompress?   (with `owner`, when `deposit` exceeds `usdc`)
  slot, error? }
```

* `owner`, `limit`, `mode` (`"ioc"` default, `"gtc"`, `"post"`), `minFill`
  and `close` are optional. `side` takes the usual spellings (`"long"` /
  `"buy"`, `"short"` / `"sell"`). `0 <= lo < hi <= grid`, `lots >= 1`,
  `limit <= set` and `minFill <= lots`, else 400. The default `limit` is the
  most permissive: `set` for a long, 0 for a short; a `gtc` or `post` order
  needs one.
* `legs` counts the resting orders and the ladder step sides the package
  fills.
* With `owner`: `free` is their free cash in the market; `redeemed` the sets
  the fill releases by netting against their opposite claims' unreserved
  lots wherever the intervals overlap (exact complements, the `close` claim
  included, first; `futures/docs/futures.md`, "Overlap redemption");
  `netPay = pay - redeemed`, what their cash actually changes by; `escrow` what a resting remainder (`gtc`/`post`)
  reserves at `limit`; `deposit` what `submit` would deposit from the
  ATA (`max(0, pay - redeemed + escrow - free)`). **[added]** When
  `deposit` exceeds `usdc`: `compressedUsdc` is the owner's compressed USDC
  and `decompress = deposit - usdc`, what the order takes from it (see
  "Spending compressed USDC").
* `error` (the package fields are still set): `NotTradable`, `NoLiquidity`
  (an `ioc` order fills nothing), `WouldCross` (a `post` order would fill),
  `MinFill`, `InsufficientFunds` (`deposit` exceeds the ATA plus the
  compressed USDC), and for `close`
  `ClaimNotFound`, `NotComplement` or `ClaimReserved`. `close` needs `owner`.

### Market transactions (`POST /v1/tx/prepare`)

`params` name the market as `market` (address or symbol). Every transaction
starts with `SetComputeUnitLimit(1_400_000)`; `submit` steps also request a
256 KiB heap frame (`heapFrame`), which `Submit` needs. The sponsor pays the
fee (the owner with `payer: "owner"`, see "Owner-paid transactions"); the
1232-byte limit applies.

| action | params | instructions |
|---|---|---|
| `deposit` | `market, amount` | deposit from the owner's USDC ATA (400 when `amount` exceeds it) |
| `withdraw` | `market, amount` (atoms, or `"all"`) | [createAta], withdraw. 400 when the owner has no trader or `amount` exceeds `free` |
| `submit` | `market, side, lo, hi, lots, limit?, minFill?, mode?, close?: { slot, seq }, withdraw?: bool` | heapFrame, [createAta, decompressLightAta, decompress], [deposit], submit, [createAta, withdraw all] |
| `close` | `market, slot, seq, lots?, limit?, minFill?, withdraw?: bool (default true)` | heapFrame, [createAta, decompressLightAta, decompress], [deposit], submit (`ioc`, naming the claim), [createAta, withdraw all] |
| `cancel` | `market, slot, seq, withdraw?: bool` | cancel, [createAta, withdraw all] |
| `poolWithdraw` | `market, shares` (atoms, or `"all"`) | [createAta], poolWithdraw (legacy pools' exit, dev) |
| `finalize` [added] | `market` | finalize |
| `settle` [added] | `market, withdraw?: bool (default true)` | heapFrame, [finalize], [settle], [settleMaker], [prune], [createAta, withdraw all] |

* **`submit`.** Priced as `/v1/quote`; any quote `error` is refused with
  400 `"<error>: <reason>"`. The proposal is solved now from the cached
  book and travels in the instruction. When the trader's free cash is
  short of what the order needs, a `deposit` of exactly the shortfall
  (`deposit` in the quote) comes first: the fill's payment less the sets its
  netting releases (the program redeems those before it debits), plus the
  escrow of a resting remainder (`limit * lots` for a long, `(set - limit) *
  lots` for a short, nothing for a close order). `minFill` defaults to 1 for
  `ioc` and 0 otherwise; `mode` to `"ioc"`; `limit` to the most permissive
  for `ioc`. `withdraw: true` adds a withdrawal of all free cash.
  **[added]** When the deposit is more than the ATA holds, the shortfall is
  decompressed from the owner's compressed USDC into their ATA first, in
  this transaction when it fits, else in `before` (see "Spending compressed
  USDC"); the summary's `decompress` is that amount.
* **`close`** sells (or buys back) the claim `(slot, seq)`: the
  opposite side on its interval naming it, `ioc`; `lots` defaults to its
  unreserved lots, `limit` to the most permissive (0 when selling a long,
  `set` when buying back a short). The sale nets against the claim, so the
  seller receives the premium (or the released sets less the premium) and,
  by default, withdraws all free cash in the same transaction.
* **`cancel`** cancels the owner's order `(slot, seq)`, releasing its
  escrow (or its claim reservation).
* **`poolWithdraw`** (the legacy pools' exit: dev builds of the program
  keep `PoolWithdraw` so the dev chain's pool shares can leave; `PoolDeposit`
  is retired) redeems `shares` (`1..=` the position's shares,
  or `"all"`) for cash paid from the pool's free cash to the owner's USDC
  ATA (created first when missing). 400 when the market has no Earn pool,
  the owner has no position or no shares, the shares are all of the pool's
  while `valueHigh` exceeds their payout (`NotSettled`: the last shares
  leave at settlement or once the open positions close), or the payout now
  (`floor(shares * valueLow / pool shares)`) exceeds the pool trader's
  `free` cash: the rest backs its quotes and claims. Both messages say how
  many shares can be withdrawn now. Summary: `pool`, `shares` (the
  shares redeemed, `"all"` resolved to the position's), `all` (bool) and
  `amount`, the payout at the cached pool value; the program prices it
  again when it lands (`InsufficientFunds` if the pool's free cash is short
  then).
* **`finalize`** finalizes an expired market (the program's
  permissionless `Finalize`): once its oracle period is settled (and the
  market has expired), it settles at the period's value (`status:
  "settled"`). A failed period, or one the oracle hasn't settled, is
  settled by the governance council instead (the program's
  `CouncilSettle`, `futures-ops council-settle`), any time after expiry,
  at the value it sets (`status: "failed"`); `finalize` refuses it. 400
  `AlreadyFinal`, `NotExpired` or `NotSettled` (with the reason and, for
  an unsettled period, the expiry from which the council may act) when the
  program would refuse it; 503 while the period is not readable. Summary:
  `status`, `settlementValue` and `periodStatus`. Anyone may finalize; the
  keeper normally does.
* **`settle`** pays the owner out of an expired market without the
  keeper: it finalizes first when the market is still open (as
  `finalize`, same refusals), pays the owner's claims (`Settle`, at
  most 40 per transaction: the keeper's batch; `remainingClaims` says how
  many are left, prepare again for them), settles their ladder
  (`SettleMaker`: its inventory paid, its escrow released), releases their
  resting orders' escrow (`Prune`), and by default withdraws all free cash
  to the owner's USDC ATA (created first when missing; `withdraw: false`
  leaves it in the market). 400 `NoAccount` without a trader in the market,
  `NothingToSettle` when there is nothing to finalize, settle or withdraw.
  Summary: `status`, `settlementValue`, `periodStatus`, `finalize` (bool),
  `claims` (slots paid), `remainingClaims`, `maker` (ladder slot or null),
  `orders` (slots released), `payout` (what the claims and the ladder's
  inventory pay at the settlement), `released` (escrow freed), `free` (the
  trader's free cash before), `withdraw` (bool) and `amount` (`free +
  payout + released` when withdrawing, else `"0"`); the program prices it
  again when it lands.
* `Finalize`, `Settle`, `SettleMaker` and `Prune` need no signer; the
  service adds the owner as an extra read-only signer account (the program
  reads only the leading accounts), so the sponsor still co-signs only a
  transaction the owner signed.
* The summary carries `market`, `symbol`, `label` and the action's fields;
  `submit`/`close` also `expectedFill`, `premium`, `averageEntryValue`,
  `pay`, `netPay`, `legs`, `deposit` and `withdraw`.
* A stale proposal either still proves optimality when it lands or fails
  (for example `NotBestPrice`, `FifoViolation`); prepare again. Futures
  program errors are named in `/v1/tx/:signature`, `/wait` and stream `tx` events
  (base `0x46520000`: `InsufficientFunds`, `MinFill`, `WouldCross`,
  `ClaimReserved`, …).
* `POST /v1/tx/quote-prepare` with an order returns its quote and the
  `submit` together (see quote-prepare).

### Events and indexing

Every futures instruction writes its market (account 0), so each market's
transactions are indexed from `getSignaturesForAddress(market)` (oldest
first, a cursor per market in `futures.cursors`), and the tracker stores
the events of the transactions it follows at once. Rounds follow the
futures program's logs (`logsSubscribe`): a round 500 ms after each of its
transactions, then every 1.5 s until that transaction is indexed, and one
every 10 s regardless. Only lines logged while
the futures program is on top of the invoke stack count; each is attributed
to the market of its top-level instruction. Tables (created idempotently,
append-only, never cleared): `futures.events` (every event, idempotent on
signature and event index) and `futures.trades` (each `ARTAKE01` that
filled). The event decoder has its own version (`futures.meta` key
`futures_decoder_version`): bumping it drops only the cursors, so the
markets are rescanned and the idempotent inserts add what was missing.

**[adjusted]** Until 2026-10-15 these tables were `futures.range_events`,
`futures.range_trades`, `futures.range_cursors` and `futures.range_markets`
(the registry), and the decoder's key `range_decoder_version` (now
`futures_decoder_version`: `decoder_version` was the retired clearing
indexer's key, which may still be there). The schema step at startup
renames each table (with its indexes, primary keys
included) and the key in place, in one transaction, when the old name
exists and the new one does not; then it gives the stored events their new
type names (`rangeTake` → `take`, …). Every row is kept, and a fresh
database gets the new names directly. A table found under both names
stops the schema step with an error (the retired clearing markets' tables
had these names and are removed before this version runs), so history is
never served from the wrong table.

| type (tag) | data |
|---|---|
| `deposit` (`ARDEPO01`), `withdraw` (`ARWDRW01`) | `owner`, `amount` |
| `take` (`ARTAKE01`) | `owner`, `side`, `mode`, `lo`, `hi`, `lots`, `filled`, `premium`, `paid`, `seq`, `rested` (order slot or null) |
| `fill` (`ARFILL01`) | `owner` (the resting order's), `taker`, `side` (the resting order's), `close`, `lo`, `hi`, `lots`, `price`, `slot`, `seq` |
| `ladderFill` (`ARLADR01`) | `maker`, `taker`, `side` (the maker's: long = its bids filled), `lo`, `hi` (steps spanned), `lotSteps`, `premium` |
| `place` (`ARPLACE1`) | `owner`, `slot`, `seq`, `side`, `close`, `lo`, `hi`, `lots`, `limit`, `escrow` |
| `cancel` (`ARCANCL1`) | `owner`, `target` (`order`/`ladder`), `slot` (null for a ladder), `seq`, `released` |
| `quote` (`ARQUOTE1`) | `owner`, `maker`, `seq`, `lo`, `hi`, `escrow`, `bidLots`, `askLots` |
| `redeem` (`ARREDM01`) | `owner`, `lo`, `hi`, `lots`, `cash` |
| `transfer` (`ARXFER01`) | `from`, `to`, `side`, `lo`, `hi`, `lots` |
| `finalize` (`ARFINL01`) | `market`, `status`, `settlementFine`, `settlementValueAtoms` |
| `settle` (`ARSETL01`) | `owner`, `claim`, `side`, `lo`, `hi`, `lots`, `payout` |
| `settleMaker` (`ARMKSE01`) | `owner`, `maker`, `payout` |
| `market` (`ARMRKT01`) | `market`, `symbol`, `productId`, `expiryTs`, `registry` |
| `pool` (`ARPOOL01`) | `pool`, `market`, `operator` (an Earn pool created; the operator is the primary party, the pool the counterparty) |
| `poolDeposit` (`ARPDEP01`) | `owner`, `pool`, `amount`, `shares` (minted) |
| `poolWithdraw` (`ARPWDR01`) | `owner`, `pool`, `shares` (redeemed), `amount` (paid) |
| `earnCreate` (`ARVCRT01`) **[added]** | `vault`, `operator`, `cashAccount` (the operator is the primary party, the vault the counterparty) |
| `earnDeposit` (`ARVDEP01`) **[added]** | `owner`, `vault`, `amount`, `shares` (minted) |
| `earnWithdraw` (`ARVWDR01`) **[added]** | `owner`, `vault`, `shares` (redeemed), `amount` (paid) |
| `earnFund` (`ARVFND01`), `earnDefund` (`ARVDFD01`) **[added]** | `vault` (the owner), `market`, `amount` (moved into, or back out of, the market) |
| `earnMigrate` (`ARVMIG01`) **[added]** | `owner` (the holder), `vault`, `pool`, `amount` (the pool's lowest value), `shares` (vault shares minted) |

Earn deposits and withdrawals are in the depositor's history (and, with
the vault as counterparty, in the vault address's); amounts and shares are
decimal strings in atoms. The vault trades as its own trader in each market:
its fills, quotes and cancels are ordinary events whose `owner` is the vault
address. `VaultDeposit` and `VaultWithdraw` name the vault, not a market, as
their account 0: their events' `market` (and `instrument` in the history) is
the vault's address, and the indexer follows the vault's address like a
market's. The legacy pools' events read the same way with the pool.

### Manifest

```json
"futures": {
  "program": "<pubkey>",
  "treasury": "<pubkey>",
  "markets": [ { "symbol": "RAMX", "productId": "<64 hex>", "expiryTs": 1793491200, "label": "RAMX Oct 26",
                 "market": "<pubkey>", "registry": "<pubkey>", "authority": "<pubkey>", "vault": "<pubkey>",
                 "period": "<pubkey>", "grid": 200, "k": 10000, "stepValue": 1000000, "valueDecimals": 6,
                 "refValue": 100000000 } ]
}
```

Optional. `program` and each market's `market` and `expiryTs` (and
`symbol` or `productId`) are required; the market parameters, vault and
period are read from the market account, and the manifest's copies are only
checked against it (a mismatch is logged). A malformed section or market is
left out with a warning, and the rest of the service runs as before. The
futures program, the treasury and the treasury's Earn pool in each market
are reserved owners (no sponsored transactions, left out of the
leaderboard).

### Retired markets [added]

Every market the service has seen is recorded in `futures.markets` (the
registry, created with the schema and never cleared): address, program, symbol, label, expiry, product id, period,
vault, Earn pool, `grid`, `k`, `stepValue`, `valueDecimals`, the reference
settlement, the genesis hash, when it was first recorded and last listed,
and its final status and settlement once its account shows them. The
manifest's markets are recorded from their accounts at start and every 10
minutes; a market with indexed events but no record is read from chain once
and recorded under its product symbol and expiry label. Parameters are
never rewritten.

A recorded market the manifest no longer lists is **retired**. Markets
added to the manifest (a new expiry, `futures-ops list-next`) are served
within about 5 s, without a restart ("Deployment changes"); markets
leaving it stay readable:

* by address, label or series key: `/v1/markets/:market` (and `/book`,
  `/depth`, `/trades`, `/candles`, `/index`); with `all=1` in
  `/v1/markets`. Market views carry `retired: true` (the manifest's
  `false`). Trades, candles and the index need only the registry row, so
  they answer even when the account can no longer be read;
* in `/v1/accounts/:owner` (with `retired: true`), in
  `/v1/accounts/:owner/history` (symbol, label and values from the
  registry) and in the leaderboard (a market whose account is gone is
  valued at its recorded settlement once final, else at the index as
  `unpriced`);
* in `/v1/stream` (`market=` or `instrument=` by address or label).

Their account, period and Earn pool stay subscribed in the account cache
(a market already final and drained, with no claim, order, ladder or cash
left, keeps the state read once). The indexer follows them every 30 s with
their own program until one pass after they are drained; their rows stay.

Transactions: only `withdraw`, `poolWithdraw`, `cancel`, `finalize` and
`settle` (cash out and settlement); any other market action, and
`/v1/tx/quote-prepare`, is refused with 400 `RetiredMarket`. Quotes
(`/v1/quote`) still answer.

## Earn [added]

One Earn vault for the whole futures product (`futures/docs/futures.md`,
"The Earn vault"): the PDA `["ameba-range-v1", "earn", treasury]` of the
manifest's `futures.treasury`, which operates it. Depositors hold shares of
everything it holds: its idle cash and its trader in each of its markets,
where the treasury maker quotes with it. A deposit buys shares at the vault's
**highest** value over every settlement outcome, a withdrawal pays its
**lowest**, so neither moves value from the other depositors; the gap is the
open positions' range and closes as they settle. The vault, its markets and
an owner's position are followed in the account cache; nothing simulates.

### `GET /v1/earn`

```text
{ vault: null | EarnVault, chainTime }
EarnVault = { address, operator, cashAccount, shares, cash, deposited, withdrawn, valueLow, valueHigh,
              markets: [{ market, symbol | null, label | null, expiryTs | null, status | null, tradable,
                          free, reserved, valueLow, valueHigh }],
              rewards: null | EarnRewards }
EarnRewards = { mint, decimals, account, house | null, startTs, endTs, amount, distributed, paid }
```

* `vault` is `null` while the vault does not exist (or without a treasury).
* `cash` is its idle cash (in `cashAccount`; anything else sent to that
  account is not counted); `shares` the shares outstanding;
  `deposited`/`withdrawn` the cash ever paid in and out.
* `markets` are the vault's markets in its order (every market where it
  may hold a trader, at most `VAULT_MARKETS`, 13): its trader's `free` and
  escrowed (`reserved`) cash there and what it holds there is worth
  (`valueLow`/`valueHigh`, over every settlement outcome; the settlement
  once final). `symbol`, `label`, `expiryTs` and `status` are `null` for a
  market the service does not know.
* `valueLow`/`valueHigh`: `cash` plus every market's values, the program's
  own valuation (markets settle independently, so the extremes add).
* `rewards` is the vault's AMBA stream (`futures/docs/futures.md`, "Earn
  rewards"), `null` without one: `amount` atoms of `mint` (`decimals`)
  released linearly from `startTs` to `endTs` out of the reward `account`
  to depositors by their shares, `house`'s shares left out;
  `distributed` is what has gone to depositors up to `chainTime` (accrued
  as the program would), `paid` what has been paid out of `account`.

### `GET /v1/accounts/:owner/earn`

```text
{ owner, usdc, vault: EarnVault | null, chainTime,
  position: null | { address, exists, shares, valueLow, valueHigh, deposited, withdrawn,
                     earningsLow, earningsHigh, share, withdrawable,
                     rewards: null | { pending, paid, earned, perSecond, asOf } } }
```

* `position` is `null` while the vault does not exist; otherwise the
  owner's position account (PDA `["ameba-range-v1", "earn-lp", vault,
  owner]`, `exists: false` and zeros before the first deposit). `usdc` is
  the owner's USDC ATA balance.
* `valueLow` = `floor(shares * vault valueLow / vault shares)`: what
  withdrawing every share pays now; `valueHigh` likewise.
  `earningsLow`/`earningsHigh` (signed) are those less the cash put in
  (`deposited - withdrawn`). `share` is `shares / vault shares` as a
  decimal fraction with up to nine places (`"0.25"`; `"0"` without shares):
  the owner's stake of the whole of Earn.
* `withdrawable` is what can be withdrawn now: all of `valueLow`, unless the
  owner holds all of the vault's shares while its open positions may still
  pay more (the last shares wait for settlement: one share stays), or the
  vault's cash (idle plus its free and escrowed cash in its markets) is
  short of the payout.
* `rewards` (`null` without a stream) is the owner's AMBA in atoms of the
  reward mint: `pending` accrued to `chainTime` and not paid yet, `paid`
  ever paid into their AMBA account, `earned` both. Once the stream ends
  the keeper pays everyone's `pending`; the token's transfer fee comes out
  of what arrives. **[added 2026-10-20]** The owner may also claim it at any
  time (`earnClaim`, below).
  **[added 2026-10-20]** `asOf` is the chain time (unix seconds) the
  figures are for, the response's `chainTime`; `perSecond` (atoms, a
  decimal string) is what the position earns each second from then: the
  stream's release rate, `amount / (endTs - startTs)`, times the
  position's `shares` over the vault's earning shares (its `shares` less the house's), rounded down once:
  `floor(amount × shares / ((endTs - startTs) × earning shares))`. It is
  `"0"` before `startTs`, from `endTs` on, without shares, and for the
  house. While the shares stay as they are, `pending + perSecond × (t -
  asOf)` for `t` up to `endTs` counts the rewards up live (the program
  rounds each accrual down, so what it owes may differ by a few atoms);
  read the position again after a deposit, withdrawal or claim, or when
  the vault's shares change.

### `GET /v1/accounts/:owner/earn/activity?limit=100&before=<signature>`

```text
{ owner, activity: [{ signature, slot, ts, kind ("deposit"|"withdraw"|"migrate"), amount, shares }] }
```

The owner's `earnDeposit`, `earnWithdraw` and `earnMigrate` events (a
legacy pool moved into the vault), newest first, from the history
(`/v1/accounts/:owner/history`, whose `before` and `limit` they take; `ts`
as there).

### Earn transactions (`POST /v1/tx/prepare`)

| action | params | instructions |
|---|---|---|
| `earnDeposit` | `amount` (atoms) | vaultDeposit from the owner's USDC ATA |
| `earnWithdraw` | `amount` (atoms, or `"all"`) | [createAta], vaultWithdraw |
| `earnClaim` **[added 2026-10-20]** | none | [createAmbaAta], vaultRewardsClaim |

* **`earnDeposit`**: `amount` is `1..=` the ATA balance, else 400. The
  sponsor (the owner, owner-paid) pays the fee and, on the first deposit,
  the position account's rent. The instruction names every market of the vault in its order (the
  program values the vault in each). 400 `InsufficientFunds` when `amount`
  buys no share at the vault's value now. Summary: `vault`, `amount` and
  `shares`, the shares it mints at the cached value (`amount` into an
  empty vault, else `floor(amount * shares / valueHigh)`); the program
  prices it again when it lands.
* **`earnWithdraw`**: `amount` atoms are sent as the fewest shares that
  pay at least that (`1..=` the position's `valueLow`); `"all"` sends every
  share. Paid from the vault's idle cash, then from its cash in the
  markets holding the most of it (named as draws; where a market's free
  cash is short the program retires the vault's ladder there to release
  its escrow, and the maker quotes again). 400 `NoPosition` without shares,
  `NotSettled` for all of the vault's shares while its open positions may
  pay more, `InsufficientFunds` beyond `withdrawable`. Summary: `vault`,
  `shares`, `all`, `amount` (the payout at the cached value) and `draws`
  (the markets drawn on).
* **`earnClaim`** **[added 2026-10-20]**: pays the position's AMBA rewards
  (`rewards.pending`) now, before the stream ends, into the owner's own
  associated account of the stream's `mint` under the mint's token program
  (Token-2022 for AMBA; the token's transfer fee comes out of what
  arrives). `createAmbaAta` is an idempotent creation of that account,
  added when it does not exist, its rent paid by the sponsor (the owner,
  owner-paid). `vaultRewardsClaim` is the program's `VaultRewardsClaim`
  (tag 26) with the owner as an extra read-only signer after its six
  accounts (the program asks no signature; the owner signs so that the
  sponsor pays only for claims their owners sign). The transaction starts
  with `SetComputeUnitLimit(200_000)`, sponsored or owner-paid (simulated
  on the dev chain, a claim took about 24,300 CU, about 43,800 with the
  account created). Refused
  with 400 `Nothing to claim: Earn has no AMBA rewards` without a stream
  and `Nothing to claim yet` when `pending` is 0 now (no position, the
  house, or everything already paid). Summary: `vault`, `amba` (the
  pending atoms now; the claim pays what is pending when it lands, at
  least this) and `ambaAccount` (the account paid into).
* `earnDeposit` and `earnWithdraw` fit one legacy transaction for up to
  the vault's 13 markets (two drawn, the ATA created); the 1,232-byte check
  applies as for every action.
* `poolWithdraw` remains for the legacy per-market pools' shares (dev);
  `poolDeposit` is gone.

## Stake [added 2026-10-17]

Staked AMBA backs the Earn vault's maker (`futures/docs/futures.md`,
"Staked AMBA backs the maker"). Stakers' AMBA, at the operator's posted
price averaged over time and haircut, is credit the maker may borrow in a
market (each market's `credit`). The stakers earn their share of its profit
there in USDC and an AMBA stream (the transfer-fee AMBA). When a market
ends short, the stakers' USDC buffer and then their AMBA pay that market's
winners. Read from the account cache.

**[adjusted 2026-10-18]** Unstaking takes seven days (`unstakeSecs`,
604,800; the user's decision). `startUnstake` starts the clock on staked
AMBA; until it ends that AMBA keeps earning and still pays a shortfall, but
no longer counts toward the capacity. Then `unstake` (or the keeper, which
completes finished unstaking by itself into the owner's associated AMBA
account) pays it out, as far as what stays still backs the credit lent;
the rest waits for that credit to come back as markets settle. Nothing
leaves while a market the pool lends to is in its last day before expiry
(the pool's `expiryWindowSecs`) or past expiry with its credit unsettled:
it waits until the markets it backs settle.

### `GET /v1/stake`

`{ pool: StakePool | null, chainTime }`, `null` while the pool does not
exist.

```text
StakePool = {
  address, vault, operator, ambaMint, ambaDecimals, ambaProgram, ambaAccount, usdcAccount,
  staked (AMBA atoms), shares, unstaking (AMBA atoms of shares unstaking), unstakeSecs (604800),
  price: { posted, postedUsd, postedTs, average, averageUsd, windowSecs },
  stakedValue (USDC atoms at the average), capacity (of the AMBA not unstaking, × haircut), allocated (credit lent),
  free (AMBA atoms not backing lent credit), buffer, usdcOwed, usdcDistributed, profitIn, covered,
  recoveryOwed (AMBA set aside for winners, unpaid), slashed (ever),
  params: { haircutBps, profitBps, bufferBps, twapWindowSecs, expiryWindowSecs },
  rewards: null | { account, startTs, endTs, amount, distributed, paid },
  markets: [{ market, symbol, label, limit, debt, exposure, lends, settled, shortfall, recoveryAmba }]
}
```

Prices are USDC atoms per whole AMBA times 10^6 (`postedUsd`/`averageUsd`
in dollars). Atoms are decimal strings.

### `GET /v1/accounts/:owner/stake`

`{ owner, amba (the owner's AMBA balance, atoms), pool, position, chainTime }`:

```text
StakePosition = {
  address, exists, shares, amba (what the shares hold), value (USDC at the average), share,
  active (AMBA atoms not unstaking), unstaking (AMBA atoms unstaking),
  unstakeEndsAt (unix seconds the seven days end, or null when nothing is unstaking),
  claimable (AMBA atoms `unstake` pays now: finished, not backing lent credit, and no market it backs
    in its last day or settling; 0 while one is),
  stakedIn, unstaked,
  pending: { amba, usdc }, paid: { amba, usdc },
  perSecond: { amba }, asOf      [added 2026-10-20]
}
```

**[adjusted 2026-10-18]** `unstakeable` is gone: `claimable` is what may
leave now, `active` what `startUnstake` may take.

* `pending` is what `stakeClaim` pays now, accrued to `chainTime`: `amba`
  from the pool's AMBA stream (`rewards`), `usdc` the stakers' share of the
  maker's profit; `paid` is what was ever paid into the owner's accounts.
* **[added 2026-10-20]** `asOf` is the chain time (unix seconds) the
  figures are for, the response's `chainTime`. `perSecond.amba` (atoms, a
  decimal string) is what the position earns each second from then from
  the AMBA stream: its release rate, `amount / (endTs - startTs)`, times
  the position's `shares` over the pool's `shares`, rounded down once:
  `floor(amount × shares / ((endTs - startTs) × pool shares))`. Every share
  earns, unstaking ones included, and the stream has no house; a position
  of an earlier share epoch counts its converted `shares` (as this read
  reports them). It is `"0"` before `startTs`, from `endTs` on, without a
  stream and without shares. While the shares stay as they are,
  `pending.amba + perSecond.amba × (t - asOf)` for `t` up to `endTs` counts
  the AMBA up live (the program rounds each accrual down, so what it owes
  may differ by a few atoms); read the position again after a stake,
  unstake or claim, or when the pool's shares change. The USDC profit
  share has no rate: it arrives when a market's credit settles
  (`CreditSettle`), so `pending.usdc` moves only then. `stakeClaim` pays
  both at any time.

### Stake transactions (`POST /v1/tx/prepare`)

| action | params | instructions |
|---|---|---|
| `stake` | `amount` (AMBA atoms) | stake from the owner's associated AMBA account |
| `startUnstake` **[added 2026-10-18]** | `amount` (AMBA atoms, or `"all"`/omitted) | startUnstake |
| `cancelUnstake` **[added 2026-10-18]** | none | cancelUnstake |
| `unstake` | `amount` (AMBA atoms, or `"all"`/omitted) | [createAmbaAta], unstake |
| `stakeClaim` | none | [createAmbaAta], [createAta], stakeClaim |

* **`stake`**: `amount` is `1..=` the AMBA balance; the shares are for what
  arrives (AMBA's 0.25% transfer fee comes out). The sponsor pays the fee
  and, the first time, the position's rent.
* **`startUnstake`**: starts the seven days on `amount` atoms of `active`,
  sent as the fewest shares paying at least that (never more than the
  active shares); `"all"` or no `amount`: every active share. More while
  some is unstaking restarts the seven days for all of it. The sponsor
  pays the fee and any rent (an older position grows for the timer). The summary adds `shares`, `amba` and
  `unstakeEndsAt`. 400 `NothingActive` when all of it is already unstaking.
* **`cancelUnstake`**: everything unstaking is staked again (it counts
  toward the capacity again). 400 `NothingUnstaking` without any.
* **`unstake`** **[adjusted 2026-10-18]**: pays finished unstaking AMBA.
  `amount` atoms are sent as the fewest unstaking shares paying at least
  that, up to `claimable`; `"all"` or no `amount`: all of it. Refused with
  400 `NothingUnstaking` (nothing unstaking), `NotSettled: unstaking ends
  <date>` before the seven days end, and `CreditLimit` when the stake left
  would not back the credit lent ("… the rest waits for credit to come back
  as markets settle"; `"all"` while only part is claimable).
  **[adjusted 2026-10-18]** Also `NotSettled: the unstaked AMBA waits until
  the markets it backs settle` while a market holding the pool's credit is
  in its last `expiryWindowSecs` or past expiry with its credit unsettled
  (or the pool, from before it counted its markets, lends anything), and
  `MarketsUnknown` while the API doesn't know every market holding the
  pool's credit. The `unstake` instruction carries those markets
  (read-only) after its fixed accounts.
* **`stakeClaim`**: pays the position's pending AMBA and USDC into the
  owner's associated accounts. 400 `NothingOwed` without any.

### Owner-paid stake transactions [added 2026-10-19]

**[adjusted 2026-10-19]** The stake actions take `params.payer: "owner"`
as every action does (see "Owner-paid transactions"; until this change
they were the only ones): the owner pays the fee, the position account
(`stake`), an older pool's or position's growth (`startUnstake`),
`createAmbaAta` and `createAta`, and the transaction starts with
`SetComputeUnitLimit(200_000)` instead of 1,400,000.

## Portfolio [added 2026-10-24]

What an owner's Earn deposit and stake are worth over time, in USDC, for
the Earn page's portfolio chart. A sampler values every owner it knows every
`FUTURES_PORTFOLIO_INTERVAL_SECS` (900 s) from one batched account read and
keeps the rows for good (`futures.portfolio_samples`). It knows the owners
who deposited into Earn (its events, read at start) and those who read an
Earn or stake position, or prepare `earnDeposit` or `stake`, through this
service (stake events are not indexed). An owner holding nothing gets no
rows, except one when they stop holding.

### `GET /v1/accounts/:owner/portfolio?from&to&points`

```text
{ owner,
  now: { t, total, earn, stake, unclaimed, stakeAmba, unclaimedUsdc, unclaimedAmba },
  points: [{ t, total, earn, stake, unclaimed }] }
```

* `t` is wall time (unix seconds); values are atoms as decimal strings:
  `earn` the Earn deposit at what withdrawing every share pays now (the
  Earn read's `valueLow`), `stake` the AMBA the stake shares hold
  (`stakeAmba`, AMBA atoms; unstaking included) at the stake pool's average
  price (the stake read's `value`), `unclaimed` what was earned and not
  claimed yet: the stakers' USDC profit share (`unclaimedUsdc`) plus the
  AMBA of both streams (`unclaimedAmba`) at that price. `total` is all
  three. Without a stake pool, AMBA is counted but not valued.
* `points` are the samples in `[from, to]` (default: all of them), oldest
  first: the span they cover cut into `points` (default 1,000, at most
  20,000) equal slices, the last sample in each; then `now` as the last
  point unless `to` is in the past. Without a database, only `now`.

## Stream [added]

### `GET /v1/stream?market=<market>&owner=<pubkey>`

Any parameter may be omitted, but not both. `market` names a market
(address, symbol, label or series key, as `:market` of
`/v1/markets/:market`).
`instrument` is accepted as another name for it, and both may be given (an
unknown one is 404).

* `owner` adds `event: tx` events for that owner's submitted transactions.
  Data is the `/wait` body without `timedOut` and `waitedMs`. An event is
  sent on every change: processed, confirmed (with events), failed, failed
  with logs, or expired.
* `owner` also adds `event: wallet` events, `{ owner, usdcAtoms, slot }`,
  for the owner's USDC ATA: one with the current balance on connect, then
  one whenever the balance changes (pushed from the ATA's websocket
  subscription at `processed`, within milliseconds). The ATA is followed for
  as long as a stream for that owner is open.
* `owner` also adds `event: walletCompressed` events, `{ owner, usdcAtoms,
  lightAtaAtoms, compressedAtoms, slot }` (`/v1/wallets/:owner/compressed-usdc`
  without `lightAta` and `decimals`): one once it is read after connect
  (the stream's other events never wait for it), then one whenever either
  amount changes. The Light ATA is followed like the ATA; the compressed
  token accounts are read again when the owner's ATA changes while they
  hold USDC (a decompress lands there), once Photon has indexed that slot.
* Responses carry `Cache-Control: no-cache, no-transform` and
  `X-Accel-Buffering: no`, so proxies and Next.js compression do not buffer
  them.
* **[adjusted]** A request whose `Accept-Encoding` names `gzip` (browsers,
  Node's `fetch`) gets the stream gzip encoded (`Content-Encoding: gzip`;
  every response carries `Vary: Accept-Encoding`). It is one gzip member in
  which every event is deflated on its own and sync-flushed: each one is
  sent, and decodes, whole, the moment it is ready, as before. A busy
  market's `book` event is about 10 KB that way (123 KB as text). The
  member's trailer is written only if the stream ends, which it does not
  while the client stays: a client sees its end as a closed connection, as
  before. `no-transform` keeps proxies (Next.js, Cloudflare) from decoding
  or encoding it again.
* **[adjusted]** `book`, `trade` and `account` were `rangeBook`,
  `rangeTrade` and `rangeAccount` (see "Renamed from range"); the retired
  clearing markets' own `book` and `trade` events are gone.

Server-sent events, for each followed market:

* `event: book`: data is the market's live sources as
  `/v1/markets/:market/book` lists them, with what a book display reads of
  each: **[adjusted]**

  ```text
  { market, slot, sources: [{ source: { kind: "order", slot } | { kind: "maker", maker (slot) },
                              side ("long"|"short"), lo, hi, loValue, hiValue, lots, entryValue }] }
  ```

  One is sent on connect, then one on every change of the market account
  (pushed from its websocket subscription at `processed`). It was the whole
  `/book` body (about 264 KB on a busy market, 217 KB of it the sources):
  the orders, ladders, and each source's `owner`, `price`, `seq`, `step`
  and `tier` are left out, since the event goes to every stream on every
  change. Read `/book` for them.
* `event: trade`: data is one `/v1/markets/:market/trades` entry plus
  `market`, sent as the indexer or the transaction tracker stores the
  trade (a tracked transaction's at confirmation).

With `owner`:

* `event: account`: data is the owner's entry of `/v1/accounts/:owner`
  for one market (with `retired`) plus `owner` and `slot`, sent whenever their own state there changes: their trader's
  cash, orders, claims or ladder, their Earn pool shares, or the market's
  status, settlement or tradability. Moves of the rest of the book (which
  change `exit` and pool values) do not send one; `book` carries
  those. Nothing is sent on connect: open the stream, then read
  `/v1/accounts/:owner` once.

Keep-alive comments are sent every 15 s. Polling
`/v1/markets/:market/book` and `/trades` every 1 to 2 s is equivalent.

## News [added]

### `GET /v1/news`

The home page's news links from Lithos (`https://lithosgraphein.com`): its
six pages (index, ai, robotics, materials, space, energy) fetched together,
each page's external story links (fragments and tracking parameters
removed) with their images, merged and deduplicated (at most 256 stories and
256 source links).

```text
200 { ok: true, data: { sourceSystem: "lithos", fetchedAt (ISO time), pageCount, fetchedPageCount,
                        notModifiedPageCount, failedPageCount, storyLinkCount, sourceLinkCount, totalLinkCount,
                        stories: [{ url, title, category, pageUrl, writtenUtc, imageUrl, imageSrc, imageAlt }],
                        storyLinks: [url], sourceLinks: [url], links: [url],
                        pages: [{ pageUrl, category, fetchedAt, requestStatus ("fetched"|"failed"), stories, storyLinks,
                                  sourceLinks, writtenUtc, defaultImageUrl, error }] } }
502 { ok: false, message, code ("UPSTREAM_LITHOS_UNAVAILABLE" | "UPSTREAM_LITHOS_EMPTY") }
```

The body is a news-link document (stories, sources and freshness) with no
protocol block. Pages are fetched on request, not in the background:
a snapshot is served for 5 minutes, then the next request refreshes it (one
refresh shared by all waiting requests, 7 s for all six pages). When a
refresh fails, the previous snapshot is served for up to an hour. Settings:
`LITHOS_ORIGIN`, `LITHOS_REQUEST_TIMEOUT_MS` (1000..10000),
`LITHOS_CACHE_TTL_MS` (30000..900000), `LITHOS_STALE_TTL_MS`
(300000..86400000) and `LITHOS_MAXIMUM_PAGE_BYTES` (65536..2097152, default
1 MiB).

## Oracle [added]

Two documents from the oracle program (`programs.oracle`), prepared in
memory every 30 s and served from there (no RPC per request). Both use the
`{ ok: true, data }` envelope; before the first read since start they answer
503 `{ ok: false, code: "ORACLE_SNAPSHOT_PREPARING", message }`.

```text
Protocol  = { programId, namespace: "ameba-oracle-v1", cluster, releaseTag, releaseCommit }
Freshness = { source: "futures-api", observedAt (ISO, the last good read), observedAtSlot (string),
              ageMs, maximumAgeMs (120000), stale (ageMs > maximumAgeMs),
              refreshHealthy (the last read succeeded), readReady (healthy and not stale),
              tradeReady: false, lastAttemptAt (ISO), lastErrorCode (null | "ORACLE_RPC_UNAVAILABLE") }
Fact      = { marketId ("ramx"), expiryId ("RAMX-202610"), onChainAvailable: true, oracleMonthAvailable,
              lastUpdatedSlot (string|null), settlementStatus ("Final"|"Provisional"|"FrozenPendingEvidence"|null),
              phase ("SourceSubmission"|"Scramble"|"Opening"|"Game"|"Settled"|"Closed"|null),
              sourceCount (number|null), oracleMonthAddress,
              schedule: { scrambleStartUtc, listingUtc, expiryUtc, finalizedUtc } | null,
              coverage: { requiredSkus, coveredSkus, finalized, plannedScrambleStartUtc, plannedListingUtc } | null,
              rewards: { phase ("Building"|"Funded"|"EntitlementsFinalized"|…), totalAtomic, remainingAtomic } | null,
              skus: [{ bucketId (hex), bucketLabel (string|null), bucketName (string|null), sourceBudgetAtomic,
                       openingBudgetAtomic, updateBudgetAtomic, listingBondAtomic, supportBondAtomic,
                       openingBondAtomic, updateBondAtomic (strings) }],
              buckets: [Bucket] | null }                  // [added] a commodity-bucket month's; "Commodity buckets"
Bucket    = { bucketId (hex), bucketLabel ("DDR5_16GBIT"), bucketName ("DDR5 16Gb"), group ("DDR5"),
              weightBps (number), weightSource ("recipe"|"spec") }
Routing   = { status (a spec status: "ELIGIBLE_FOR_CONTRACT", …), reasons: [{ status, field, message }] }
```

* One fact per oracle `Period` (product, expiry): the manifest's
  `oracle.periods` when it lists any (other periods of the program, such as
  test periods, are not served unless `FUTURES_DISCOVER_INSTRUMENTS=1`),
  else every period of the program. `marketId` is the
  lowercase product id; `expiryId` is `PRODUCT-YYYYMM` with the UTC month of
  `expiry_ts - 1` (the futures label month: 1793491200 → `202610`).
* **[added]** The month's calendar (`schedule`, ISO times, `null` for an
  unset one): Scramble's day 1 (`scrambleStartUtc`; placement days 1-4, the
  kill window days 5-6, the freeze on day 7), its listing (`listingUtc`:
  Opening ends and Game begins), its period's expiry and when it was
  finalized. `coverage` is the month's SKU coverage manifest (a month formed
  the ordinary way; `null` for one created in Game): how many of its products
  have a supported source and whether coverage is finalized (which ends the
  source window). `rewards` is its USDC reward schedule (`null` without one:
  no rewards). `skus` lists, only while the month forms (SourceSubmission,
  Scramble, Opening) and has a schedule, the products it takes sources for:
  its SKU pools (`getProgramAccounts`, 240 bytes, month at offset 38, each
  at its PDA under the schedule), by `bucketId`, with each product's reward
  budgets and bonds; `[]` otherwise. Their `bucketLabel`, like a source's, is
  the governed label, or for a product with no governed label set the
  bucket id's own padded ASCII; `bucketName` is the commodity bucket's name
  on a commodity-bucket month, else `null`.
* **[added]** `buckets` is a commodity-bucket month's buckets
  ("Commodity buckets"), in the spec's order, each with its manifest group
  and weight: the weight its recipe sources carry once the month froze
  (`weightSource: "recipe"`, the largest `bucketWeightBps` among its
  Frozen, OpeningPending, Active or Inactive sources), else the spec's
  (`"spec"`: before the freeze, or for a bucket the recipe has no source
  in). `null` for a month on part numbers (and for a product without
  commodity buckets).
* `oracleMonthAddress` is the period's canonical month PDA
  (`[ameba-oracle-v1, g3-oracle_month, period, expiry_le]`), also when the
  month does not exist yet (`oracleMonthAvailable: false`, the month fields
  `null`, as for a Forming period).
* Every account is checked before use: owner, exact length, layout version,
  zero padding, stored bump and PDA (periods, months, sources, locator links
  and objects). Accounts that fail are left out and logged.

### `GET /v1/oracle/public-snapshot`

```text
200 { ok: true, data: {
  protocol: Protocol,
  state: { status ("current"|"empty"), markets: Fact[], months: Fact[], latest: Fact|null, history: Fact[] },
  markets: Fact[],                       // equal to state.markets
  freshness: Freshness,
  sourceLinks: [{ marketId, expiryId, oracleMonthAddress, phase, sourceCount, observedAt,
                  usage: "source-display-only",
                  sources: [{ sourceAddress, sourceId (hex), bucketId (hex), bucketLabel (string|null),
                              bucketName (string|null), series (SeriesDefinition|null), routing (Routing|null),
                              canonicalLocator: { url, hash, verified: true } | null, proposer, status,
                              baselineStateAtomic, currentStateAtomic, supportStakeAtomic (strings),
                              bucketWeightBps, openingSubmitted, lastFinalizedStep, observationCount (strings) }] }] } }
```

* `markets` and `months` are the same facts, by market then expiry.
  `history` is the facts with a month, highest `lastUpdatedSlot` first;
  `latest` is its first entry.
* `status` is `"empty"` when the oracle program has no periods (so no
  months), and also while no oracle program is configured: `markets`,
  `months` and `history` are `[]`, `latest` is `null`, and freshness is
  read-ready. It is a valid, current answer ("no oracle months yet"), not an
  outage: show the empty state, not an error.
* `sourceLinks` has one inventory per month (every month, by `expiryId`):
  the month's sources (`getProgramAccounts`, 352 bytes, month at offset 2;
  for a Game month, read again by key while its `sourceCount` holds), by
  `sourceId`. While a month takes sources (SourceSubmission, Scramble) the
  inventory is every verified proposal: the month's own `sourceCount` counts
  only supported ones, so a proposal still waiting for a supporter is listed
  too. From the freeze on it is the month's recipe (sources Frozen,
  OpeningPending, Active or Inactive; proposals left out of it, rejected,
  merged, timed out or never supported, are not listed), and it is published
  only when complete (as many as the month's `sourceCount`); otherwise it is
  left out and the series has no inventory. The inventory's own
  `sourceCount` is how many it lists.
* `canonicalLocator` is the sealed locator object the source's role-0
  evidence link points to, when its bytes hash to the source's
  `canonical_locator_hash` (`SHA-256("locator" || url)`) and the URL is
  public HTTPS; else `null`.
* `bucketLabel` is the governed SKU label whose zero-padded ASCII bytes
  equal `bucketId`, from the label set the month uses ("Commodity buckets":
  its part numbers, `governance/*_product_sku_manifest.v1.json`, or its
  commodity buckets); for a product with no governed label set,
  `bucketId`'s own zero-padded printable ASCII; else `null`. `status` is the
  source status name (`Active`, …).
* **[added]** On a commodity-bucket month, `bucketName` is the bucket's
  name, `series` the source's comparison series (its definition object
  parsed, `null` when it is not one or not on chain) and `routing` its
  routing (`null` when its definition is not on chain); all three are
  `null` on other months ("Commodity buckets").
* A source has no proposal time: the oracle program stores no creation
  slot or timestamp in a source, its evidence links (a proposal's link time
  is 0) or a sponsorship agreement. Off-chain candidates have `createdAt`
  (`GET /oracle/source-candidates/recent`).

### Commodity buckets [added]

On 2026-10-20 the user moved RAMX and NANDX from module and SSD part
numbers to commodity chip buckets for every month expiring after 2026-11-01
(`oracle/governance/commodity_submission_spec.v2.json`; the crate
`oracle/crates/commodity-spec` is its code, compiled in). NANDX's buckets
were 3D NAND wafer assessments until 2026-10-22, when the user made them
packaged NAND chips like RAMX's (spec v2; same labels, so the same SKU
list). A bucket is an on-chain SKU (id: its label, zero-padded to 32 bytes)
of the product's bucket manifest (`governance/ramx_product_sku_manifest.v2.json`,
`nandx_product_sku_manifest.v3.json`, `batx_product_sku_manifest.v1.json`);
its `group` is the manifest group it is in (RAMX `DDR5`/`DDR4`, NANDX
`TLC`/`QLC`); a
comparison series is one source, whose definition object holds the series'
definition text; an observation is one opening or update print of that
source ("Oracle observations"). Months formed on part numbers keep them
(the v1 manifests stay compiled in). Every product the crate names is
served the same way.

* **Which labels a month uses.** The governed set (part numbers or
  buckets) its SKU coverage manifest commits to (`required_sku_root`).
  Without one that matches, and while the product's on-chain SKU manifest is
  absent or registers one of the product's governed roots: the set of the
  month's era (buckets for a month expiring after 2026-11-01, part numbers
  before), unless every SKU the month holds (its SKU pools' and sources'
  bucket ids) is in the other set and not all in that one. Else no labels.
  A month whose set is a bucket set is a **commodity-bucket month**.
* **Definitions.** On a commodity-bucket month each source's definition
  object is read from chain: its role-1 evidence link
  (`[ns, "g3-evidence-link-v1", source, [1], source_definition_hash]`, the
  source as event and parent) points to a sealed kind-2 object
  (`[ns, "g3-evidence-bytes-v1", payer, [2], hash]`) whose bytes hash to the
  source's `source_definition_hash` (`SHA-256(bytes)`). petri's wrapper
  (`"ameba-oracle-source-definition-v1" || 0 || u32 LE length || text`) is
  stripped, and so is a web proof's frozen selector ending the text (`;
  Evidence: web proof, selector <64 hex>`, "Oracle source candidates");
  text whose `schema` is `amoeba.comparison-series.v1` is parsed as a series
  (`series`), anything else is not one. The links and objects
  ride in the two `getMultipleAccounts` the locators already take (no extra
  call), and a definition, named by its hash, is read once and kept in
  memory for good. One whose link or object is not on chain is looked for
  again at the next read. Months on part numbers read none.
* **Routing** (`routing`, the crate's `route_series_in_month`): the month's
  listed sources (its inventory) in source-id order (the chain keeps no
  proposal order, so this is the order anyone can rebuild from chain
  state), stage open while the month takes sources (SourceSubmission,
  Scramble) and committed from the freeze on, each source committed when it
  is in the recipe. A series registered under another bucket than its
  series names is `WRONG_PRODUCT` (`field: "bucket"`). A definition on
  chain that is not a series is `NEEDS_METADATA` (`field: "definition"`,
  "not a comparison series"); a source whose definition object is not on
  chain (a dev-bootstrapped source has only its locator link) has `series`
  and `routing` `null`, as petri routes it. Routing classifies; it never
  removes a source from the inventory.

### `GET /v1/oracle/catalog`

```text
200 { ok: true, data: { protocol: Protocol, freshness: Freshness,
  markets: [{ marketId, onChainAvailable: true, expiries: [{ expiryId, settlementUtc (ISO of expiry_ts) }] }] } }
```

Each product's periods, from the same read as the snapshot (same protocol
and freshness), for the research service.

### Manifest

`futures-ops bootstrap` writes the oracle into the deployment manifest:

```json
"oracle": {
  "program": "<oracle program id>",
  "releaseTag": "localnet-dev",
  "releaseCommit": "<oracle source commit>",
  "periods": { "RAMX-1793491200": "<Period>", "NANDX-1793491200": "<Period>" },
  "months":  { "RAMX-1793491200": "<OracleMonth>", "NANDX-1793491200": "<OracleMonth>" },
  "sponsorLookupTables": { "RAMX-1796083200": "<address lookup table>" }
}
```

Keys are `<SYMBOL>-<expiry unix seconds>`. `months` may be omitted (each
period's canonical month PDA is used); `releaseTag` and `releaseCommit` are
optional (reported as `unreleased` and `unknown`). **[added]**
`formingSeries` (series keys, written by `futures-ops open-month`) and the
older `forming` (symbols) mark the series that form the ordinary way; they
are in `periods` and `months` too and are served like any series. The
service reads them only to take a forming series' changes in place
("Deployment changes"). **[added 2026-10-21]** `sponsorLookupTables`
(written by `futures-ops open-month` and `sponsor-table`) names a month's
sponsorship lookup table, owned by the address lookup table program, which
holds the month's period, month, coverage manifest, reward schedule, carry
period, the system program, the oracle's gate when it is governed, then
every SKU pool by bucket id; an entry that is not a series key and an
address is left out (logged). The section drives
`programs.oracle`, the index months and the `/v1/oracle/*` reads.
`FUTURES_ORACLE_PROGRAM` names the oracle program for the reads only (not the
index) while the manifest has no `oracle` section.

## Oracle source candidates [added]

Gasless researcher registration and source candidates, for the frontend's
source discovery page (`/oracle/candidates`, through its
`/api/oracle/candidates/<path>` proxy) and the Petri research service.
These paths are at the API root, not under `/v1`, with the paths, statuses
and envelopes of the old server (:4100), so consumers change only their base
URL. Stored in Postgres (`futures.oracle_candidate_agents`,
`futures.oracle_candidates`, `futures.oracle_candidate_rate`; a redeploy keeps
them). No request signs or submits a transaction, and a candidate has no
oracle weight. Sponsorship (oracle tag 30/22) puts a candidate on the oracle:
`prepare` returns the transactions unsigned, and the sponsor's wallet pays
for, signs and sends them (`/rpc/sendTransaction` relays them).

Replies are `200|201 { ok: true, data: { protocol: Protocol, ... } }` (the
oracle's `Protocol`, as in "Oracle") or `{ ok: false, message, code }` with
the status below; every reply has `Cache-Control: no-store`. Until the
database is ready every route but `GET /oracle/sponsorships` answers 503
`ORACLE_CANDIDATE_STORE_UNAVAILABLE`.

**[added 2026-10-21] Only the oracle's products.** These routes serve the
products and months the oracle serves: those of the manifest's
`oracle.periods` (every one while it lists none, or with
`FUTURES_DISCOVER_INSTRUMENTS=1`), read on each request, so a month that
`open-month` or `list-next` adds is served at once. A candidate whose
`marketId` (any case) is not one of those products is refused at
submission (422 `ORACLE_SPONSOR_REJECTED`, "`<MARKET>` isn't one of the
oracle's products."); one stored before is in no list or feed page and is
not found by its content hash (404, so it can't be sponsored either) while
its product isn't served. Nothing is deleted. `GET /oracle/sponsorships`
lists only the served months' agreements. A product a dev-chain rehearsal
left on chain reaches no page.

```text
Agent     = { agentId (64 hex), payoutAuthority (base58 pubkey), createdAt (ms) }
Details   = { marketId, expiryId, bucketId, canonicalUrl, sourceType, productDefinition,
              priceField, region, currency, evidence }            // all strings
Candidate = { agentId, payoutAuthority, contentHash, duplicateKey, details: Details,
              sourceDefinition, sourceTypeHash, canonicalLocatorHash, sourceDefinitionHash,
              authentication?: Authentication,                  // [added 2026-10-20]
              createdAt (ms), expiresAt (ms), observation: null,
              series: SeriesDefinition | null, routing: Routing | null }   // [added]
Authentication = {                                // what the API's own verification found
  method ("web_proof"), bundleDigest, proofDigest, observationId, submissionId,   // 64 hex each
  trust ("development"|"production"), serverTrust ("mozilla_roots"|"development_root"),
  notary: { name, alg ("secp256k1"), key (hex) }, verifierPolicy, verifierPolicyDigest,
  selectorDigest, manifestHash, locator (the proven https URL),
  price: { text (as the source printed it), atoms (string, six decimals) },
  currency: { value, provenance } | null, unit: { value, provenance } | null,
                                   // provenance "authenticated" or "contributor_annotation"
  evidenceType ("catalog_price"|"published_benchmark"), witnessedAt (unix s, notary-attested
  session start), sourceObservedAt? (the date the record printed, text), verifiedAt (unix s) }
```

### `POST /oracle/agents`

Body `{ payoutAuthority }` (exactly that key, at most 1024 bytes,
`Content-Type: application/json`). `201 { agent: Agent, credential }`: the
credential (64 hex) is shown once; only its SHA-256 is stored. The payout
authority is a non-default 32-byte base58 key, else 422. At most 200
registrations per hour per client address (SHA-256 of the peer IP), and
10000 agents.

### `GET /oracle/agents/:agentId`

`200 { agent: Agent & { history: { candidates, accepted, rejected, sponsored } } }`;
404 `ORACLE_AGENT_NOT_FOUND`.

### `POST /oracle/source-candidates`

`Authorization: Bearer <credential>`; body `Details` (exactly the ten
fields, each a string, at most 16384 bytes), optionally with
`authentication: { bundleDigest }` beside them. `201 { duplicate: false,
candidate }`, or `200 { duplicate: true, candidate }` with the earlier
candidate of the same `marketId`, `expiryId`, `bucketId` and `canonicalUrl`
(its attribution is kept). Rules: ids 1-64 of `[A-Za-z0-9_-]`, `bucketId` a
non-zero 64-hex digest, `canonicalUrl` a public lowercase HTTPS URL (no port,
credentials, fragment, local name or secret-looking parameter; never
fetched), `currency` three uppercase letters, the other texts printable and
trimmed (`sourceType` 2-128, `productDefinition` 8-3072, `priceField` 1-128,
`region` 2-128, `evidence` 12-4096 bytes). An `expiryId` with a suffix after
its month (`RAMX-202610-…`) is stored as the product month `RAMX-202610` (the
futures label month).
Limits: 60 submissions per agent per hour (duplicates count), 100 pending
candidates per agent, 50000 in all; a candidate expires from the list after
7 days and is deleted 90 days later.

**[added 2026-10-20] Authenticated evidence.** `authentication` is exactly
`{ "bundleDigest": "<64 lowercase hex>" }`, the digest of a bundle uploaded
to `POST /oracle/evidence`; any other key in it (a `verified` flag, an
envelope, a trust level) or another value is 400
`ORACLE_EVIDENCE_INPUT_INVALID`, and nothing a client says about the
evidence is read or stored. The details are checked first, by the rules
above (a proven page is still refused if its URL is not public). Then the
API loads the stored bundle (422 `ORACLE_EVIDENCE_REJECTED` if none was
uploaded), verifies it again under its current policy, and checks it fits
the candidate (422 `ORACLE_EVIDENCE_REJECTED` with the reason otherwise):
the proof's locator (`https://<server><path and query>`) must equal
`canonicalUrl`; a proof bound to a wallet must be bound to the agent's
`payoutAuthority`; quotations and transaction records are not admitted as
sources. Each observation is credited once per scope
(`candidate:<marketId>:<expiryId>:<bucketId>`, the stored expiry id): a
candidate whose `submissionId` an earlier candidate already holds is `200
{ duplicate: true, candidate }` with that earlier candidate and its
attribution (whatever its URL or agent); the same bundle in another bucket
is another submission. The stored candidate's `authentication` is built only
from the API's own verification. Its source definition freezes the proof's
extraction selector, so a later update's proof can be checked against the
same series: `sourceDefinition` ends with `; Evidence: web proof, selector
<selectorDigest>` (and `sourceDefinitionHash` covers that text), and
`contentHash` takes `selectorDigest` as a thirteenth field. A candidate
without `authentication` hashes and reads exactly as before.

**[added] A candidate with a comparison series.** The body may add
`series`, a comparison series (`amoeba.comparison-series.v1`, the crate's
`SeriesDefinition`: unknown fields are refused, 422). Then only `marketId`,
`expiryId` and `evidence` are required, and:

* `canonicalUrl` is the series' `origin.locator`, `sourceType` its source
  type (`comparison-series/v1:<evidenceClass>`) and `bucketId` the hex of its
  bucket's id (the label zero-padded to 32 bytes); a given value must be the
  same (422 otherwise). Left out, `productDefinition` is the bucket and the
  row (`"RAMX DDR5 16Gb: DDR5 16Gb (2Gx8) 4800/5600"`), `priceField` the
  `statistic`, `region` the `geography` value (or state) and `currency` the
  series' currency; given, they are kept. The usual detail rules apply to
  the result.
* `sourceDefinition` is the series' definition text (compact JSON, unset
  fields left out; at most 4058 bytes, what a source's definition object
  holds, else 422), committed like any definition
  (`sourceDefinitionHash` = SHA-256 of petri's wrapper of that text), and
  `series` is the series as stored.
* `contentHash` commits to the details and the definition text
  (`ameba-oracle-series-candidate-content-v1`), `duplicateKey` to
  `marketId`, `expiryId`, `bucketId`, `canonicalUrl` and the definition text
  (`ameba-oracle-series-candidate-dedupe-v1`): the same series for the same
  month is one candidate whoever submits it; another row, statistic or scope
  on the same page is another.
* `routing` is computed when it is stored and kept with it: against its
  month as the last oracle read served it (`route_series_in_month` over the
  month's listed sources, as a new proposal: before the freeze it fits or
  repeats one; from the freeze on it is `OUTSIDE_COMMITTED_SERIES`); a month
  on part numbers adds `OUTSIDE_COMMITTED_SERIES` (`field: "expiryId"`) to
  the series' own routing; a month not served yet routes the series on its
  own (`route_series`); a market without commodity buckets is
  `WRONG_PRODUCT`. Routing never refuses a candidate.

A candidate without `series` is checked, committed and stored exactly as
before; its `series` and `routing` are `null`. The list, the feed and
`GET …/:contentHash` return both fields for every candidate.

**[added] A series and a web proof.** A candidate may carry both. Its
source definition is then the series' definition text followed by `;
Evidence: web proof, selector <selectorDigest>` (the selector ends the text,
where the keeper reads it; the series' text and the selector together fit
4058 bytes, else 422), `sourceDefinitionHash` covers that text, and
`contentHash` commits to it as the series form's thirteenth field. The
`duplicateKey` stays the series' (its definition text without the
selector): evidence never makes a candidate new, so the same series with
and without a proof is one candidate, and a proof's submission id is
credited once per scope as above. The proof must fit the candidate the
series made (its `canonicalUrl` is the series' `origin.locator`). A source
whose definition carries a selector is read as its series all the same
(the oracle snapshot strips the trailing selector before parsing).

### `GET /oracle/source-candidates?marketId&expiryId&agentId&after&limit`

`200 { candidates: Candidate[], nextCursor }`: unexpired (or observed) candidates by
`contentHash` ascending, after the `after` cursor (a content hash), `limit`
1-100 (default 50; JavaScript `Number` rules); `nextCursor` is the last
`contentHash` of the page, or `null` when it is empty. A repeated parameter
takes its last value; an empty `marketId`, `expiryId` or `agentId` matches
nothing (an empty `after` is no cursor); `expiryId` maps a
per-strike id like submission does. Other parameters, or an `agentId` or
`after` that is not 64 lowercase hex, are 400 `ORACLE_CANDIDATE_QUERY_INVALID`;
a bad `limit` is 400 `ORACLE_CANDIDATE_PAGE_INVALID`.

### `GET /oracle/source-candidates/recent?limit&before` [added]

The live feed of what researchers submit: every stored candidate of the
oracle's products, newest first, for a page that polls the first page and
pages back with the cursor.

```text
200 { candidates: FeedItem[], nextCursor (string|null), onChainObservedAt (ms|null) }
FeedItem = Candidate & {
  expired,                                   // expiresAt <= now; listed until deleted (90 days later)
  sponsorship: Sponsorship | null,           // as GET /oracle/sponsorships lists it
  onChain: { sourceAddress, sourceId (hex), expiryId, status, proposer } | null }
```

* **Order and paging.** By `createdAt` descending, then `contentHash`
  descending; expired candidates are included (`expired: true`; the list
  above leaves them out). `limit` 1-100 (default 50; JavaScript `Number`
  rules, as in the list). `before` is a cursor, `<createdAt>-<contentHash>`
  (decimal ms, `-`, 64 lowercase hex): the page starts after that candidate.
  `nextCursor` is the last item's cursor when the page is full, else `null`
  (a full last page is followed by an empty one). New candidates join at the
  front, so a cursor never skips or repeats one. A repeated parameter takes
  its last value; an empty `before` is no cursor. Any other parameter, or a
  malformed `before`, is 400 `ORACLE_CANDIDATE_QUERY_INVALID`; a bad `limit`
  is 400 `ORACLE_CANDIDATE_PAGE_INVALID`. Any other method is 405.
* **The chain join** costs no RPC per request: it is the last oracle read
  (the one behind `/v1/oracle/public-snapshot`, every 30 s; its time is
  `onChainObservedAt`, `null` before the first, when every `sponsorship` and
  `onChain` is `null`). `onChain` is the served months' verified source
  (any status, a proposal left out of a month's recipe too) whose source id is
  the content hash (how sponsorship and `petri oracle sponsor` propose it),
  else one of the candidate's series (`details.expiryId`, compared as
  upper-case letters and digits) with its `bucketId` and
  `canonicalLocatorHash` (the same find proposed directly under another id).
  `status` is the source status name. `sponsorship` is the candidate's
  agreement (`candidate` = its content hash): the read takes the agreements
  of the sources whose proposer is their agreement PDA (a
  `SponsoredActivate` proposes for its agreement) by key, one
  `getMultipleAccounts` per 100, none while no source is sponsored; a failed
  read keeps the last ones. An agreement whose source is outside the served
  months, or was closed, is not found (`null`); a sponsorship prepared but
  not yet landed has none either.
* One indexed query (`oracle_candidates_recent`, `(created_at,
  content_hash)`), read-only and without a credential, like the list.

### `GET /oracle/source-candidates/:contentHash`

`200 { candidate }` (also once expired, until deleted); 404
`ORACLE_CANDIDATE_NOT_FOUND`. Any other method or path under these
prefixes is 405 `METHOD_NOT_ALLOWED`.

### `POST /oracle/source-candidates/:contentHash/prepare` (and `/refresh`)

Sponsors a known candidate (404 otherwise), as `petri oracle sponsor` does.
Body `{ sponsor }` (exactly that key, at most 1024 bytes, `Content-Type:
application/json`): the sponsor's wallet, a canonical base58 key on the
ed25519 curve, else 400 `ORACLE_SPONSOR_INPUT_INVALID`. `refresh` is the same
request: it prepares what is left to send now (uploads already sealed are
skipped, an unfinished one resumes).

```text
200 { sponsorship: {
  candidate, sponsor, marketId ("ramx"), expiryId ("RAMX-202612"), sku (the bucket's label | null),
  period, month, source, agreement, bondAtoms, collateralAtoms, outstandingAtoms, researcher, agentId,
  evidence: { locator, definition },          // the sponsor's evidence objects
  lookupTable (the month's table the activation resolves through | null),   // [added 2026-10-21]
  blockhash, lastValidBlockHeight,
  transactions: [{ label, steps: [name], version ("legacy"|"v0"), transaction (base64, unsigned) }] } }
```

* **Reads.** The candidate's period: the oracle's periods whose product is
  `details.marketId` and whose label month (`expiryId`) is
  `details.expiryId`, a Live one first, then Forming, then the nearest
  expiry. Then, in one read, its month, SKU coverage manifest, the product's
  SKU reward pool, the sponsor's oracle collateral, the source (id: the
  content hash), its agreement, the sponsor's two evidence objects and its
  exposure to the researcher, and the month's lookup table when the
  manifest names one; and, for a month outside the governed SKU set, the
  month's sources.
* **Refusals**, in this order: the commitments (`sourceTypeHash`,
  `canonicalLocatorHash`, `sourceDefinitionHash`) must match the details (422
  `ORACLE_SPONSOR_REJECTED`); no period or month, a month not in
  `SourceSubmission` or `Scramble`, or no reward pool for the product (409
  `ORACLE_SPONSOR_CLOSED`); the source or its agreement exists (409
  `ORACLE_SPONSOR_EXISTS`); the sponsor's `available_balance` is below the
  pool's `listing_bond` (409 `ORACLE_SPONSOR_COLLATERAL`); the product is not
  in the month's committed SKU set, an unfinished upload holds other bytes,
  or a transaction is over 1232 bytes on the wire (422
  `ORACLE_SPONSOR_REJECTED`). Messages are plain sentences for people. A chain
  read, the blockhash or the governance gate that is not available is 503
  `ORACLE_SPONSOR_UNAVAILABLE`.
* **Transactions**, in the order they must land (each confirmed before the
  next): `uploadLocator` (kind 1: the locator URL's bytes) and
  `uploadDefinition` (kind 2: the source definition's semantic preimage),
  three 192-byte `writeEvidence` chunks per transaction, none for an object
  already sealed; then `sponsorSource` (`SponsoredActivate`, source id = the
  content hash, the pool's exact `listing_bond`, the SKU index and proof).
  **[added]** A series candidate's definition upload is its definition text
  in the same wrapper, so the hash it registered is the one sealed. The SKU
  proof is against whichever governed set (part numbers or commodity
  buckets) reproduces the month's coverage root. A month of more than one
  SKU needs proof nodes a legacy transaction can't carry next to the
  activation's accounts (RAMX's four-bucket chip month's legacy
  `sponsorSource` is 1234 bytes ungated). **[added 2026-10-21]** So a month whose manifest
  entry names a lookup table (`oracle.sponsorLookupTables`) gets its
  `sponsorSource` as a **v0** transaction (`version: "v0"`) that resolves
  every account the table holds through it: the governed four-bucket RAMX
  chip month's is 1071 bytes. The uploads stay legacy (`version: "legacy"`).
  Deserialize each as a versioned transaction (`VersionedTransaction`);
  `/rpc/sendTransaction` relays either. A table that is not an active
  lookup table on chain (missing, another owner, deactivated) leaves the
  activation legacy (logged), so an oversize one is refused with 422 as
  before.
  Terms: funding 0 (sponsor cash), the cluster's split (`/v1/config`
  `sponsorship`: 80/20 with a fifth of the researcher's part held in their
  reserve, or on mainnet-beta the legacy half), `maximumBond` = the bond, `maximumOutstanding` =
  the sponsor's bonds already out to this researcher (its `SponsorExposure`)
  plus the collateral's `available_balance`, at least the bond. The program
  refuses when the bonds out, this one included, pass that cap, so it never
  binds before the collateral does. Each
  transaction starts with the compute-unit limit (1,400,000); the sponsor is
  its fee payer and only signer, and pays every account's rent. Oracle
  instructions carry the oracle's governance gate where the deployment gates
  it (as `/v1/tx/prepare` binds the futures gate). Sizes are measured on the
  wire (the unsigned transaction's zeroed signature slot included): a
  one-SKU month's `sponsorSource` is 1219 bytes. The blockhash is the cached
  one, so the transactions are good for about a minute.

### `GET /oracle/sponsorships?funder`

`200 { sponsorships: Sponsorship[] }`: every agreement of the oracle program
(`SourceSponsorship`, 320 bytes, marker `ORSPONS1`, at its canonical address)
in a served month (its `month`), or with `funder` (a wallet; the funder is at
byte 73) the ones it paid for, by agreement address. Read from chain on each
request.

```text
Sponsorship = { agreement, source, month, candidate (content hash), funder, reserve, bucketId, sku,
                bondAtoms, outcome ("waiting"|"accepted"|"rejected"|"closed"), rewardSettled, rewardPaidAtoms }
```

Another parameter, or a `funder` that is not a wallet, is 400
`ORACLE_SPONSOR_INPUT_INVALID`.

| Status | Code |
|---|---|
| 400 | `INVALID_JSON`, `INVALID_REQUEST_ENCODING`, `INCOMPLETE_REQUEST_BODY`, `ORACLE_AGENT_INPUT_INVALID`, `ORACLE_CANDIDATE_QUERY_INVALID`, `ORACLE_CANDIDATE_PAGE_INVALID`, `ORACLE_SPONSOR_INPUT_INVALID`, `ORACLE_EVIDENCE_INPUT_INVALID`, `ORACLE_OBSERVATION_INPUT_INVALID`, `ORACLE_OBSERVATION_QUERY_INVALID` |
| 401 | `ORACLE_AGENT_UNAUTHORIZED` (missing, malformed or unknown credential), `ORACLE_OBSERVATION_UNAUTHORIZED` (an observation record not signed by its claim's claimant) |
| 403 | `ORACLE_AGENT_BINDING_MISMATCH` |
| 404 | `ORACLE_AGENT_NOT_FOUND`, `ORACLE_CANDIDATE_NOT_FOUND`, `ORACLE_OBSERVATION_MONTH_NOT_FOUND`, `ORACLE_OBSERVATION_SOURCE_NOT_FOUND` |
| 405 | `METHOD_NOT_ALLOWED` |
| 409 | `ORACLE_SPONSOR_CLOSED`, `ORACLE_SPONSOR_EXISTS`, `ORACLE_SPONSOR_COLLATERAL` |
| 413 | `REQUEST_TOO_LARGE`, `ORACLE_CANDIDATE_TOO_LARGE` |
| 415 | `UNSUPPORTED_MEDIA_TYPE` |
| 422 | `ORACLE_SPONSOR_REJECTED` (invalid payout authority or candidate; a candidate that can't be sponsored as it is), `ORACLE_EVIDENCE_REJECTED` (the named bundle was not uploaded, does not verify or does not fit the candidate), `ORACLE_OBSERVATION_UNVERIFIED` |
| 429 | `ORACLE_CANDIDATE_RATE_LIMIT`, `ORACLE_CANDIDATE_CAPACITY` |
| 500 | `ORACLE_CANDIDATE_STORE_FAILED`, `ORACLE_EVIDENCE_VERIFY_FAILED` |
| 503 | `ORACLE_CANDIDATE_STORE_UNAVAILABLE`, `ORACLE_AGENT_CAPACITY`, `ORACLE_SPONSOR_UNAVAILABLE`, `ORACLE_CANDIDATE_DECISION_UNAVAILABLE`, `ORACLE_OBSERVATION_UNAVAILABLE` |

The old server answered every `ORACLE_*` refusal as `CURRENT_REQUEST_FAILED`
with the message `current request failed` (same statuses); this service
names the refusal.

Limits are set by `FUTURES_ORACLE_CANDIDATE_MAX_AGENTS`, `_MAX_RECORDS`,
`_MAX_PENDING_PER_AGENT`, `_SUBMISSIONS_PER_HOUR`, `_REGISTRATIONS_PER_HOUR`
and `_TTL_MS`. `ameba-futures-api --import-candidates <sqlite path>` imports
the old server's `oracle-candidates-*.sqlite` (read-only) into
`FUTURES_DATABASE_URL` and prints the counts: agents keep their credentials;
candidates are re-admitted (per-strike expiry ids mapped as above, earliest
kept on a collision); rate windows are not imported. It is idempotent.

## Oracle observations [added]

Observation records: the metadata of one opening or update print of a
comparison-series source (the commodity spec's
`amoeba.series-observation.v1`: its period, publication, release and
evidence), which the chain doesn't hold. Each is bound to the print's claim
on chain, routed (the crate's `route_observation`) and kept in Postgres
(`futures.oracle_series_observations`; a redeploy keeps it). Same paths,
envelopes and headers as "Oracle source candidates" (at the root;
`{ ok: true, data: { protocol, … } }`). **[changed 2026-10-21] Signed by
the claimant** (the user's decision): a record carries `signer` and
`signature`, and only the wallet that made the claim can record it; its
value and evidence must be the claim's too, so it can't change what the
print says.

```text
Record = { sourceId (hex), claim (base58), marketId ("ramx"), expiryId ("RAMX-202611"),
           observation: Observation, routing: Routing, recordedAt (ms),
           signer (base58 | null), signature (base58 | null),     // null: kept before records were signed
           chain: { kind ("opening"|"update"), sourceAddress, sourceStatus, committed (in the recipe),
                    claimStatus ("Accepted", "Revealed", …), attempt (opening; else null),
                    claimId (update, hex; else null), claimant, valueAtoms (string), sourceTime (unix s),
                    evidence ("archive"|"hash"|null), slot (string) } }
```

### `POST /oracle/observations`

Body (at most 16384 bytes, `Content-Type: application/json`), exactly:

```json
{ "marketId": "ramx", "expiryId": "RAMX-202611", "sourceId": "<64 hex>", "claim": "<base58>",
  "observation": { "schema": "amoeba.series-observation.v1", "observationId": "…", "rawPrice": "5.25", … },
  "signer": "<base58 wallet>", "signature": "<base58 ed25519 signature>" }
```

**The signature** is the claimant's ed25519 signature, as a wallet signs a
message, over `ameba_commodity_spec::observation_record_message(marketId,
expiryId, sourceId, claim, observation)`: the UTF-8 domain
`amoeba-observation-record-v1\n`, then the compact JSON
`{"marketId","expiryId","sourceId","claim","observation"}` of the fields as
sent (the observation in its canonical field order, unset fields left out).
No hash. A signature that doesn't verify over those bytes for `signer` is
401 `ORACLE_OBSERVATION_UNAUTHORIZED` before anything is read; so is a
`signer` that isn't the claim's claimant on chain (the opening claim's or
update claim's `claimant`). A missing or malformed `signer` or
`signature` is 400 `ORACLE_OBSERVATION_INPUT_INVALID`. Nothing refused is
kept.

`201 { duplicate: false, observation: Record }`, or `200 { duplicate: true,
observation: Record }` with the stored record of the same claim (an opening
claim once per attempt) or of the same source and `observationId`.

* **The month** is the served month of `marketId` and `expiryId` as the
  last oracle read found it (404 `ORACLE_OBSERVATION_MONTH_NOT_FOUND`
  otherwise, also before the first read).
* **On chain**, one `getMultipleAccounts` reads the source (its PDA under
  the month, verified as the snapshot verifies sources; 404
  `ORACLE_OBSERVATION_SOURCE_NOT_FOUND`) and the claim, which must be the
  oracle's and either the source's opening claim (its PDA
  `[ns, "oracle_opening_claim", month, source]`, initialized, a print made:
  not `Empty`) or one of its update claims (`UpdateClaimV2` at its PDA
  `[ns, "oracle-update-claim-v2", month, source, claimant, claimId]`, layout
  marker `UC2` v4, of this month, source and source id, revealed: not `Open`
  or `Committed`, a reveal time and a value). Then:
  `price_atoms(rawPrice)` (6 decimals) must be the claim's value (the
  opening state or the update's new state); `evidenceReference`, when given,
  must be the claim's archived capture (its archive URL: `archive_url_hash`
  of it is the claim's, and the opening or update evidence commitment over
  the month, source, value, source time, the source's locator and definition
  hashes and that URL is the claim's `evidence_hash`), or the hex of that
  `evidence_hash`. Else 422 `ORACLE_OBSERVATION_UNVERIFIED`; nothing is
  kept. The source's definition is read too unless it is kept already
  ("Commodity buckets").
* **Routing** (`route_observation`), stored with the record: the series is
  the source's definition (not a series, or not on chain: the record is
  kept all the same, `NEEDS_METADATA`, `field: "series"`), committed when
  the source is in the month's recipe, the expiry date the month's
  (`YYYY-MM-DD` of its expiry), the prior observations the source's
  earlier records (their `releaseId` and `referencePeriodEnd`, in the order
  they were kept). A source
  registered under another bucket than its series names is `WRONG_PRODUCT`.
  Routing never refuses a record.
* A chain read that fails is 503 `ORACLE_OBSERVATION_UNAVAILABLE`.

### `GET /oracle/observations?expiryId&sourceId`

`200 { items: Record[] }`: the records of a month (`expiryId` as served,
`RAMX-202611`) and/or a source (`sourceId`, 64 lowercase hex), in the order
they were kept. At least one of the two; a repeated parameter takes its last
value. No parameter, another parameter or a malformed `sourceId` is 400
`ORACLE_OBSERVATION_QUERY_INVALID`. Any other method is 405.

## Oracle evidence [added 2026-10-20]

Authenticated evidence for oracle sources (the design is
[evidence/README.md](../evidence/README.md)): a contributor proves what an
HTTPS source showed them, a signed-in page too, as a web-proof bundle
(`amoeba-evidence-bundle-v1`: a TLSNotary presentation and an extraction
manifest), uploads it here and names it in a source candidate. **Trust
model:** a bundle authenticates only if a notary the verifier policy lists
(`/v1/config` `evidence.webProof.notaryKeys`) attested the TLS session and
the server's certificate chains to Mozilla's roots, so trusting the result
means trusting that notary not to collude with the prover; a development
notary or test root makes the whole result `development` trust, never
production evidence. Authentication is not the oracle's decision: an
authenticated quote is still a quote, and the chain's challenge window and
council stay the enforcement path.

**The API verifies every bundle itself** with the verifier petri and anyone
else runs (`amoeba_evidence::verify_bundle`, under the policy `/v1/config`
names) and ignores what clients claim: a bundle carrying a `verified` key or
an envelope is refused by its format, and every field below is recomputed
from the presentation and the manifest. These paths are at the API root,
like the candidates; replies are plain JSON (no `{ ok, data }` envelope)
with `Cache-Control: no-store`, refusals `{ error, code, verification? }`.
Stored in Postgres (`futures.oracle_evidence`: digest, proof digest,
observation id, method, trust, the verification as it was at upload, the
bundle bytes; `futures.oracle_evidence_submissions`: submission id, scope,
observation id, bundle digest, target candidate). Nothing deletes from them.

```text
Verification = amoeba-evidence-verification-v1 {
  format, stages: { received, provenance, extraction, eligibility: { outcome, reason? } },
                  // outcome "passed"|"failed"|"unsupported"|"not_checked"
  envelope?: { format: "amoeba-authenticated-evidence-v1", authentication, origin, extraction,
               observation, times: { witnessedAt, sourceObservedAt?, submittedAt? }, observationId,
               submissionId?, annotations? },
  disclosure?: { serverName, request, hiddenRequestHeaders, responseHeaders, body },
  bundleDigest? }        // evidence/crates/evidence/src/lib.rs; times are { value (unix s, text), provenance }
```

### `POST /oracle/evidence`

The body is the bundle JSON (`Content-Type: application/json`, at most
2 MiB, the verifier policy's bound on hostile input; no credential): a web
proof (`method: "web_proof"`), a DKIM-signed message (`"email_dkim"`) or a
zero-knowledge proof about one that carries no message (`"email_dkim_zk"`,
verified in-process against the compiled-in circuit key; `evidence/README.md`).
The API verifies it (received, provenance, extraction; eligibility is checked
where it is used). Not authenticated: `422 { error, code:
"EVIDENCE_NOT_AUTHENTICATED", verification }`, and nothing is stored.
Authenticated: stored under its digest, the envelope's
`times.submittedAt = { value: <unix s>, provenance: "recorder_clock" }` (the
API's clock), and

```text
200 { bundleDigest, proofDigest, observationId, trust ("development"|"production"),
      stored: true, duplicate (false), artifact: "/oracle/evidence/<bundleDigest>", verification }
```

The digest covers the method, backend, presentation (the message, or the
proof and its claims) and canonical manifest, not the annotations or the
JSON's spacing: a re-upload of the same bundle is
the stored record (`duplicate: true`, the first upload's bytes, verification
and `submittedAt`). Other refusals: 413 `EVIDENCE_TOO_LARGE`, 415
`UNSUPPORTED_MEDIA_TYPE`, 400 `INCOMPLETE_REQUEST_BODY`, 503
`EVIDENCE_STORE_UNAVAILABLE` (no database yet), 500 `EVIDENCE_STORE_FAILED`
or `EVIDENCE_VERIFY_FAILED`.

### `GET /oracle/evidence/:digest`

The stored bundle, byte for byte as first uploaded (`Content-Type:
application/json`, `Cache-Control: public, max-age=31536000, immutable`: a
digest's bytes never change). The public artifact anyone fetches and
re-verifies; no credential. 404 `EVIDENCE_NOT_FOUND`, 400
`EVIDENCE_DIGEST_INVALID` (not 64 lowercase hex), 503 without a database.

### `GET /oracle/evidence/:digest/verification`

The stored bundle verified again now, under the current policy:

```text
200 { bundleDigest, authenticated (bool), verification,
      record: { bundleDigest, proofDigest, observationId, method, trust, createdAt (unix s),
                verification },             // as stored at upload
      submissions: [{ submissionId, scope, observationId, target (candidate content hash),
                      createdAt (unix s) }] }
```

404, 400 and 503 as above.

### `GET /oracle/notary`

A WebSocket (`Upgrade: websocket`) to the API host's notary, the one at
`FUTURES_NOTARY_ADDR` (default `127.0.0.1:4520`; never an address the client
names). Binary (and text) message payloads go to the notary as bytes, and
the notary's bytes come back as binary messages, uninterpreted: the
prover ↔ notary protocol (`evidence/crates/webproof/src/wire.rs`), which
petri's prover speaks to a `ws://` or `wss://` notary URL
(`amoeba_webproof::transport`). The socket closes
when the notary closes; when the client stops sending, the notary's input
is closed and its last bytes still arrive. 503 `NOTARY_UNAVAILABLE` before
the upgrade when the notary can't be reached. The notary bounds its own
messages and sessions; the bridge adds no timeouts or limits of its own
(axum's default WebSocket message size applies).

## Browser RPC gateway [added]

### `POST /rpc` and `POST /rpc/<method>`

The browser's JSON-RPC endpoint (`https://dev.amoeba.farm/rpc`). These paths
are at the API root, not under `/v1`. `/rpc/sendTransaction` is the packet
submission route: it gets the capacity reserved for submissions.

* **Origins.** A request with an `Origin` header must come from an allowed
  origin (`FUTURES_RPC_ALLOWED_ORIGINS`, comma-separated; default
  `https://amoeba.farm`, `https://www.amoeba.farm`, `https://dev.amoeba.farm`,
  `http://localhost:3000`, `http://127.0.0.1:3000`). Requests without an
  `Origin` (servers, the CLI) are served. `OPTIONS` answers the CORS
  preflight with 204. Every answer carries `Access-Control-Allow-Methods:
  POST, OPTIONS`, `Access-Control-Allow-Headers: Content-Type, Solana-Client`,
  `Access-Control-Max-Age: 600`, `Vary: Origin`, `Cache-Control: no-store`
  and, for an allowed origin, `Access-Control-Allow-Origin`.
* **Requests.** `POST` only, no query string, `Content-Type:
  application/json` (optionally `; charset=utf-8`), at most 64 KiB, one
  JSON-RPC 2.0 request object (no batches). Numbers must be canonical safe
  integers and no object may repeat a key. `id` is null, a string of at most
  256 characters or a safe integer.
* **Methods.** 18 are allowed, each with an exact parameter grammar. Cost in
  limiter units in brackets:
  `getSlot` [1], `getGenesisHash` [1], `getMultipleAccounts` [2] (1 to 100
  keys), `getAccountInfo` [2], `getLatestBlockhash` [1], `getBlockTime` [1],
  `getBlockHeight` [1], `getSignatureStatuses` [1] (1 to 256),
  `isBlockhashValid` [1], `getMinimumBalanceForRentExemption` [1],
  `getBalance` [1], `getTokenAccountsByOwner` [2], `getSignaturesForAddress`
  [4], `getTransaction` [4], `getCompressedTokenAccountsByOwnerV2` [4],
  `getValidityProofV2` [4], `getCompressedAccountsByOwnerV2` [4],
  `sendTransaction` [4]. Commitments are `confirmed` or `finalized`; account
  encodings `base64` or `jsonParsed` (`dataSlice` only with `base64`, at most
  4096 bytes); `sendTransaction` takes `[packet, { encoding: "base64",
  skipPreflight?, preflightCommitment?, maxRetries? (0..5), minContextSlot? }]`.
  The admitted request is re-rendered canonically (keys sorted) and sent to
  the validator, or to Photon (`FUTURES_PHOTON_URL`, default
  `http://127.0.0.1:8784`) for the compression methods.
* **Answers.** A forwarded request is answered 200 with the upstream's bytes
  and content type unchanged, including JSON-RPC errors such as a preflight
  failure.
* **`sendTransaction`.** The gateway never signs. A packet whose fee payer is
  this service's sponsor is refused with 409 `GAS_SPONSORSHIP_UNAVAILABLE`
  (sponsored transactions go through `/v1/tx/prepare` and `/v1/tx/submit`).
  When the validator accepts a fully signed packet, the transaction tracker
  resends the same bytes every 300 ms until it is processed or its blockhash
  expires (at most 32 relayed packets are followed at once), so the browser
  can send with `maxRetries: 0`. `GET /v1/tx/:signature/wait` follows a
  relayed signature like a sponsored one.
* **Limits.** At most 16 requests in flight (2 held back for submissions), 4
  per client (1 held back); token buckets of 40 units/s (burst 80) overall
  and 8 units/s (burst 16) per client, with 8 and 4 units held back for
  submissions; a short queue (32, at most 2 s). The client is the
  `cf-connecting-ip` address when the peer is the local tunnel, else the
  peer address. The `FUTURES_RPC_MAXIMUM_IN_FLIGHT_REQUESTS`,
  `FUTURES_RPC_MAXIMUM_IN_FLIGHT_PER_CLIENT`, `FUTURES_RPC_UNITS_PER_SECOND`,
  `FUTURES_RPC_UNIT_BURST`, `FUTURES_RPC_CLIENT_UNITS_PER_SECOND`,
  `FUTURES_RPC_CLIENT_UNIT_BURST` and `FUTURES_RPC_MAXIMUM_CLIENTS` settings
  change them. One request has 15 s (`FUTURES_RPC_REQUEST_TIMEOUT_MS`).

Refusals are `{ "ok": false, "message": string, "code": string }`:

| Status | Code | When |
|---|---|---|
| 400 | `INVALID_RPC_GATEWAY_TARGET` | A query string was sent |
| 400 | `INVALID_RPC_REQUEST` | Not valid bounded JSON, or not one request object |
| 400 | `RPC_ADMISSION_REQUEST_INVALID` | The envelope is wrong (`jsonrpc`, `id`, `method`, extra keys) |
| 400 | `RPC_METHOD_MISMATCH` | `/rpc/<method>` names another method than the body |
| 400 | `INVALID_CONTENT_LENGTH`, `INCOMPLETE_REQUEST_BODY` | Bad or unfinished body |
| 403 | `RPC_ORIGIN_NOT_ALLOWED` | The `Origin` is not allowed (or a preflight has none) |
| 404 | `NOT_FOUND` | `/rpc/<x>` where `x` is not a method-shaped name |
| 405 | `METHOD_NOT_ALLOWED` | Not `POST` or `OPTIONS` |
| 409 | `GAS_SPONSORSHIP_UNAVAILABLE` | `sendTransaction` of a packet paid by the sponsor; nothing was sent |
| 413 | `REQUEST_TOO_LARGE` | Body over 64 KiB |
| 415 | `UNSUPPORTED_MEDIA_TYPE` | Not `application/json` |
| 422 | `RPC_METHOD_NOT_ADMITTED` | The method is not allowed |
| 422 | `RPC_PARAMS_NOT_ADMITTED` | The params do not match the method's grammar |
| 429 | `RPC_GATEWAY_BUSY`, `RPC_GATEWAY_RATE_LIMITED` | Concurrency or cost limit |
| 500 | `RPC_TRANSPORT_FAILED` | The upstream could not be reached |
| 502 | `RPC_UPSTREAM_UNAVAILABLE`, `INVALID_RPC_UPSTREAM_RESPONSE`, `UPSTREAM_RESPONSE_TOO_LARGE`, `INCOMPLETE_UPSTREAM_RESPONSE`, `RPC_UPSTREAM_REDIRECT_REJECTED` | Upstream failure |
| 503 | `RPC_GATEWAY_TIMEOUT` | The deadline passed while waiting for capacity |
| 504 | `UPSTREAM_TIMEOUT`, `CURRENT_EDGE_REQUEST_TIMEOUT` | The 15 s deadline passed |

## MCP [added 2026-10-26]

`POST /mcp`: a hosted [MCP](https://modelcontextprotocol.io) server for AI
agents, over MCP's streamable HTTP transport (protocol `2025-06-18`;
`2025-03-26` and `2024-11-05` are answered in their own version). Claude
Code: `claude mcp add --transport http amoeba https://api.amoeba.farm/mcp`;
Codex: `codex mcp add amoeba --url https://api.amoeba.farm/mcp`; `petri mcp
enable` registers it for both.

* Stateless: no session (no `Mcp-Session-Id`), no server stream (`GET /mcp`
  and `DELETE /mcp` are 405), no authentication. A request is one JSON-RPC
  message, or a batch of them; the answer is `application/json`. A
  notification (or a response) is 202 with no body. A body over 64 KiB is
  413, one that is not JSON 400 (`-32700`).
* Methods: `initialize`, `ping`, `tools/list`, `tools/call`; any other is
  `-32601`. An unknown tool, or arguments that do not fit its schema (an
  unknown or missing argument, a wrong type, a value out of its enum or
  bounds, a market, owner or signature with a character outside
  `[A-Za-z0-9_:-]` and spaces), is `-32602`, before any route is called.
  A market is what `/v1/markets/:market` takes: its address, label, symbol
  or `SYMBOL-<expiryTs>`.
* Each tool is a request to this API's own routes, made in process with the
  caller's address (`ConnectInfo`, `cf-connecting-ip`): the same checks,
  limits and shapes as the routes'. A tool's result is the route's JSON as
  `structuredContent` (and as text); a refusal (non-2xx) is
  `isError: true` with `{ ok: false, status, error: <the route's body> }`.
* It never signs. A sponsored transaction gets the sponsor's signature at
  `/v1/tx/submit`, only after the owner's (see there); an owner-paid one
  goes out through `/rpc/sendTransaction`, which refuses sponsor-paid
  packets.

| Tool | Route |
|---|---|
| `config` | `GET /v1/config` |
| `markets_list` (`all`) | `GET /v1/markets` |
| `market_show`, `market_book` (`market`) | `GET /v1/markets/:market`, `.../book` |
| `market_depth` (`market, lo, hi, max`) | `GET /v1/markets/:market/depth` |
| `market_trades` (`market, limit, before`) | `GET /v1/markets/:market/trades` |
| `market_candles` (`market, resolution, from, to`) | `GET /v1/markets/:market/candles` |
| `market_index` (`market, from, to, limit`) | `GET /v1/markets/:market/index` |
| `quote` (the `/v1/quote` body's fields; `lo`, `hi` default to the market's `suggestion`) | `POST /v1/quote` |
| `account`, `wallet_usdc`, `account_earn`, `account_stake` (`owner`) | `GET /v1/accounts/:owner`, `/v1/wallets/:owner/usdc`, `.../earn`, `.../stake` |
| `account_history` (`owner, limit, before, types`) | `GET /v1/accounts/:owner/history` |
| `portfolio` (`owner, from, to, points`) | `GET /v1/accounts/:owner/portfolio` |
| `earn`, `stake`, `news`, `oracle_catalog` | `GET /v1/earn`, `/v1/stake`, `/v1/news`, `/v1/oracle/catalog` |
| `oracle_months`, `oracle_sources` (`market`, `expiry`) | `GET /v1/oracle/public-snapshot`: `state.months` or `sourceLinks` of that market or month, with `protocol` and `freshness` |
| `oracle_candidates` (`marketId, expiryId, agentId, after, limit`) | `GET /oracle/source-candidates` |
| `prepare_order` (`owner`, the order, `withdraw`, `payer`) | `POST /v1/tx/quote-prepare` |
| `prepare_transaction` (`owner, action, params, payer`) | `POST /v1/tx/prepare` |
| `submit_transaction` (`id, transaction`) | `POST /v1/tx/submit` |
| `send_transaction` (`transaction`) | `POST /rpc/sendTransaction` (`{ encoding: "base64" }`): `{ signature }` |
| `transaction_status`, `transaction_wait` (`signature`, `commitment`, `timeoutMs`) | `GET /v1/tx/:signature`, `.../wait` |

`prepare_order` and `prepare_transaction` add to the route's answer
`howToSign` (who signs it and which tool sends it: `submit_transaction` for a
sponsored one, with its `id`; `send_transaction` for an owner-paid one, its
`before` ones first) and, for a market, `reviewLink`: the month's page on the
site with the ticket on the order's side
(`<site>/trade/<symbol>?expiry=YYYY-MM&side=long|short`), where a person
places the order from their own wallet. `<site>` is `FUTURES_SITE_URL`, else
`https://amoeba.farm` on mainnet and `https://dev.amoeba.farm` elsewhere.
Only `submit_transaction` and `send_transaction` are not `readOnlyHint`.

## Debug [added]

`GET /v1/debug/cache/:address` returns
`{ known, exists, slot, owner, lamports, len, head }`, which is what the
account cache holds for the address (public chain data only).

## Health [added]

`GET /health` and `GET /v1/health` return

```text
{ ok, markets, retiredMarkets, slot, database, chainTime, wallTime,
  logNotes, logNotesMatched, logs: { futures: { notes, matched } } }
```

* `markets` counts the manifest's markets, `retiredMarkets` the retired
  ones. **[adjusted]** `rangeMarkets` (the same count as `markets`) is
  gone and `retiredRangeMarkets` is now `retiredMarkets` (see "Renamed
  from range"). `clearingMarkets` and `logs.clearing` are gone with the
  clearing markets.
* `slot` is the newest slot of any served market's cached account, `null`
  before the first.
* `logNotes` counts the futures program's processed-transaction log
  notifications (`logsSubscribe`), `logNotesMatched` those that matched a
  transaction the tracker follows (the same as `logs.futures`, formerly
  `logs.range`). A matched
  notification resolves `/v1/tx/:signature/wait` at processed with the
  events decoded from the logs (with the prepared `market`), without
  fetching the transaction.

## Deployment changes

The service reads the deployment manifest every 5 s.

* **New listings** are served in place: a manifest that only gains series
  (`oracle.periods`, `oracle.months`) and markets (`futures.markets`),
  as the monthly `futures-ops list-next` writes it. The new markets, their
  periods and Earn pools are read into the account cache and followed; they
  appear in `/v1/config`, `/v1/markets` and every lookup by address,
  label or series key; the registry records them and the indexer follows
  them at its next round; their months join the reference index and
  `/v1/oracle/*`, and their Earn pools the reserved owners. Open streams,
  prepared transactions and caches carry on.
* **[added] Months forming the ordinary way** are served in place too:
  * a month `futures-ops open-month` opens: its series in `oracle.periods`,
    `oracle.months` and `oracle.formingSeries`. It joins `/v1/oracle/*` at
    the next oracle read (every 30 s): a fact in `state.months` with its
    `schedule`, `coverage`, `rewards` and `skus`, and its proposals in
    `sourceLinks`. Sponsoring a find for it (`POST
    /oracle/source-candidates/:contentHash/prepare`) reads its period, month,
    coverage and SKU pools from chain, so it never waits for the manifest;
  * any change to the lists of forming series, `oracle.formingSeries` or
    the older whole-product `oracle.forming` (a series leaving one keeps its
    period and month served: the service reads each month's phase from
    chain, not from these lists);
  * a forming series (in `formingSeries`, or of a product in `forming`) with
    no futures market leaving `oracle.periods` and `oracle.months`, as
    `futures-ops unlist-month` takes it out: it leaves `/v1/oracle/*` and
    the reference index at their next read;
  * **[added 2026-10-21]** any change to `oracle.sponsorLookupTables` (a
    month's table added, replaced or removed): `/v1/config` and the next
    prepared sponsorship use the map as it stands.
* **Any other change** (programs, keys, accounts, the oracle or futures
  program, a listed entry changed or removed, also pruning fields the
  service ignores): the service logs what changed and exits with status 75
  (`EX_TEMPFAIL`; the journal shows `status=75/TEMPFAIL`), and systemd
  restarts it on the new manifest within about 10 s. **[adjusted]** It
  exited with status 0 before, which only the dev unit (`Restart=always`)
  restarts; the production unit (`Restart=on-failure`) restarts a non-zero
  exit, so both units restart it 5 s later (`RestartSec=5`).

A manifest that does not parse yet is read again at the next check; one
whose new listings do not load is logged, and the service keeps serving
the listings it has until the file changes again.

Indexed history is append-only: no deployment change deletes events,
trades, cursors or registry rows. A new event decoder only drops the
cursors, so every market is rescanned and the idempotent inserts add
what was missing. Markets that leave the manifest stay readable (see
"Retired markets").

## Limits and protection

* The sponsor co-signs only messages it built (see submit). It never signs
  for an owner equal to:
  * the sponsor
  * the mint authority
  * a program id
  * the USDC mint
  * any `accounts.*` key in the deployment manifest
  * the futures program and its treasury (`futures.program`, `futures.treasury`)
    and the treasury's Earn pool in each market

  Requests for these owners get 403.
* Quotes, prepares and submits have no per-owner limit. Prepare and
  quote-prepare share one limit across all owners: 12000/min (it guards the
  package solver's CPU; preparing costs the sponsor nothing).
* Faucet (dev only): 3 per owner per hour.
* Each submit costs the sponsor one fee even when the transaction fails,
  because preflight is skipped.
* Prepared messages expire after 90 s.
* Evidence bundles (`POST /oracle/evidence`) are at most 2 MiB, the
  verifier policy's bound on hostile input; there is no rate limit.
* **[added 2026-10-26]** A bot bundle (`POST /v1/bots`) is at most 256 KiB
  of text (413 beyond), a bound on hostile input; a bundle with a long
  script is a few KiB. Its GitHub file is fetched from
  `raw.githubusercontent.com` only (a redirect elsewhere is not followed),
  within 30 s and at most 256 KiB, after its source passed the strict
  character check (see "Bots"). No rate limits, caps or allowlists.


## Leaderboard

`GET /v1/leaderboard?days=7|30|90` (absent: all time) → `{ generatedAt, since, clockOffsetSecs, traders: [{ owner, pnlAtoms, paidAtoms, volumeAtoms, trades, lastTradeAt, unpriced }], index: { latestEventAt, events } }`.

The markets' fills rank the traders: a trader's lots on each exact interval are followed from their fills (`take`, `fill`), an opposite fill closing the oldest lots first, and P&L for a window covers the lots opened in it: sale premium less opening premium for closed lots, plus open lots at what they would sell for now (the executable depth on the same interval: the bids for longs, the asks to buy back shorts). Lots beyond that depth, or in a market that is open but not tradable, count at their payout at the reference index (zero without a reading) and set `unpriced`; a settled or failed market pays its settlement. Ladder fills (the maker treasury's), transfers (the sender keeps the lots) and splits (the lots keep their interval) are not followed. A fill's volume is its lots at their entry, `lo * k + premium`.

Retired markets (see "Retired markets") count like the manifest's.

The deployment's own keys (the reserved owners: every `accounts.*` key of the manifest, the sponsor, the treasury and its Earn pools) are excluded. **[adjusted]** The retired clearing markets' cohorts no longer count. Windows are cluster time; `lastTradeAt`/`latestEventAt` are shifted by `clockOffsetSecs` (wall − cluster) for display.

**[adjusted]** The 7, 30 and 90-day and all-time boards are recomputed in the background, all four from one read of the events (and one solve of each executable depth they share); the next pass starts 10 s after one ends, or twice the time the pass took when that is longer. A request answers at once with the last board (`generatedAt`, chain time, says when it was computed). Only before the first pass, or for another `days`, is a board computed on demand (another `days` is kept 5 s).

## Bots [added 2026-10-26]

The bot hub's registry: people publish petri trading bots (one TOML bundle
each) and install other people's; each running bot trades from a wallet of
its own, registered here, so its on-chain results rank the bot it runs. The
contract with petri is `cli/docs/bots.md` ("Publishing, installing,
ranking" and "Marketplace"); the code is `api/src/bots.rs`.

* **Signatures** are ed25519 over the UTF-8 message, base58 in the body,
  verified against the public key the body names. One that doesn't verify
  is 401; a malformed key or signature is 400.
* **A bundle's hash** is the SHA-256 (lowercase hex) of its text with the
  runner's own lines left out: every line between a `[bot]` header line
  (the line, trimmed, is exactly `[bot]`) and the next table header (any
  line whose first non-blank character is `[`) that matches
  `^\s*(id|market|budget|from)\s*=`. Every other line is kept as it is,
  with its `\n` (a last line without one stays without). The hash is taken
  over the text, never a parsed copy; `[bot] author` is hashed. The
  contract's test vector hashes to
  `03f3aea639cdd13e7bb4580da1df62e8a6b659d546a3204cf0d9a4697741e90a`.
* **Kinds** are the bundle's row kinds as written (`strategy/script`,
  `weights`, …), each once, in order, rows with `disabled = true` left out.
* **Times** (`publishedAt`, samples' `t`, `generatedAt`, the windows) are
  wall time, unix seconds.
* Tables (in schema `futures`, created at startup, append-only: rows are
  only added): `bots` (each id and its author), `bot_versions` (every
  version with its text, signature and what was read from it),
  `bot_verifications` (each GitHub file that verified a version),
  `bot_runs` (every registration; the view `bot_runs_latest` is each
  wallet's latest) and `bot_pnl_samples`.

```text
BotSummary = { id, name, about, author, market, label, symbol, kinds: string[], version, hash,
               publishedAt, verified, github: null | { owner, repo, path, ref, url },
               runs, runners, pnlAtoms, volumeAtoms }
```

The latest version's: `name` (`[bot] name`, else the id), `about` (the
bot's thesis, one line), `market` (the address its `[bot] market` resolved
to), `label` (that market's label; for a product bot, `market = "nandx"`,
which trades the product's front month and rolls, the product symbol) and
`symbol`. `verified`: a GitHub file verified this version (`github` names
it, the latest when several did; `url` is its github.com page). `runs`: the
counted runs (below), `runners` their distinct owners, `pnlAtoms` and
`volumeAtoms` their sums (each wallet's newest sample).

### `POST /v1/bots`

Body `{ text, author, signature, github? }`, signed message `"amoeba bot
publish v1\n" + hash` (the hash of `text`). → `201 { id, version, hash,
publishedAt, verified, github }` for a new version, `200` with the existing
version when the id already has one of this hash.

* The API reads `[bot]` `id` (letters, digits, `-`, `_`), `market`,
  `name`, `about` and `author`, and the row kinds (with `toml_edit`, the
  parser petri uses); other settings are petri's. A text that isn't such a
  bundle is 400.
* `market` resolves as `/v1/markets/:market` does (address, label, symbol,
  series key) or by petri's name for a market (`ramx-nov26`: the label's
  words in lower case, the symbol then a dash), the manifest's markets
  first, then retired ones. An unknown market is 400.
* `[bot] author`, when the bundle has one, must be `author` (400 otherwise).
* The registry id is the bundle's `id`: the first author to publish it owns
  it; another author is 409. The ids `leaderboard` and `runs` are route
  names under `/v1/bots/` and are 400.
* A posted `[bot] from` line is not kept (petri publishes without it and an
  install adds it back; it is not hashed either).
* `github` (optional): `"OWNER/REPO[/PATH][@REF]"` (a leading `github:` is
  allowed), PATH `petri-bot.toml` and REF `HEAD` by default. It is checked
  before any URL is built: OWNER letters, digits and `-` (not first); REPO,
  each PATH segment and each REF segment letters, digits, `.`, `_` and `-`,
  none empty, `.` or `..` (400 otherwise). The API fetches
  `https://raw.githubusercontent.com/OWNER/REPO/REF/PATH`; when the file's
  hash is the posted text's and its `[bot] author` is `author`, the version
  is verified (whoever controls that repository published it). A fetch that
  fails, another bundle there, or another or no author there is 400 saying
  which, and nothing is stored. A version verified once stays verified: a
  later publish of the same hash without `github` answers `verified: true`.
  Without `github`, a new version is unverified.

### `GET /v1/bots?market=&sort=pnl|new|runs&days=`

→ `{ bots: [BotSummary & { rank, returnBps }] }`, every published bot.

* `market` filters by the latest version's market: its address, product
  symbol (every month of it) or label (as here or as petri writes it), or
  the market the filter resolves to.
* `sort`: `pnl` (default; best P&L first, then volume, runs, newest), `new`
  (the newest version first) or `runs` (most counted runs first). Another
  value is 400.
* `days` (7, 30, 90, …): `pnlAtoms` and `volumeAtoms` over that window, as
  the leaderboard computes them; with `sort=pnl` the list ranks by it.
  Absent: all time.
* `rank` is the position in this list (1, 2, …), `returnBps` as on the
  leaderboard.

### `GET /v1/bots/:id`

→ `BotSummary & { text, versions: [{ version, hash, publishedAt, verified }] }`:
`text` is the latest version's as stored, `versions` oldest first. 404 for
an id nobody published.

### `POST /v1/bots/runs`

Body `{ wallet, owner, bot, hash, name, market, walletSignature,
ownerSignature }`, signed message `"amoeba bot run v1\n" + wallet + "\n" +
owner + "\n" + (bot or "") + "\n" + hash` by the bot wallet
(`walletSignature`) and by its owner, the main wallet (`ownerSignature`).
→ `{ wallet, bot, counted }`.

* `bot` is a registry id, or null (or `""`) for an unpublished bot, which
  still gets its P&L. `hash` is the bundle's (64 lowercase hex, else 400).
  `name` and `market` are stored as sent (null: `""`).
* The deployment's own keys (the trader leaderboard's reserved owners and
  the sponsor) can't register a run, as `wallet` or `owner`: 403.
* Idempotent per wallet: the latest registration replaces the earlier ones
  (all are kept; one that says what the latest says adds nothing).
* `counted`: `hash` is a published version of `bot`, so the run counts for
  that bot's ranking. It is worked out when read, so a run registered
  before its version was published counts from then on.
* The wallet's P&L is sampled at once (then every 5 minutes).

### `GET /v1/bots/runs/:wallet?from&to&points`

```text
{ wallet, owner, bot, hash, name, market, counted, pnlAtoms, equityAtoms, depositedAtoms,
  withdrawnAtoms, volumeAtoms, points: [{ t, pnlAtoms }] }
```

The wallet's latest registration (404 without one) and its P&L now:

* **P&L** (every market, the manifest's and the retired ones):
  `withdrawn - deposited + equity`. `deposited` and `withdrawn` sum the
  wallet's indexed `deposit` and `withdraw` events. `equity` sums, in each
  market where the wallet has a trader slot (its cached account), its free
  and reserved cash, each claim valued as `/v1/accounts/:owner` shows it
  (the executable `exit.net` for the lots the exit sells, the rest at
  `payoutAtIndex`, else 0; no exit while the market isn't tradable or is
  retired), and its ladder inventory valued the same way (each step's
  inventory as a one-step claim, long or short, as the program counts it).
* **Volume**: its fills (`take`, and `fill` of its resting orders) as the
  trader leaderboard counts them (`lots * lo * k + premium`, the premium
  at least zero), plus its ladder fills' premium (`ladderFill` as maker).
* `points`: its samples in `[from, to]` (default: all of them), oldest
  first, the span cut into `points` (default 1,000, at most 20,000) equal
  slices, the last sample of each; then the P&L now unless `to` is in the
  past.

Every registered wallet's P&L (with its equity, deposits, withdrawals and
volume) is sampled every 5 minutes, and when it registers, into
`futures.bot_pnl_samples`, never cleared.

### `GET /v1/bots/leaderboard?days=7|30|90`

```text
{ generatedAt, bots: [{ rank, id, name, author, label, symbol, kinds, verified, github, runs,
                        runners, pnlAtoms, returnBps, volumeAtoms }] }
```

The bots with counted runs, best P&L first (then volume, runs, id). Only
counted runs count: each wallet's latest registration whose hash is a
published version of the bot it names, the deployment's own keys left out.
P&L and volume now are each wallet's newest sample; a window's (`days`;
absent: all time) are those less the wallet's newest sample at or before
the window's start (`generatedAt - days * 86400`), zero without one.
`returnBps` is that P&L over what the runs' wallets ever deposited into
markets (their newest samples), in basis points, truncated (0 when nothing
was deposited).
