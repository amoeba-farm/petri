# Market lifecycle

Each RAMX and NANDX market is one index and one expiry. Its oracle month is formed first, then the market trades while that month is live, and after expiry it settles on the month's final index. A market's index, expiry and oracle month are fixed when it is created. Later stages verify them but never change them.

## Key times

| Event | When |
|---|---|
| Trading opens | When the market's oracle period goes Live: its sources and opening values are final. A market can be listed earlier, but it takes no trade or order until then; the ticket says "Trading opens when the oracle period is live." |
| Expiry | 00:00 UTC on the first day of the month after the month in the market's name. No trades or orders are accepted after it. |
| Final index | Each source's median over the five weekdays before expiry, then the median across the sources in each bucket, then the weighted sum of the buckets. |
| Oracle settlement | No earlier than 24 hours after expiry. This window is for evidence and disputes, and open disputes can extend it. |
| Market settlement | Once the oracle period is settled. Anyone can settle the market; Amoeba's keeper does it. If the period has not settled after expiry, the governance council can settle the market instead. |
| Payouts | Into each owner's free cash in the market, as positions settle. You withdraw them. |

## Names and months

A market is named for the last month it trades: **RAMX Oct 26** trades through October 2026, expires at 00:00 UTC on November 1, 2026, and settles on the oracle month `RAMX-202610`. The trade page always shows the exact date; rely on it rather than working it out from the name.

Markets follow the oracle's rolling three-month ladder. Amoeba lists each index's market for the first of each of the next three months, so as the nearest month expires, a new one three months out is listed; listed markets keep their terms. Check the trade page's **Expiry** list, or `GET /v1/markets`, for current markets.

## Stages

| Stage | What happens |
|---|---|
| Product definition | The product's SKUs, basket and methodology are fixed. |
| Oracle formation | The month's sources are proposed, supported, challenged and frozen into the recipe. The launch month's sources were approved by the governance council instead; see [How the oracle works](https://docs.amoeba.farm/oracle/01-how-it-is-structured/#who-set-the-launch-months-sources). |
| Opening | Each source's opening value is recorded. The index is measured from these openings. When the opening is final, the oracle period goes live. |
| Trading | The market trades on its book. Oracle updates continue while it trades. |
| Expiry | Trading ends at the expiry time. |
| Oracle settlement | The oracle month's final index is computed and published on chain. |
| Market settlement | The market reads the settled oracle period, fixes its settlement value, and pays every position into its owner's free cash. |
| Withdrawal | Owners move their cash to their wallets. |

For how sources, challenges and medians work, see [How the oracle works](https://docs.amoeba.farm/oracle/01-how-it-is-structured/) and [Evidence rules](https://docs.amoeba.farm/reference/02-evidence-rules/).

### Ordinary source calendar

When the oracle forms a month through its open process, it runs an eight-day calendar before listing:

| Day | Step |
|---|---|
| Days 1–4 | Source placement and support |
| Days 5–6 | Source challenges |
| Day 7 | Challenge resolution and recipe freeze |
| Day 8 | Opening |

If source coverage completes late, the listing date moves later but the expiry does not. A late listing therefore leaves less time to trade. A countdown reaching zero does not mean every dispute has been resolved. Check the month's status on the **Oracle** page.

## Settlement and payout

1. **Expiry.** At expiry, no new trades or orders are accepted.
2. **The oracle's value.** The oracle month finalizes no earlier than 24 hours after expiry, once its evidence and dispute window has closed, and its period becomes Settled with the final index. The observation window always ends at expiry; the waiting period does not move it.
3. **Market settlement.** The market reads the oracle period it was created against: the same index, expiry and oracle program. It fixes the settlement value to 1/10,000 of an index point, then pays each position into its owner's free cash in that market: a long the points the index settled above its low end, up to its range's width, and a short the rest. Resting orders are released, and the cash they held is freed too. Amoeba's keeper runs these steps; anyone can.
4. **Withdrawal.** The payout stays in the market until you move it: **Withdraw** in the web app, or `petri withdraw market all`.

If the oracle period is still not settled after expiry, the governance council can set the market's settlement value instead, any time after expiry. The market stays unsettled until the oracle or the council settles it. A month that traded never ends Failed: the oracle fails only periods that never went Live, and those took no trades.

See [Capped futures](https://docs.amoeba.farm/trading/03-capped-futures/) for payoffs and [Risk disclosures](https://docs.amoeba.farm/reference/03-risk-disclosures/) for timing risks.
