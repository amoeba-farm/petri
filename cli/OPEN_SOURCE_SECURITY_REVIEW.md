# Client boundary security review

> Internal engineering review of Petri's current client boundary on the
> `futures/dev-cutover` branch. It is not a security certification, deployment
> approval, or production release.

Petri signs for two programs of one deployment: the futures program and
the standalone oracle program. It trusts the program ids of one deployment,
taken from a manifest the user names (`--deployment`) or from the futures API's
`/v1/config`.

## Deployment identity

Before any signature, `src/chain_identity.rs` checks that:

- the RPC's genesis hash equals the deployment's (a deployment without a genesis
  hash is refused);
- the oracle and futures programs exist on that cluster and are executable
  accounts owned by a BPF loader;
- on a governed deployment, the controller is pinned (a custom one only on the
  trusted localnet genesis) and lists the futures program as a target;
- for oracle writes, the oracle's config account is initialized at its
  canonical PDA with the current layout, and bonds in the deployment's USDC mint.

A mismatch refuses before a signer is loaded or a transaction is sent.

## Futures transactions

Futures transactions are built locally with the pinned futures client crate
(`ameba_sdk_futures::futures`), on a market account read from chain that the
trusted futures program owns, in the deployment's USDC and reading its oracle. A
transaction prepared by the futures API with its sponsor as fee payer is checked
before signing (`src/futures/check.rs`): the fee payer must be the sponsor, only
the compute budget, the creation of the owner's own USDC account and the trusted
futures program may be called, every futures step must target the chosen market
with the owner as signer and the owner's own token account, the order must be
exactly the requested one, and a deposit may be no larger than the order can
need. Every market order carries a limit no worse than its quote plus the
slippage and a minimum fill of one lot; every close names its position. On a
governed deployment every futures instruction carries the futures program's gate
account and current epoch; a prepared transaction's gates are checked against a
fresh read, and a stale epoch or a substituted, writable or signing gate is
refused.

## Oracle transactions

Oracle instructions are built only with the pinned `ameba_oracle` crate's
builders, passing the deployment's oracle program id (the crate is built
without its localnet dev profile, so its compiled-in id is never used). Every
oracle account Petri reads before building is checked for owner, exact length,
layout and zero padding, canonical PDA, and its binding to the parent (period →
month → source → claim). SKU membership proofs are produced only from a set
that reproduces the month's committed coverage root. The oracle is its own
governance target: when the deployment gates it, every oracle instruction
carries the oracle's gate account and current epoch, and a deployment that
neither registers the oracle's gate nor is the trusted localnet's
`localnet-dev` oracle is refused.

A sealed update's value, archive URL and secret salt are stored in a private
file (mode 0600) beside the Petri config and are never printed, logged or
returned to MCP. Every oracle write shows its review and asks for confirmation
unless `--yes`; `--simulate` runs it through `simulateTransaction` with no
signer. Instruction families the oracle program has not ported (council,
rewards, sponsorship, cleanup) refuse before anything is prepared.

## Wallet

Keypair files are opened beneath a filesystem root with unsafe-traversal and
permission checks; group/world-readable files are refused unless
`--allow-insecure-keypair`. Hardware wallets are supported through
`solana-remote-wallet`. Petri prints only public keys.

## MCP

The MCP server is read-only. Its child processes run with
`PETRI_MCP_READ_ONLY=1`, which blocks signing and limits wallet resolution to
the `wallet.address` action. Undeclared arguments and flag-like values are
rejected before Petri is launched; signer locators and provider credentials are
stripped from the child environment; output is terminal-safe JSON with private
fields removed.

## Untrusted input

API and RPC responses are untrusted. Plain output neutralizes terminal control
and bidirectional-format characters and bounds line length; JSON output escapes
them. Missing or stale data is reported as unavailable, never replaced.
