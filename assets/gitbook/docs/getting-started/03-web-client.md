# Web Client

The Amoeba Farm web client is the browser interface for markets, accounts, wallet state, and trading.

The public web client should be treated as coming soon / active rollout. Treat the docs as the public path for what each screen should explain, and the live app as the source for what is currently enabled.

## What the Web Client Should Make Easy

The web client should let a user:

- open the market list;
- choose a market such as RAMX or NANDX;
- inspect the current month and expiry;
- view capped option choices;
- see maximum loss before entering;
- check oracle and settlement status;
- review wallet and ledger state.

## Web Client vs CLI

Use the web client when you want the normal visual product experience.

Use the CLI when you want repeatable commands, JSON output, automation, or local signer workflows.

Both paths should point at the same market and oracle facts.
