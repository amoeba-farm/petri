# Capped futures

A **capped future** is a long or a short on a range of the RAMX or NANDX index, for one expiry. You pay its full cost up front. That payment is your maximum loss: there are no margin calls and no liquidation. You trade them in the web app's trade ticket and with [Petri](https://docs.amoeba.farm/getting-started/02-cli/).

> **Note:**
> Capped futures run on Solana mainnet and settle in real USDC. See [Deployment](https://docs.amoeba.farm/technical/05-deployment/).

## At a glance

| Question | Answer |
|---|---|
| What can I trade? | Long or short exposure to the RAMX or NANDX index on a range you choose, for one listed expiry. |
| How much can I lose? | What you pay when the position opens, shown as **You pay**. Nothing more is ever asked of you. |
| What can I make? | **Per point** for every index point the index settles inside your range in your favour: $0.01 per lot per point. A long collects up to its range's high end, a short down to its low end. |
| When does it settle? | At the market's expiry, 00:00 UTC on the first day of the month after the month in its name, on the oracle's final index. You can close earlier. |
| Why trust the result? | The market settles on the published oracle result for its index and month, the same for every position in it. See [How the oracle works](https://docs.amoeba.farm/oracle/01-how-it-is-structured/). |

## The payoff

A position covers a range of the index from a low end, LO, to a high end, HI. If the index settles at S, each lot pays:

| Side | Points paid per lot |
|---|---|
| Long | How far S is above LO, up to HI − LO. Nothing at LO or below. |
| Short | How far S is below HI, up to HI − LO. Nothing at HI or above. |

Each point pays $0.01 per lot, so 100 lots pay $1.00 a point. A long and a short on the same range together always pay HI − LO points: the whole range. That pair is fully funded from the moment it trades. The market holds exactly the range's value behind it until settlement.

## Entry and cost

Every position opens at an **Entry** on the market's single price line, between LO and HI. The entry splits the range's value between the two sides:

- **A long** pays for the points from LO up to its entry: lots × (entry − LO) × $0.01.
- **A short** puts in the rest of the range: lots × (HI − entry) × $0.01.

What you pay is the most you can lose. Here is a long of 100 lots on 87–135 at an average entry of 97.37. It pays $10.37 and gains $1.00 per point:

| Final index | Position pays | Net result |
|---|---|---|
| 87 or lower | $0.00 | −$10.37 |
| 97.37 | $10.37 | $0.00 |
| 110 | $23.00 | +$12.63 |
| 135 or higher | $48.00 | +$37.63 |

A short of 100 lots on the same range at the same entry puts in $48.00 − $10.37 = $37.63, and its results are the mirror image.

## Leverage and the range

**Leverage** sets the losing end of the range:

- a long's LO is entry − entry ÷ leverage, so its leverage is entry ÷ (entry − LO);
- a short's HI is entry + entry ÷ leverage, so its leverage is entry ÷ (HI − entry).

Higher leverage puts the losing end closer to the entry: the position costs less, and it is lost after a smaller move. The web ticket offers 1×, 2×, 3×, 5×, 10×, 20×, 25×, 50× and 100×, wherever that end fits on the market's scale (0 to 200 points). The default is 10×.

The **winning end** is yours to choose: **Up to** for a long, **Down to** for a short. By default it is the end of the market's suggested range: the index plus or minus two standard deviations of where the market expects it to settle, as the maker's prices imply, kept within the range the maker quotes.

Different leverages trade against each other. At an entry of 100, a 50× long and a 5× short on 98–120 are exact opposites: the long pays $0.02 per lot, the short puts in $0.20, and together they hold the range's $0.22.

## Opening a position

1. Choose **Long** or **Short**, then **Market** or **Limit**.
2. Set the leverage.
3. Set **Up to** or **Down to**, or leave it empty for the suggested range.
4. Choose an **Expiry** and enter the **Amount** in USDC. The ticket opens as many whole lots as that amount pays for at the prices on the book.
5. For **Market**, set **Max slippage** (2% by default). For **Limit**, enter a **Limit entry**.
6. Check the payoff shape, **Entry**, **Per point** and **You pay**.
7. Select **Open long** or **Open short** (**Place limit long** or **Place limit short**), then approve in your wallet.

With Petri, `petri long ramx 100` and `petri short ramx 100` do the same, with `-x` for the leverage and `--to` for the winning end. See [Trading and wallet actions](https://docs.amoeba.farm/petri-cli/04-trading-and-strategy-commands/).

### Market and limit orders

- **Market** fills now. The ticket quotes the order live; the order fills at most the lots it showed, no worse than the quote plus your slippage. Nothing of it rests on the book. If nothing can fill, the ticket says so.
- **Limit** sets your entry. Whatever can fill at that entry now fills, and the rest rests on the book until it fills or you cancel it. A resting order holds the cash it would put in at its own entry. Cancel it under **Orders** with **Cancel**.

## How orders fill

Every range and every leverage trades on one book per market. An order fills as a **package**: a set of trades in which every point of the market is covered by as many long lots as short lots. Your long on 90–110 can fill against a short on 90–100 and another on 100–110, or against a short on 90–105 and the market maker's offers on 105–110. Each trade keeps its own range; only the number of lots can fill in part.

A treasury market maker quotes both sides of every one-point step of the market, so any range has a buyer and a seller. Of all the packages the book allows, your order gets the best total price, then the most lots at that price, then the oldest orders first. See [Trading and liquidity](https://docs.amoeba.farm/markets/02-liquidity-and-trading/).

## Closing a position

Open positions appear under **Portfolio**, each with its range, lots, entry and **P&L at index**.

To close before expiry, select **Exit**. The close ticket sells the position (or buys back a short) on the same range at the best price on the book. Choose the **Lots to close** and **Max slippage**, check the **Exit price**, **Realized P&L** and **USDC received**, then select **Close position**. The USDC goes to your wallet in the same transaction. Closing part of a position keeps the rest on the same range. With Petri, use `petri close`.

An opposite position on exactly the same range cancels yours out and releases the range's value to you. A position on a different range does not: it is a new position.

## Settlement

If you hold a position to expiry, it settles on the oracle's final index for its market: a long pays (S − LO) points per lot, between zero and HI − LO, and a short pays the rest of the range. The payout goes into your free cash in that market, and **Withdraw** moves it to your wallet. See [Market lifecycle](https://docs.amoeba.farm/reference/01-market-lifecycle/#settlement-and-payout).

## Costs

There is no trading fee, no settlement fee and no funding. In the web app Amoeba's sponsor pays the network fee. See [Fees and transaction costs](https://docs.amoeba.farm/trading/02-fees/).

> **Caution:**
> Your maximum loss is fixed, but losing all of it is a normal outcome. A long loses everything if the index settles at or below its low end; a short loses everything at or above its high end. Read the [Risk disclosures](https://docs.amoeba.farm/reference/03-risk-disclosures/) first.
