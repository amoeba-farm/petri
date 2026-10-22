# Glossary

Plain definitions of the terms used in Amoeba Farm, using the web app’s own labels.

## RAMX / NANDX / BATX

The three indexes. RAMX follows a DRAM memory chip contract-price benchmark. NANDX follows a NAND flash chip contract-price benchmark. Their launch month, Oct 26, follows memory module and storage prices instead. BATX, a prototype, follows China domestic industrial prices of new lithium-ion battery cells in yuan. Its first month is Nov 26. All three settle in USDC.

## Index

An index's benchmark level, starting each month at 100. It is not the dollar price of a part.

## Market

One index and one expiry, such as **RAMX Oct 26**, with one book and one price line.

## Expiry

When a market stops trading: 00:00 UTC on the first day of the month after the month in its name. Settlement follows once the oracle's final index is published.

## Capped future

A long or a short on a range of the index, paid for in full when it opens. It has no margin call and no liquidation. See [Capped futures](https://docs.amoeba.farm/trading/03-capped-futures/).

## Range

The part of the index a position covers, from a low end to a high end on whole index points.

## Long

A position that collects the points the index settles above its range's low end, up to its high end.

## Short

A position that collects the points the index settles below its range's high end, down to its low end.

## Lot

The unit of size. A lot pays $0.01 per index point inside its range (each market's `lotUsdPerPoint`).

## Entry

The price a position opens at, on the market's price line, between its range's ends.

## Per point

What a position gains for each index point inside its range: its lots times $0.01.

## You pay

What goes in when a position opens: a long's cost, or a short's share of the range's value. It is the most the position can lose.

## Leverage

Entry divided by the distance from the entry to the range's losing end. Higher leverage puts that end closer to the entry.

## Up to / Down to

Where a long's (up to) or a short's (down to) range ends on the winning side.

## Suggested range

The ticket's default range: about two standard deviations either side of the index of where the maker's prices imply it will settle. It is for display and never sets a price.

## Complete set

A long and a short on the same range. Together they pay the range's width, and the market holds that cash behind them.

## Package

The set of trades an order fills in, balanced so that every point of the market is covered by as many long lots as short lots.

## Price line

The trades whose range contains the index. The market's price and chart follow it.

## Mark

The market's last price on the price line, or the index before the first trade.

## Treasury maker

Amoeba's market maker. It quotes a bid and an offer on every one-point step of each market, with the Earn vault's cash.

## Order Book

The market's bids and offers by price. It is the same whatever the ticket shows.

## Price Map

The order book's cumulative bids and offers as a chart.

## Max slippage

How much worse than the quote a market order may fill. 2% by default.

## Limit entry

The entry of a limit order. What does not fill at once rests on the book.

## Resting order

A limit order waiting on the book. It holds the cash it would put in at its own entry.

## Exit

Closing a position: selling a long, or buying back a short, on the same range at the best price on the book.

## Escrow

The cash a resting order holds. **Cancel** releases it.

## Free cash

Your cash in a market that no order holds, shown as **Free**. Payouts and released escrow land here. **Withdraw** moves it to your wallet.

## Ledger

Your ledger on the trade page and under **Account**, with the tabs **Portfolio**, **Orders**, **Activity** and **Claims**.

## Portfolio

Your positions, each with **Exit**.

## Orders

Your working orders, each with **Cancel**, and your free cash in each market, with **Withdraw**.

## Activity

Your recent transactions.

## P&L at index

A position's result if it settled at the current oracle index. What closing pays now is shown in the close ticket.

## Sign-in

Google (through Magic), Phantom, Solflare or Backpack.

## Earn

The page for depositing USDC into the Earn vault behind the treasury maker. See [Earn](https://docs.amoeba.farm/earn/02-earn/).

## Earn vault

One vault for every market: depositors provide its cash, and its operator quotes with it in each market it funds.

## Piece

A one-point range `[x, x + 1)`, the unit the treasury maker buys and sells.

## Your Deposit

What withdrawing all your Earn shares pays now: your part of the vault at its lowest value.

## Total Deposited

The whole Earn vault's value at its lowest: its cash plus what its held pieces pay at the worst settlement.

## Your Pool Share

Your shares as a part of all the Earn vault's shares.

## Your Earnings

Your Deposit less your deposits plus your withdrawals. It can be negative.

## Shares

Your part of the Earn vault. Deposits buy them at the vault's highest value; withdrawals pay at its lowest.

## Lowest and highest value

What the vault's cash and positions are worth at the worst and the best settlement outcome. They differ by the range of its open positions.

## Operator

The account that quotes for the Earn vault. It can trade for the vault and move its cash between markets, but never withdraw it.

## Amoeba Oracle

The system that forms each month's source list and computes the final index.

## SKU

One exact product identity in a market's governed manifest. From the months expiring after 2026-11-01 each bucket is one SKU: RAMX, NANDX and BATX list 4 each. The launch month's manifests list 52 and 48 part numbers.

## Bucket

A product with a fixed weight in the index: a commodity such as DDR5 16Gb chips, 1Tb TLC NAND chips or LFP storage cells, or in the launch month a row of part numbers. Every eligible source in a bucket counts equally, and the bucket moves by their median. See [Products, buckets, and weights](https://docs.amoeba.farm/reference/04-products-indexes-buckets/).

## Source

A public price admitted for a bucket. Its movement is measured from its opening value.

## Comparison series

A source of a month on chip or battery-cell buckets: one defined price-assessment row, or one recurring direct quote, with its price basis, statistic, unit, currency, period and terms fixed before it reports. RAMX and NANDX count contract prices only. It is compared only with its own opening value.

## Observation

One published price of a comparison series for one period, with its evidence. Reading the same release again is not a new observation.

## Routing status

What the oracle's rules make of a series or an observation, such as **Counts this month**, **Fits the bucket** or **Not a contract price**. Only **Counts this month** counts toward settlement. See [Evidence rules](https://docs.amoeba.farm/reference/02-evidence-rules/#routing-statuses).

## Scramble / Challenge / Freeze / Opening / Game Mode

The stages of an oracle month: propose sources, dispute them, lock the recipe, record each source's opening value, then keep source values current. See [How the oracle works](https://docs.amoeba.farm/oracle/01-how-it-is-structured/).

## Opening value

A source's accepted starting value. All of its later movement is measured against it.

## Bond / stake

USDC posted with an oracle action, such as **Proposed stake** or **Challenge bond**. It can be lost if the action fails.

## Oracle council

The five governance council seats acting on emergency oracle cases. Three matching public ballots decide.

## Oracle period

The on-chain record of one index and expiry that a market settles on: Forming, Live, Settled or Failed.

## Final index

The index value a market settles on. It comes from each source's median over the last five weekdays before expiry and the median across sources in each bucket. It is published no earlier than 24 hours after expiry. See the [Math appendix](https://docs.amoeba.farm/reference/math-appendix/#settlement-index).

## Reference value

The index value a market is listed at. It never settles a market: if the oracle period is not settled after expiry, the governance council can set the settlement value any time after expiry.

## Keeper

Amoeba's service that settles markets after the oracle settles, paying positions into their owners' free cash.

## Oracle reference index

The index as the oracle currently measures it. It is not final until settlement.

## Last known

A value kept from an earlier read and shown with its original time while a refresh fails. It never enables a trade.

## Atom

The smallest unit of an amount. USDC uses 6 decimal places.

## Grid

A market's price scale in whole steps: 0 to 200 index points on Amoeba's markets.

## k

Atoms one lot of a one-point range pays in full: 10,000 ($0.01) on Amoeba's markets.

## Claim

The program's name for a position: a side, a range and a number of lots.

## Futures program

The Solana program that holds every market's cash and positions and enforces every rule.

## Futures API

Amoeba's service that quotes orders, prepares and submits transactions, and serves market, account and oracle data.

## Sponsor

The key that pays the network fee of transactions the futures API prepares.

## Governance council

Five equal seats that approve program upgrades and gate changes. Typical actions need three approvals.

## Governance gate

An account every futures instruction must pass with its current epoch. While the gate is frozen, the program rejects instructions.

## Solana mainnet

The public Solana network where the futures and oracle programs run and the markets settle in real USDC. See [Deployment](https://docs.amoeba.farm/technical/05-deployment/).
