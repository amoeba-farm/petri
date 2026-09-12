# API

Amoeba Farm exposes developer surfaces through the web app, backend API, and Petri CLI.

For most users, the safest developer entrypoint is the CLI with JSON output. Direct backend routes are useful for integrations, but they should be treated as versioned product surfaces, not private implementation details.

## Public Developer Paths

| Path | Use it for |
| --- | --- |
| Web client | Normal user market and trading experience. |
| Petri CLI | Repeatable commands, JSON output, market inspection, wallet checks, and agent workflows. |
| Backend API | Market snapshots, trade preparation, ledger reads, oracle and settlement data. |

## Public Route Families

The public happy path should focus on:

- market lists and market snapshots;
- charts and oracle status reads;
- trade prepare and submit flows;
- wallet ledger and history reads;
- settlement reads and claim status.

Operator routes, pool provisioning, schedulers, settlement submission, and mutation-heavy custody actions belong in operator or developer reference material, not the first public path.

## CLI JSON

Examples:

```bash
petri --json markets
petri --json markets show ramx
petri --json contracts --market ramx
petri --json oracle recipe ramx
```

Use JSON mode for scripts and agent wrappers.

## API Boundaries

Public developer docs should not expose:

- private keys;
- local keypair paths;
- secret API keys;
- operator-only routes;
- unpublished endpoint internals;
- local-only service assumptions.

## Response Shapes

Next API proxy routes should use:

```json
{ "ok": true, "data": {} }
```

or:

```json
{ "ok": false, "error": { "code": "example", "message": "Human readable message." } }
```

Backend routes may return successful payloads as:

```json
{ "ok": true, "market": "ramx" }
```

and errors as:

```json
{ "ok": false, "message": "Human readable message." }
```

Integrations should check the exact route contract before treating one envelope as universal.

For detailed command coverage, read [Command Reference](../cli/06-command-reference.md).
