# Markets: RAMX, NANDX, and Future Markets

Amoeba Farm markets begin with defined product families.

The names below are product families and market surfaces. A live contract should always point to an exact product version, expiry, oracle recipe, and settlement rule.

## RAMX

RAMX is the memory-market family focused on RAM and DRAM exposure.

The current RAMX oracle work centers on a module spot index structure. It uses defined rows such as memory generation, form factor, capacity, and source pins. Each accepted source is measured against its own opening value, then combined through the frozen monthly recipe.

RAMX is meant for people who want a cleaner view on RAM and DRAM market movement than they can get from broad semiconductor proxies.

## NANDX

NANDX is the memory-market family focused on NAND and storage exposure.

Like RAMX, a NANDX market should publish the product bucket, source rules, oracle calendar, and settlement method before users trade a live contract.

## Future Markets

Amoeba Farm can expand beyond RAMX and NANDX only when the market can be defined clearly enough to support:

- repeatable public sources;
- product buckets users can understand;
- a challengeable oracle recipe;
- bounded contract settlement;
- risk disclosures that match the live product.

If a future market name appears in planning notes, it should not be treated as live until its public product definition is published.

## How to Read a Market Page

Before trading, look for:

- the market name and product version;
- current month and expiry;
- fixed maximum loss;
- maximum payout or cap;
- liquidity or writer depth;
- oracle status;
- settlement window;
- risk disclosures.
