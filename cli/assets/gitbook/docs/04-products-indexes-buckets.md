# Products, buckets, and weights

Three separate things decide what a RAMX, NANDX, or BATX position pays:

| Layer | What it fixes | When it can change |
|---|---|---|
| Product definition | Which hardware the market covers: its buckets, what each one may price, and their weights. | Only with a new product version. |
| Oracle recipe | For one month: the sources, the bucket each source belongs to, each bucket's weight, and each source's opening value. | Locked at the Freeze stage. Fixed until settlement. |
| Market | The index, the expiry, the oracle month it settles on, and its price scale. Your position adds its own side, range and lots. | Fixed when the market is created. |

A displayed chart or product page is not one of these. Only the oracle recipe, the market's terms and your position decide a payout.

## Product definitions

From the months expiring after 2026-11-01, RAMX and NANDX price commodity memory chips, not modules or drives. BATX prices battery cells from its first month. Each product has a fixed set of buckets with fixed weights.

**RAMX** prices new, unused, standard-production DDR4 and DDR5 SDRAM chips with one die per package, on a contract-price basis:

| Bucket | Weight |
|---|---|
| DDR5 16Gb | 40% |
| DDR5 24Gb | 20% |
| DDR4 16Gb | 25% |
| DDR4 8Gb | 15% |

The density is the single die's: a package holding two 8Gb dies is not a 16Gb chip. Different organizations (x4, x8, x16), standard speed bins, manufacturers and quantity tiers can all count, each as its own comparison series. RAMX does not include memory modules, bare wafers or dies, stacked or multi-die packages, LPDDR, GDDR or HBM, engineering samples, or reclaimed, refurbished, eTT/uTT, downgraded or untested parts.

DDR5 32Gb chips are a research candidate at 0%, not a RAMX bucket: a DDR5 32Gb series reads **Research only**. It is evidence for a later decision, not a settlement input.

**NANDX** prices new, unused, standard-production 3D NAND flash chips with one die per package, by cell type and die density, on a contract-price basis, as RAMX prices DRAM chips:

| Bucket | Weight |
|---|---|
| 1Tb TLC | 40% |
| 1Tb QLC | 30% |
| 512Gb TLC | 20% |
| 256Gb TLC | 10% |

The density is the single die's: a package holding four 1Tb dies is not a 1Tb chip. Different interfaces and I/O speeds, packages, manufacturers, layer generations and quantity tiers can all count, each as its own comparison series; an assessment can cover several manufacturers and layer generations. NANDX does not include NAND wafers or wafer assessments, bare dies, stacked or multi-die packages, SSD, eMMC, UFS or memory card prices, SLC, MLC or planar NAND, engineering samples, or reclaimed, refurbished, downgraded, white-label or untested parts. Until 2026-10-22 NANDX's buckets were fabricated-wafer assessments; no month was formed on them on mainnet.

These definitions are new. The comparison series in each bucket come from open submissions and are still being filled in, and published contract-price coverage is thin for some buckets, especially DDR5 24Gb and the NANDX chips.

**BATX** prices new manufactured lithium-ion battery cells. It uses China domestic industrial prices in Chinese yuan (CNY), per Wh where possible. Its first month is BATX Nov 26, expiring 2026-12-01 00:00 UTC. It has no launch month: its sources form through the [ordinary monthly process](https://docs.amoeba.farm/oracle/01-how-it-is-structured/#the-ordinary-monthly-process) from its first month.

| Bucket | What it prices | Weight |
|---|---|---|
| `LFP_STORAGE` | LFP (lithium iron phosphate) prismatic cells for stationary energy storage | 40% |
| `LFP_EV` | LFP prismatic cells for electric vehicles | 30% |
| `TERNARY_PRISM_EV` | Nickel-based ternary (NCM/NCA) prismatic cells for electric vehicles | 20% |
| `TERNARY_POUCH_EV` | Nickel-based ternary pouch cells for electric vehicles | 10% |

These weights are design allocations, not market shares. Detailed capacities and models, such as a 280 Ah or a 314 Ah storage cell, are comparison series inside a bucket. They are not separately weighted markets. Its sources and buckets combine as RAMX's and NANDX's do. Each source is measured from its own opening value. Each bucket moves by the median of its sources, and the index by the weighted sum of its buckets.

BATX does not include lithium carbonate or cathode powders, assembled packs, complete storage systems, used or second-life cells, retail prices or bundles, or, in version 1, cylindrical cells. US retail prices and European distributor quotes are not blended in. A series in a currency other than yuan, or from outside China, is research only.

BATX measures price changes in yuan. It settles in USDC, and that adds no currency return.

BATX is a prototype. No publisher's prices are licensed for its settlement yet. TrendForce's terms forbid using its prices in prediction markets, and other publishers require a license. A bucket settles on its own median only with at least three eligible sources. With fewer, the council decides (see [Settlement](https://docs.amoeba.farm/oracle/01-how-it-is-structured/#settlement)).

### The launch month's product definition

The launch month, RAMX Oct 26 and NANDX Oct 26 (expiring 2026-11-01), uses the first product definitions, of memory modules and storage products.

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

A bucket is one product in the oracle recipe: a commodity such as DDR5 16Gb chips or LFP storage cells, or in the launch month one product row. Each bucket has a weight, and the weights in a month add up to exactly 100%.

Within a bucket, every admitted source counts equally. The bucket moves by the median of its sources' movements. The index move is the weighted sum of the bucket moves.

Keep these three ideas apart:

| Concept | Role |
|---|---|
| Bucket weight | The share of the index a bucket carries. |
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

A source is one repeatable public price. Naming a website is not enough.

From the months expiring after 2026-11-01, each source is a **comparison series**: one defined contract-price assessment row, or one recurring direct contract quote. Its definition fixes, before it reports:

- the public page and the exact row or part it prices, and its bucket,
- whether it is a published assessment or a direct price, and its price basis,
- which statistic is used (for example, the average), in which native unit and currency,
- the reference period and how often it is published.

A BATX series can also price on a spot or firm-offer basis. Each series fixes one basis before it reports.

Each published price for one period, with its evidence, is an **observation** of that series. In the launch month, a source definition fixes the page, the product, the price field (for example, the latest price), the region and the currency.

The full evidence standard is in [Evidence rules](https://docs.amoeba.farm/reference/02-evidence-rules/).

## Opening values and the index level

Each source has an accepted opening value for the month. Its movement is measured against that opening, not against other sources' prices. This lets sources that quote different price levels be combined fairly.

Each month's index starts at 100. A weighted move of +3% gives an index of 103.

## Locked recipe and full coverage

Once the month reaches Freeze, its sources and weights are locked: from then on they are the month's committed series. Later updates report new prices for the same sources. They cannot add sources, change weights, or change the method. A new source can only count in a later month.

A month cannot go ahead until every required bucket has a source and the weights total 100%. If a source stops reporting, it carries its last accepted value forward. The index is never quietly re-weighted around a missing row.

The month's stages and the settlement calculation are described in [How the oracle works](https://docs.amoeba.farm/oracle/01-how-it-is-structured/). Formulas are in the [Math appendix](https://docs.amoeba.farm/reference/math-appendix/).
