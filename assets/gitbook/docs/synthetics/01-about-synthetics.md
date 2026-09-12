# About Synthetics

A synthetic market lets someone trade exposure to a published outcome without taking delivery of the underlying thing.

In Amoeba Farm, the first underlying things are hardware-input benchmarks. A user is not buying a stick of RAM or a pallet of NAND. They are trading a contract whose payout depends on how a defined market index settles.

## Why Synthetics Matter

Many important markets are hard for individuals to reach directly.

Hardware input markets are a good example. Memory prices move through distributor channels, retailer pages, manufacturer listings, benchmarks, and procurement relationships. A person can watch the cycle, but clean exposure is usually indirect.

Common indirect paths include:

- chip equities;
- broad semiconductor indexes;
- cloud or AI infrastructure equities;
- private procurement relationships;
- informal quote watching.

Those paths can be useful, but they are noisy. Amoeba Farm is built for people who want a cleaner view on the underlying hardware cycle itself.

## What Amoeba Farm Adds

Amoeba Farm turns a messy information problem into a public market process:

1. define the product bucket;
2. publish the monthly source recipe;
3. accept evidence and challenges;
4. compute the oracle result;
5. settle bounded contracts from that result.

The synthetic market is only useful if the index is inspectable. That is why the oracle and evidence system are part of the product, not a hidden backend detail.

## What Synthetics Do Not Do

Synthetic markets do not make a trade safe. They also do not create physical delivery unless a product explicitly says so.

For Amoeba Farm v1, treat synthetic contracts as fixed-risk monthly exposure to a published settlement result. Physical inventory and future-output claims are separate roadmap categories unless a live product page says otherwise.
