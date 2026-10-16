# Products, buckets, and weights

Three separate things decide what a RAMX or NANDX position pays:

| Layer | What it fixes | When it can change |
|---|---|---|
| Product definition | Which hardware the market covers: the governed list of part numbers, grouped into rows. | Only with a new product version. |
| Oracle recipe | For one month: the sources, the bucket each source belongs to, each bucket's weight, and each source's opening value. | Locked at the Freeze stage. Fixed until settlement. |
| Market | The index, the expiry, the oracle month it settles on, and its price scale. Your position adds its own side, range and lots. | Fixed when the market is created. |

A displayed chart or product page is not one of these. Only the oracle recipe, the market's terms and your position decide a payout.

## Product definitions

**RAMX** covers 52 memory module part numbers in 13 rows:

| Group | Rows | Part numbers |
|---|---|---|
| DDR5 RDIMM | 32GB, 64GB, 96GB | 14 |
| DDR4 RDIMM | 16GB, 32GB, 64GB (3200) | 12 |
| DDR5 UDIMM | 16GB, 32GB (5600) | 6 |
| DDR4 UDIMM | 8GB, 16GB, 32GB (3200) | 12 |
| DDR4 SODIMM | 8GB, 16GB (3200) | 8 |

**NANDX** covers 48 storage part numbers in 18 rows, plus 4 raw-wafer rows that are price assessments with no part numbers:

| Group | Rows | Part numbers |
|---|---|---|
| Channel SSD | 256GB, 512GB, 1TB (PCIe 3.0); 512GB, 1TB, 2TB (PCIe 4.0) | 18 |
| OEM SSD | 256GB, 512GB, 1TB (PCIe 3.0); 512GB, 1TB, 2TB (PCIe 4.0) | 18 |
| eMMC 5.1 | 64GB, 128GB, 256GB | 6 |
| UFS 2.2 | 64GB, 128GB, 256GB | 6 |
| Flash wafer | 1Tb QLC, 1Tb TLC, 512Gb TLC, 256Gb TLC | none (assessment only) |

Part numbers count toward their row. The row's weight applies to the row as a whole, not to each part number.

## Buckets and weights

A bucket is one product row in the oracle recipe. Each bucket has a weight, and the weights in a month add up to exactly 100%.

Within a bucket, every admitted source counts equally. The bucket moves by the median of its sources' movements. The index move is the weighted sum of the bucket moves.

Keep these three ideas apart:

| Concept | Role |
|---|---|
| Bucket weight | The share of the index a product row carries. |
| Source admission and support | Whether a source is in the recipe. Support bonds back a source but never change its influence. |
| Aggregation | How admitted sources combine: median within a bucket, then the weighted sum across buckets. |

## The launch month's recipe

In the launch month, the first RAMX and NANDX months on Solana mainnet, each bucket has exactly one source: a published category price on memorymarket.com. That price is a category assessment, not an observed price for any one part number. The governance council approved these sources and weights for the launch; later months form their source lists through the open process. See [How the oracle works](https://docs.amoeba.farm/oracle/01-how-it-is-structured/#who-set-the-launch-months-sources).

RAMX, 13 buckets:

| Row | Weight |
|---|---|
| DDR5 RDIMM 32GB | 8% |
| DDR5 RDIMM 64GB | 15% |
| DDR5 RDIMM 96GB | 20% |
| DDR4 RDIMM 16GB 3200 | 5% |
| DDR4 RDIMM 32GB 3200 | 8% |
| DDR4 RDIMM 64GB 3200 | 12% |
| DDR5 UDIMM 16GB 5600 | 5% |
| DDR5 UDIMM 32GB 5600 | 4% |
| DDR4 UDIMM 8GB 3200 | 3% |
| DDR4 UDIMM 16GB 3200 | 7% |
| DDR4 UDIMM 32GB 3200 | 4% |
| DDR4 SODIMM 8GB 3200 | 4% |
| DDR4 SODIMM 16GB 3200 | 5% |

NANDX, 22 buckets:

| Group | Rows and weights |
|---|---|
| Channel SSD (30%) | 256GB PCIe 3.0 3%, 512GB PCIe 3.0 5%, 1TB PCIe 3.0 6%, 512GB PCIe 4.0 5%, 1TB PCIe 4.0 7%, 2TB PCIe 4.0 4% |
| OEM SSD (25%) | 256GB PCIe 3.0 2.5%, 512GB PCIe 3.0 4%, 1TB PCIe 3.0 5%, 512GB PCIe 4.0 4%, 1TB PCIe 4.0 6%, 2TB PCIe 4.0 3.5% |
| eMMC 5.1 (10%) | 64GB 2%, 128GB 3%, 256GB 5% |
| UFS 2.2 (10%) | 64GB 2%, 128GB 3%, 256GB 5% |
| Flash wafer (25%) | 1Tb QLC 7.5%, 1Tb TLC 10%, 512Gb TLC 5%, 256Gb TLC 2.5% |

With one source per bucket, each bucket simply moves with its source. Because every source is on one website, a problem with that site affects the whole index. The **Oracle** page in the web app shows each month's sources and weights.

## What a source must define

A source is one repeatable public price. Naming a website is not enough. The source definition fixes:

- the public page where the price is published,
- the product it prices,
- which price field is used (for example, the latest price),
- the region and the currency.

The full evidence standard is in [Evidence rules](https://docs.amoeba.farm/reference/02-evidence-rules/).

## Opening values and the index level

Each source has an accepted opening value for the month. Its movement is measured against that opening, not against other sources' prices. This lets sources that quote different price levels be combined fairly.

Each month's index starts at 100. A weighted move of +3% gives an index of 103.

## Locked recipe and full coverage

Once the month reaches Freeze, its sources and weights are locked. Later updates report new prices for the same sources. They cannot add sources, change weights, or change the method.

A month cannot go ahead until every required bucket has a source and the weights total 100%. If a source stops reporting, it carries its last accepted value forward. The index is never quietly re-weighted around a missing row.

The month's stages and the settlement calculation are described in [How the oracle works](https://docs.amoeba.farm/oracle/01-how-it-is-structured/). Formulas are in the [Math appendix](https://docs.amoeba.farm/reference/math-appendix/).
