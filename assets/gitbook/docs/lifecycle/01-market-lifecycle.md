# Lifecycle Overview

Amoeba uses a monthly lifecycle because hardware data is not a clean continuous price tape.

The lifecycle separates product setup, source selection, live updates, and settlement so that a live contract is not quietly rewritten after users enter.

## Lifecycle stages

| Stage | Purpose | Main invariant |
| --- | --- | --- |
| Contract creation | Publish what the market is. | Product version is known before trade. |
| Source selection | Build the monthly source recipe. | Source weights freeze before live updates. |
| Opening print | Set denominators for accepted sources. | No opening print means no active source delta. |
| Oracle Game Mode | Update frozen source states. | Game Mode changes states, not weights. |
| Settlement and claims | Convert final oracle output into contract outcomes. | Settlement uses the published payoff rule. |

## Why stages matter

Without stages, disputes become hard to reason about. A user could argue about product definition, source validity, update value, and settlement payout all at once.

The staged lifecycle forces each question into the right window:

- product questions before contract launch;
- source membership questions before freeze;
- opening value questions before activation;
- update questions during Game Mode;
- payout questions at settlement.

That structure is what makes the oracle challengeable without letting challenges endlessly rewrite the market.
