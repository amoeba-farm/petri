# Bounties and rewards

Oracle bounties pay contributors for accepted oracle work out of a funded USDC budget. They are separate from futures payouts, Earn results, and the return of your own bond.

> **Note:**
> The launch month has **zero** reward budgets. There is no bounty to earn or collect on it. Each later month's budgets are set when it forms.

## Funding

Each oracle month has its own reward schedule backed by a USDC vault. Within it, every product bucket can have three separate budgets:

- **Source budget**, for proposing and supporting sources that are accepted.
- **Opening budget**, for accepted opening values.
- **Update budget**, for accepted price updates in Game Mode.

Any of these budgets can be zero. A zero budget does not remove the bonds, challenge periods, or other rules. The vault must actually hold the USDC behind every budget it reserves, and a budget that was frozen at zero cannot be topped up later.

Bounties do not come from trading fees. Amoeba charges no trading or protocol fees, so there is no fee revenue to share. The former AMBA reward funnel has been removed.

## How a budget is divided

| Budget | How it is shared |
|---|---|
| Source | Split equally between the accepted sources in the bucket. Each source's proposer receives a set percentage of that source's share, and its supporters divide the rest equally. |
| Opening | Split equally between the accepted opening values. |
| Update | Split in proportion to reward units. An accepted update earns 1 unit, or 3 units if its observation time falls in the five-weekday settlement window before expiry. |

Reward units are relative weights, not USDC amounts. Three units do not mean three USDC.

An unchanged report is not an update and earns nothing. When a source carries its value forward into settlement, that creates no observation and no reward.

## Researchers and sponsors

On the **Source discovery** page, a researcher can submit a candidate source without USDC or a wallet signature. A sponsor can then fund the listing bond that turns the candidate into an on-chain proposal. If that proposal earns a source bounty, the proposer's share is split like this:

| Funded by | Sponsor | Researcher | Researcher's reserve |
|---|---|---|---|
| Sponsor | 50% | 40% | 10% |
| Researcher's own reserve | 0% | 80% | 20% |

For example, a 10 USDC bounty on a sponsored source pays 5 to the sponsor, 4 to the researcher, and 1 into the researcher's reserve for future bonds. The sponsor's whole bond is at risk if the source is rejected.

Sponsorship preparation is currently unavailable, so Source discovery hides its **Sponsor** tab and the bounty split: candidates can be submitted but not funded through the web app.

## Collecting a bounty

Collecting a bounty is a separate on-chain step that you sign yourself. It is not paid automatically.

1. Your work is accepted and any challenges are closed.
2. The oracle month settles.
3. The operator finalizes each contributor's entitlement for the month.
4. You sign a claim for your entitlement. The USDC is credited to your oracle collateral balance.

A claim for a zero entitlement is rejected without moving any funds.

Your own bonds come back separately. Once a bond's case is closed, anyone can trigger its settlement, and the amount is credited to your oracle collateral balance. Challenge losses follow the rules in [Challenges and disputes](https://docs.amoeba.farm/contributors/03-challenges/#where-the-bonds-go).

The web app does not currently prepare bounty claims or oracle collateral withdrawals. Petri has no bounty-claim command. Petri returns available oracle collateral to your wallet with `petri oracle collateral withdraw`.

See also [Oracle roles](https://docs.amoeba.farm/oracle/02-token-holders-and-contributors/) and [Evidence rules](https://docs.amoeba.farm/reference/02-evidence-rules/).
