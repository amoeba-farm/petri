# Synthetic exposure

Amoeba futures are synthetic. They pay USDC based on a hardware price index. You never receive memory modules, SSDs, warehouse receipts, or any claim on physical stock.

## What the exposure is

Each market references the RAMX or NANDX index for one month. RAMX follows memory module prices; NANDX follows NAND flash and storage prices. Each index starts the month at 100 and moves with a fixed, weighted basket of product categories. See [RAMX and NANDX markets](https://docs.amoeba.farm/overview/02-markets-ramx-nandx-future/).

This is a view on component prices, not on companies. A memory maker's shares also depend on its costs, debt, product mix, and other business results. The index does not.

The index is built from published category prices. It can differ from what a buyer pays under a supply contract, and it is not a universal transaction price. Read what each market's rows cover rather than relying on the ticker. The full list is in [Products, buckets, and weights](https://docs.amoeba.farm/reference/04-products-indexes-buckets/).

## Why ranges

Hardware prices are published slowly and can jump. Open-ended futures would need margin calls and liquidations to stay safe. Amoeba avoids that: every position is a claim on a range of the index, and both sides of it are paid for in full when it opens.

- **A long** pays for the part of the range below its entry. At settlement it receives the points the index settles above the range's low end, up to the range's width. The most it can lose is what it paid.
- **A short** puts in the rest of the range. At settlement it receives the points below the range's high end, down to the low end. The most it can lose is what it put in.
- **Earn depositors** fund the market maker and share its results. If the positions it holds settle against it, deposits go down.

A long and a short on the same range together pay exactly the range's width, and the market holds that cash from the start. There are no margin calls and no liquidations. See [Capped futures](https://docs.amoeba.farm/trading/03-capped-futures/) and [Providing liquidity](https://docs.amoeba.farm/earn/01-providing-liquidity/).

## What this does not promise

- A position can settle at zero.
- Earn depositors can lose capital.
- A hedge on the index will not match your own purchase costs exactly.
- Being able to inspect the sources and method does not make them accurate, and it does not guarantee a profit or a way to exit before expiry.

Amoeba does not offer physical delivery. Read [Risk disclosures](https://docs.amoeba.farm/reference/03-risk-disclosures/) before you trade.
