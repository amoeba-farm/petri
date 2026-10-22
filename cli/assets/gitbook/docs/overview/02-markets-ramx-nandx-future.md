# RAMX, NANDX, and BATX markets

Amoeba Farm has three indexes. **RAMX** tracks DRAM memory chip prices. **NANDX** tracks NAND flash chip prices. **BATX** tracks lithium-ion battery cell prices in China. Each index has one capped futures market per monthly expiry, paid in USDC on Solana mainnet. BATX is a prototype, and its first month is BATX Nov 26.

## What each index tracks

Each index measures how a fixed basket of buckets moves during the month. It starts the month at **100**. A 3% weighted rise in the basket puts the index at 103.

From the months expiring after 2026-11-01:

| Market | What it prices | Buckets and weights |
|---|---|---|
| RAMX | Contract prices of new, standard-production DDR5 and DDR4 memory chips, one die per package | DDR5 16Gb 40%, DDR5 24Gb 20%, DDR4 16Gb 25%, DDR4 8Gb 15% |
| NANDX | Contract prices of new, standard-production 3D NAND flash chips, one die per package | 1Tb TLC 40%, 1Tb QLC 30%, 512Gb TLC 20%, 256Gb TLC 10% |
| BATX | China domestic industrial prices of new lithium-ion battery cells, in Chinese yuan | LFP storage 40%, LFP EV 30%, Ternary prismatic EV 20%, Ternary pouch EV 10% |

Each bucket's weight is fixed for the whole oracle month and shown on the **Oracle** page. The launch month, RAMX Oct 26 and NANDX Oct 26, uses the first definitions: 13 rows of memory modules (RDIMM 68%, UDIMM 23%, SODIMM 9%) and 22 rows of storage (channel SSD 30%, OEM SSD 25%, flash wafer 25%, eMMC 10%, UFS 10%). [Products, buckets, and weights](https://docs.amoeba.farm/reference/04-products-indexes-buckets/) lists every bucket and row.

The index follows the published prices of these buckets. It is not the price of a single part, the contract price you would negotiate with your own supplier, or a chip or battery maker's share price.

## BATX

BATX measures China domestic industrial prices of new manufactured lithium-ion battery cells. It is priced in Chinese yuan (CNY), per Wh where possible. Like RAMX and NANDX, it starts each month at 100 and settles in USDC. Settling in USDC adds no currency return: BATX measures price changes in yuan.

Its buckets are LFP cells for energy storage, LFP cells for electric vehicles, and nickel-based ternary prismatic and pouch cells for electric vehicles. Their weights are design allocations, not market shares. Raw materials, assembled packs, complete storage systems, used cells and retail prices are outside BATX. [Products, buckets, and weights](https://docs.amoeba.farm/reference/04-products-indexes-buckets/) has the full definition.

BATX's first month is BATX Nov 26, expiring 2026-12-01 00:00 UTC. It has no launch month: its sources form through the [ordinary monthly process](https://docs.amoeba.farm/oracle/01-how-it-is-structured/#the-ordinary-monthly-process) from its first month.

BATX is a prototype. No publisher's prices are licensed for its settlement yet.

## What you can trade

Each market trades **capped futures**: a long or a short on a range of the index.

| Position | Gains when | Pays at settlement, per lot |
|---|---|---|
| Long on LO–HI | The index settles higher | $0.01 for every point the index settles above LO, up to HI − LO points |
| Short on LO–HI | The index settles lower | $0.01 for every point the index settles below HI, up to HI − LO points |

For example, a long on 90–110 pays $0.15 per lot if RAMX settles at 105, and the full $0.20 at 110 or above. You choose the range through your leverage and where the range ends on the winning side, and you pay for the position in full when it opens: that is the most you can lose. See [Capped futures](https://docs.amoeba.farm/trading/03-capped-futures/).

Each market's index runs on a scale from 0 to 200 points, one step per point. Ranges start and end on whole points, and entries can fall between them.

## Current listings

Markets are listed on a rolling three-month ladder: each index has a market for each of the next three monthly expiries, and a new month is listed as the nearest one expires. A market takes trades once its oracle month is live. The trade page's **Expiry** list, `petri markets` and the API's `GET /v1/markets` show the markets listed now, with each one's expiry and oracle month.

## Identify a market

A market is named for its index and the last month it trades, for example **RAMX Oct 26**. It expires at 00:00 UTC on the first day of the next month, here 2026-11-01. The trade page always shows the exact date.

Each market settles on one oracle month, such as `RAMX-202610`: the index and month in the market's name. Every position in that market, whatever its range, settles on the same oracle result.

## When it settles

Markets expire at 00:00 UTC on their expiry date. The oracle takes a median of each source's prices over the last five weekdays before expiry, then applies the fixed weights. The final result is published no earlier than 24 hours after expiry, which leaves time for late evidence and disputes. The market then settles every position on that value. See [How the oracle works](https://docs.amoeba.farm/oracle/01-how-it-is-structured/) and [Market lifecycle](https://docs.amoeba.farm/reference/01-market-lifecycle/).

## Other hardware

No other market is live. The **Products** page in the web app shows parts such as a linear actuator with **Soon** in place of a market. A new market needs its own product definition, public price sources, and settlement rules before it can list.
