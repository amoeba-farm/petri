# Welcome to Amoeba Farm

Amoeba Farm is a futures market on hardware component prices. Two indexes trade: **RAMX** tracks memory module (DRAM) prices, and **NANDX** tracks NAND flash and storage prices. You trade **capped futures**: a long or a short on a range of one index, paid for up front and settled in USDC at the market's expiry. Nothing physical is delivered.

> **Caution:**
> Capped futures run on Solana mainnet and settle in real USDC. You trade them in the web app at [amoeba.farm](https://amoeba.farm); Petri's mainnet release is coming. The programs have not been audited.

## At a glance

| Question | Answer |
|---|---|
| What can I trade? | A long or a short on a range of the RAMX or NANDX index, for one expiry. Your leverage sets the range's losing end, and you choose where it ends on the winning side. See [Capped futures](https://docs.amoeba.farm/trading/03-capped-futures/). |
| How much can I lose? | What you pay up front, and no more. There are no margin calls and no liquidations. |
| What can I receive? | A fixed amount per lot for every index point inside your range: $0.01 per lot per point. A long collects the points the index settles above the range's low end, up to its high end. A short collects the points below the high end, down to the low end. |
| When does it settle? | At the market's expiry, 00:00 UTC on the first day of the month after the month in its name: RAMX Oct 26 expires on 2026-11-01. It settles on the oracle's final index for that month. You can close a position before then. |
| Why trust the price? | Each index is built from named public price sources with fixed weights recorded on chain. Each source is measured from its own opening value, and settlement uses a median over the last five weekdays. You can inspect every source on the **Oracle** page. |

## How it works

1. **The index.** Each month, RAMX and NANDX start at 100 and move with a fixed, weighted basket of product categories, such as DDR5 server modules or 1TB SSDs.
2. **The market.** Each index and expiry is one market with one price line. Every position is a claim on a range of that line. Longs and shorts on different ranges and at different leverages trade against each other in the same book.
3. **The other side.** Other traders' orders, and a treasury market maker that quotes both sides of every one-point step of the market, so any range can be bought or sold. Depositors in **Earn** fund that maker and share its result.
4. **Fully funded.** A long and a short on the same range together pay exactly the width of the range. The market holds that cash behind every position until settlement.
5. **Settlement.** After expiry, the oracle's final index decides what each position pays. Payouts go to your cash in the market, and you withdraw them to your wallet.

## Find your way

| You want to | Start here |
|---|---|
| Understand what you are trading | [Synthetic exposure](https://docs.amoeba.farm/about-synthetics/01-about-synthetics/), [RAMX and NANDX markets](https://docs.amoeba.farm/overview/02-markets-ramx-nandx-future/) |
| Make a first trade | [Getting started](https://docs.amoeba.farm/getting-started/01-where-to-begin/), [Web app](https://docs.amoeba.farm/getting-started/03-web-client/) |
| Understand a position's payoff | [Capped futures](https://docs.amoeba.farm/trading/03-capped-futures/) |
| Provide liquidity or deposit into Earn | [Earn](https://docs.amoeba.farm/earn/02-earn/), [Providing liquidity](https://docs.amoeba.farm/earn/01-providing-liquidity/) |
| Check or contribute oracle data | [How the oracle works](https://docs.amoeba.farm/oracle/01-how-it-is-structured/) |
| Use the terminal or an AI agent | [Petri](https://docs.amoeba.farm/getting-started/02-cli/), [Connect an AI agent](https://docs.amoeba.farm/getting-started/04-claude-code-codex/) |
| Build an integration | [API reference](https://docs.amoeba.farm/developer/01-api/), [Quickstart](https://docs.amoeba.farm/developer/03-sdk-quickstart/) |

## Fees

Amoeba charges no trading, settlement or Earn fees, and there is no funding. In the web app, Amoeba's sponsor pays the Solana network fee of every transaction. Petri can pay it from your wallet in SOL or use the sponsor. See [Fees and transaction costs](https://docs.amoeba.farm/trading/02-fees/).

> **Caution:**
> You can lose everything you pay for a position, and Earn deposits can lose value. Check the market, expiry, range and amount before you approve. Read [Risk disclosures](https://docs.amoeba.farm/reference/03-risk-disclosures/). Program addresses are in [Deployment](https://docs.amoeba.farm/technical/05-deployment/).
