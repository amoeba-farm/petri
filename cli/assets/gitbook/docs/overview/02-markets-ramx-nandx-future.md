# RAMX and NANDX markets

Amoeba Farm has two indexes. **RAMX** tracks memory module (DRAM) prices. **NANDX** tracks NAND flash and storage prices. Each index has one capped futures market per monthly expiry, paid in USDC on Solana mainnet.

## What each index tracks

Each index measures how a fixed basket of product categories moves during the month. It starts the month at **100**. A 3% weighted rise in the basket puts the index at 103.

| Market | Product categories | Weights in the launch month |
|---|---|---|
| RAMX | DDR5 and DDR4 server modules (RDIMM), DDR5 and DDR4 unbuffered modules (UDIMM), and DDR4 laptop modules (SODIMM) | RDIMM 68%, UDIMM 23%, SODIMM 9% |
| NANDX | Channel SSDs, OEM SSDs, eMMC, UFS, and raw NAND flash wafers | Channel SSD 30%, OEM SSD 25%, eMMC 10%, UFS 10%, flash wafer 25% |

Each category is split into rows by generation and capacity, for example "DDR5 RDIMM 64GB" or "Channel SSD 1TB PCIe 4.0". RAMX has 13 rows and NANDX has 22. Each row's weight is fixed for the whole oracle month and shown on the **Oracle** page. [Products, buckets, and weights](https://docs.amoeba.farm/reference/04-products-indexes-buckets/) lists every row.

The index follows the published prices of these categories. It is not the price of a single part, a contract price you would negotiate with a supplier, or a semiconductor company's share price.

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
