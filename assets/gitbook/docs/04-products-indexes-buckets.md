# Products, Indexes, and Buckets

Amoeba markets start with product definitions.

The oracle can only produce a useful movement if the thing being measured is defined carefully. Product definitions should be narrow enough to compare movement, but not so narrow that the market has no usable source set.

## Product bucket

A bucket is a standardized product category. For memory markets, a bucket may include:

- product standard;
- generation;
- capacity;
- grade;
- condition;
- region or market environment;
- quote convention;
- delivery or listing convention.

Examples may include DRAM module categories, NAND flash gauges, channel-market SSD gauges, or embedded NAND gauges.

## Basket weight vs. source weight

These two weights do different jobs.

| Weight type | What it controls | When it changes |
| --- | --- | --- |
| Basket weight | The economic meaning of the product index. | Only through a published product-version process. |
| Source weight | The influence of one accepted source inside a bucket. | Formed before the live period and frozen for that month. |

Changing basket weights changes the product. Changing source weights changes the oracle recipe. Neither should change silently inside a live contract.

## Product version

Every live market should point to a product version.

A product version should publish:

- bucket definitions;
- basket weights;
- source category rules;
- oracle calendar;
- settlement window;
- payoff family;
- risk disclosures.

The public rule is simple: a contract knows its recipe before users trade it.

## Early product families

Public names such as RAMX, NANDX, or composite hardware baskets should be used only when the final definitions are published. Until then, docs should describe them as product families or examples, not final live market specs.
