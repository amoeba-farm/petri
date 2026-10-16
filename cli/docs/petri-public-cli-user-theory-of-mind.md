# Petri Public CLI User Theory Of Mind

Compressed markdown guide derived from the Current release PDF
`Petri_User_Theory_of_Mind_for_Public_CLI.pdf`.

Use this before changing Petri command names, help text, error messages,
examples, JSON/MCP schemas, TUI copy, or public CLI documentation.

Current release note: the local V3 package supports typed governed operations.
The final business audit records completed replay with approved oracle differences. An active finalized gate,
initialized unpaused business state, exact owner/action admission and final
transaction revalidation are required before execution. The source MCP adapter
shares public preparation and explicitly authorized exact-operation execution;
private keys and salts remain local. Integration and tests are deferred.
Local capability is separate from publication, hosted
activation and runtime readiness.

## Core Rule

Build for the public Petri user, not for the founder, the backend, or the agent
reading the repo. Petri is a product surface for market action and verification,
not an x-ray of infrastructure.

If a command exists only because an engineer, operator, or founder needs it, it
does not belong in the public `petri` CLI. Private systems need separate tooling
outside this command surface.

## What Petri Is

Petri is the public interface for standardized hardware markets:

- discovering live hardware markets;
- understanding capped futures: one market per index and expiry, a position
  that pays per point the index settles inside its range, and an up-front cost
  that is the most it can lose;
- viewing price, the order book, settlement, and oracle evidence;
- quoting and executing trades, and closing positions at the book;
- resting limit orders on a range and cancelling them;
- viewing positions, history, and settlements;
- contributing oracle evidence through public source workflows.

The public product should make price, risk, settlement, source evidence,
ownership, and claims legible without asking the user to understand storage,
venue plumbing, service topology, repository layout, or operator workflows.

## The User Model

The CLI is more technical than the website, but it is still public. Assume users
may be financially literate, crypto-native, or automation-oriented. Do not
assume they are Petri developers.

Primary users:

- Curious traders want markets, ranges, current levels, upside/downside, and
  simple trade execution.
- Cyclical speculators want direction, expiry, leverage, max loss, and max
  payout without margin calls or liquidations.
- Market makers want depth on a range, resting limit orders, the cash those
  orders hold, and a quick cancel.
- Hedgers and procurement users want month, product, capped exposure,
  settlement method, and reporting path.
- Oracle contributors want recipes, sources, evidence, bonds, updates,
  challenges, trust, stake, and reward state.
- Agent wrappers want stable JSON, predictable public nouns, and no hidden
  private command grammar.

## User Questions Drive Command Nouns

Every command should map to a question already present in the user's mind.

| User question | Product noun | Good command shape |
| --- | --- | --- |
| What markets exist? | markets | `petri markets` |
| What range can I trade? | market | `petri market RAMX` |
| What does the book look like? | book | `petri book RAMX` |
| What would a trade cost? | quote | `petri quote long RAMX 100` |
| How do I trade? | long, short | `petri long RAMX 100`, `petri short RAMX 100` |
| What do I hold? | positions | `petri positions` |
| How do I get out? | close | `petri close RAMX <#>` |
| How did it settle? | market, history | `petri market ramx-oct26`, `petri history` |
| What is the oracle doing now? | oracle | `petri oracle`, `petri oracle latest ...` |
| What sources support the oracle print? | oracle | `petri oracle recipe ...` |
| Can I contribute evidence? | oracle | `petri oracle sources`, `petri oracle evidence write`, `petri oracle collateral` |
| What wallet/config am I using? | wallet, config | `petri wallet`, `petri config` |

Name commands by the noun in the public user's mind, not by the database table,
backend service, execution venue, route, payload, or metaphor from the repo.

## Public CLI Journey

The CLI should form a coherent action graph:

```text
petri
petri markets
petri market RAMX
petri book RAMX
petri quote long RAMX 100
petri long RAMX 100
petri positions
petri close RAMX <#>
petri history
petri withdraw RAMX all
```

Oracle contribution is a parallel public journey:

```text
petri oracle
petri oracle latest RAMX
petri oracle recipe RAMX
petri oracle sources RAMX
petri oracle collateral
petri oracle sources challenge RAMX --source <ID> ...
```

## Allowed Public Grammar

Keep the public grammar small enough for a human to guess and stable enough for
an agent to call.

| Family | User meaning | Default behavior |
| --- | --- | --- |
| `petri` | Start here | Show a compact front door with markets, wallet, status, and next actions. |
| `markets` (`market`) | What markets exist and what is live? | List markets; show one market's price, index, suggested range and settlement. |
| `book`, `trades`, `chart` | What is the market doing? | Show one range's depth, recent trades, and the price chart. |
| `quote` | What would a trade cost? | Price an order: range, leverage, entry, what you pay (the most you can lose) and the most it pays. Nothing is sent. |
| `long`, `short` | Can I trade? | Quote first, show the cost and the most it pays, and confirm before signing. |
| `close`, `cancel` | How do I get out? | Close a position at the book, or cancel a resting order, after confirmation. |
| `positions` | What do I hold? | Show positions with P&L and what closing returns now, resting orders, and cash per market. |
| `deposit`, `withdraw` | How do I move cash? | Move USDC between the wallet and a market after confirmation. |
| `wallet` | What address and balances am I using? | Show address, SOL, USDC, and cash in each market. |
| `history` | What have I done? | Show trades, orders, fills, cancels, deposits, withdrawals, and settlements. |
| `faucet`, `status` | Is my dev setup ready? | Get dev USDC and SOL; check the cluster, programs, API and wallet. |
| `oracle` | How is truth formed and how can I contribute? | Show months, sources, candidates and bonds; review every write before signing. |
| `config` | What user-safe config is active? | Show non-secret effective configuration. |
| `mcp` | How do agents discover the public surface? | Print only the user-facing manifest. |
| `update` | How do I update the local CLI? | Check and run the user-safe update flow. |
| `tui`, `research` | How do I use the terminal interface and contribute oracle data? | Open the terminal interface; connect an agent, research sources, submit evidence, and inspect rewards. |

`claims` remains the reserved public noun for future active token- or
position-based settlement payouts. Do not expose that family until it is backed
by the active settlement path, and do not reuse it for retired receipt plumbing.

Do not add private command families such as `admin`, `ops`, `internal`, `chain`,
`matcher`, `database`, `worker`, `migration`, `route`, `vault-init`, `payload`, or
`bootstrap` to Petri.

## Forbidden Public Vocabulary

These concepts may exist in implementation code, tests, or opt-in diagnostics,
but they should not appear in public command names, first-line help, examples,
MCP manifests, JSON schema names, or ordinary errors.

| Avoid | Why | Prefer |
| --- | --- | --- |
| Postgres, SQL, database, row, migration | Storage implementation | history, record, source, evidence, settlement |
| backend, endpoint, API path, service | Transport architecture | Petri, market, oracle, source, status |
| admin, operator, ops, superuser | Private authority model | no public command |
| internal, hidden, private | Admits non-public surface | remove |
| matcher, route, venue engine | Execution venue implementation | contract, quote, trade, liquidity, price |
| chain bootstrap, init vault, upsert | Infrastructure mutation | remove from public CLI |
| payload-base64, receipt scan, sweep, finalize | Private lifecycle plumbing | claim, settlement, position |
| cache-only, backend snapshot | Diagnostic path | status, latest, offline |
| src, Rust package metadata, repo path | Developer environment | version, update, config |
| RPC, account layouts, program ids | Provider/protocol detail | wallet, position, balance, network |

A useful lint rule: if a normal user would ask "why am I seeing that?", the word
does not belong in public CLI text.

## Help, Errors, And Output

Help text teaches the product model. Keep default help short: one line per
command family, using product nouns. Verbose help may include flags, examples,
and risk warnings, but must remain user-safe.

Good output answers:

1. What can I trade?
2. How much can I lose?
3. What can I make or receive?
4. When does it settle?
5. Why can I trust the source or oracle?

Errors should describe product impact and next action:

- what failed;
- whether funds, holdings, bonds or claims are affected;
- what the user can try next;
- whether the issue is temporary or requires configuration.

Do not lead with stack traces, database failures, service names, route IDs, raw
payloads, or provider internals. Diagnostics are opt-in and must still avoid
secrets.

## JSON And MCP Rules

Agents are public users too. The MCP manifest teaches a model what Petri is, so
it must expose only the public command surface with the same nouns, flags, and
semantics as the CLI.

Use product JSON keys:

- `market_id`
- `contract_id`
- `position_id`
- `claim_id`
- `source_id`
- `expiry`
- `leverage`
- `max_loss`
- `max_payout`
- `status`

Avoid infrastructure JSON keys:

- `postgres_table`
- `backend_endpoint`
- `orderbook_account`
- `route_payload`
- `upsert_job`
- `internal_receipt_state`

Agent examples should follow the public journey: discover market, inspect the
book, quote a trade, leave sending to the user's terminal confirmation, inspect
positions and their ranges, read settlement, and inspect the oracle.

## Oracle Contributor Rules

Oracle contribution is user-facing because users can propose sources, support
sources, submit updates, challenge bad evidence, and settle bonds. Keep the
language about evidence and correctness, not infrastructure.

Map contributor intent to interface response:

- "I found a repeatable source" means ask for node, category, locator,
  definition, baseline evidence, and optional stake.
- "I saw a source change" means ask for raw value, timestamp, archive URL, and
  source-local context.
- "I think a source is wrong" means ask for reason, evidence URL, corrected
  value if available, and challenge stake.
- "What happened to my stake?" means show its amount, terminal eligibility,
  and program-derived disposition, then offer permissionless settlement when
  eligible. Never ask the user to choose refund or slash.
- "I am not ready to submit" means run the write with `--simulate` first, then send it.
- "I want to know how truth is formed" means show recipe, source weights, source
  states, challenge windows, and settlement reference in public terms.

## Implementation Checklist

Before handing off a public CLI/TUI/MCP change, run this audit:

- Top-level help shows only public command families.
- Public help, docs, MCP text, and examples do not use forbidden backend words
  except in this policy document or tests that enforce removal.
- Every command noun maps to a public mental model: market, contract, trade,
  liquidity, position, claim, settlement, oracle, wallet, history.
- Infrastructure mutations are absent from Petri; user state changes are
  reviewed before execution.
- Help descriptions answer a user question in one sentence.
- Errors describe product impact and next action.
- Machine-readable output uses the same public nouns as the CLI.
- The MCP manifest exposes only the user-facing surface.
- Examples follow discovery -> risk -> action -> ownership -> settlement ->
  claim.
- Nothing assumes the user is the founder or has repo/internal knowledge.

September 7 release note: the final business audit accepts explicitly documented oracle differences. It does not claim exact oracle economic equality or future payoff equality. The raw identity capture and business audit are dated evidence; every mutation still requires fresh runtime admission. Final integration checks were skipped at user request.
