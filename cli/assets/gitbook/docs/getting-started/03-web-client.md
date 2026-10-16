# Web app

The web app is the main way to trade RAMX and NANDX capped futures, deposit into Earn, and check the oracle. It runs at [amoeba.farm](https://amoeba.farm), on Solana mainnet with real USDC. This page walks through what you see and what each control does.

## Sign in

Select **Sign In** at the top right (**Log In** on phones). You can choose:

| Option | What happens |
|---|---|
| **Continue with Google** | Signs you in through Magic. A Solana wallet is created for you automatically. |
| **Continue with Phantom** | Connects the Phantom wallet. |
| **Continue with Solflare** | Connects the Solflare wallet. |
| **Continue with Backpack** | Connects the Backpack wallet. |

A browser wallet asks you to sign a sign-in message. This is not a transaction and moves no funds. The first time you sign in with a wallet, **Create your account** asks for an email address. Display name and phone are optional.

Once you are signed in, the header shows your wallet's USDC balance and, when you have cash inside markets, how much (for example **+7,225.00 in markets**; phones show the wallet amount only). The gear menu has **Account**, **Download CLI**, and **Log out**.

Amoeba's sponsor pays the Solana network fee of every transaction the web app prepares, so your wallet needs no SOL. The app refuses to sign a transaction that the sponsor does not pay for. See [Fees and transaction costs](https://docs.amoeba.farm/trading/02-fees/).

## Find a market

The main navigation has **Components**, **Products**, **Oracle**, **Earn**, **Leaderboard** and **Roadmap**. On phones, the menu also has **Trading** and **Account**, and the bar at the bottom has **Chart**, **Trade** and **Positions**.

- **Components** (the desktop home page) lists the component markets. Their prices, **24h Change**, charts and sparklines come from the futures markets you trade. Each market card has **Order Book** and **Trades** tabs for the month on show, and **Long** and **Short** open the trade page on that month with that side selected.
- **Products** shows real hardware, a **GPU Cluster** and a **Robotics Stack**, and the parts inside them. The parts table lists each **Part**, the **Market** that prices it, its **Change**, and a **Trade** link. Parts without a market yet show **Soon**.

Index values, charts, and liquidity on these pages are for display. They are not quotes. The price you pay is set in the trade ticket.

## The trade page

Each index has its own trade page: `/trade/ramx` and `/trade/nandx`. `?expiry=YYYY-MM` opens a given month and `?side=long` or `?side=short` picks the ticket's side, for example `/trade/ramx?expiry=2026-12&side=short`.

The header shows the price and its 24-hour change, then **Mark**, **Oracle** (the oracle reference index), **24h Volume**, **Open Interest** (the cash backing the market's open positions, in dollars), and the **Expiry** you are looking at. For a month that has settled, **Settlement** shows the settlement value in the mark's place.

The main area has three views:

- **Price History**: the market's price chart. It follows the trades whose range contains the index; the line marked **Oracle index** is the oracle's reference index.
- **Forecast**: a projection chart. It is for display only and never sets a price or a settlement.
- **Price Map**: the order book's cumulative bids and offers by price.

Beside them, **Order Book** and **Trades** show the book and the latest trades. On phones the views become the **Chart**, **Forecast**, **Book** and **Trades** tabs; **Book** holds the **Order Book** and the **Price Map**.

The **?** button (**Show trade guide**) walks through the page in eight steps: the chart, the book, the trades, then the ticket's **Long** or **Short**, leverage, **Up to** or **Down to**, amount, and the payoff with the order button.

### The order book

The book is one book for the market, the same whatever you set in the ticket. Offers sit at or above the current price and bids at or below it, each level with its **Price**, **Lots** and running **Total**. The row between them is the current price.

Select a level, or a point on the Price Map, to make that price your limit entry. The ticket switches to **Limit** and keeps your side, leverage and range. See [Trading and liquidity](https://docs.amoeba.farm/markets/02-liquidity-and-trading/) for how different ranges trade on one book.

### The ticket

1. Choose **Long** or **Short**, then **Market** or **Limit**.
2. Set the leverage with the slider. It sets the losing end of your range: the entry minus entry ÷ leverage for a long, plus it for a short. The slider offers 1×, 2×, 3×, 5×, 10×, 20×, 25×, 50× and 100×, where that end fits on the market's scale. The default is 10×.
3. Set where the range ends on the winning side: **Up to** for a long, **Down to** for a short. Left empty, it follows the market's suggested range.
4. Choose the **Expiry** and enter the **Amount** of USDC to put in (100 by default). The ticket buys as many whole lots as that amount pays for.
5. For a market order, set **Max slippage** (2% by default). For a limit order, enter the **Limit entry**.
6. Check the payoff shape and the three lines under it: **Entry**, **Per point** (what the position gains for each index point inside the range) and **You pay** (what goes in now, and the most you can lose).
7. Select **Open long** or **Open short** (**Place limit long** or **Place limit short** for a limit order), then approve in your wallet.

A market order is quoted live while you edit. It fills now, at most the lots the ticket showed and no worse than the quote plus your slippage, and nothing of it rests. A limit order fills what it can at your entry and rests on the book for the rest, until it fills or you cancel it. The ticket tells you where it fills: "Fills when the price drops to …" for a long.

An order spends your free cash in that market first and takes only the rest from your wallet. If the two together fall short, the ticket says how much they hold.

The **Expiry** list offers only months that trade. Opening a month that no longer trades, for example from a link, shows it with the ticket closed: "Trading has ended. Settled at …" once it has settled, or "Trading has ended. This month settles on the oracle's final value." before then.

There is no margin call and no liquidation. See [Capped futures](https://docs.amoeba.farm/trading/03-capped-futures/).

## Positions and orders

Your ledger appears below the ticket on the trade page and under **Account** → **Ledger**. It has four tabs: **Portfolio**, **Orders**, **Activity** and **Claims**.

| Tab | What it shows |
|---|---|
| **Portfolio** | Your positions: **Market / side**, **Lev**, **Lots**, **Entry**, **Range**, **Cost**, **Exit P&L** (what closing now pays, less what you paid; the settlement once final) and **Max payout** (what the position pays at best). Each has an **Exit** button. |
| **Orders** | Your **Working orders** in each market, with their **Price**, **Lots**, **Lev**, the cash each holds (**Escrow**), and your **Free** cash in that market. Each order has **Cancel**; each market has **Withdraw**. |
| **Activity** | One row per action, such as a market order, a limit order placed, an order filled, a position closed, a settlement payout or a withdrawal, with **Time**, **Market**, **Type**, the side, lots and range, the **USDC change**, and **Status**. |
| **Claims** | **Payouts**: each market that has settled, expired or holds your free cash, with its **Settlement** value, your **Payout**, what is **Paid out** so far, your **Free** cash there, and **Withdraw**. |

**Exit** opens **Close position**. It sells the position (or buys back a short) at the best price on the book. Choose the **Lots to close** and the **Max slippage**, then check the **Exit price**, **Realized P&L** and **USDC received**. **Close position** sends the transaction, and the USDC goes straight to your wallet. Closing part of a position keeps the rest on the same range.

**Cancel** cancels a resting order and moves the cash it held to your wallet in the same transaction. **Withdraw** moves a market's free cash to your wallet. After settlement, a position's payout is free cash in its market: collect it with **Withdraw** under **Claims** or **Orders**. See [Market lifecycle](https://docs.amoeba.farm/reference/01-market-lifecycle/#settlement-and-payout).

## Earn

**Earn** lets you deposit USDC into the Earn vault, one for every futures market. The vault funds the treasury maker that quotes every range in each market, and it earns the maker's spread. Your shares are your part of the whole vault. There are no trading fees and no funding.

- The summary shows **Your Deposit** (what withdrawing all your shares pays now), **Your Pool Share** and **Your Earnings**.
- The **Pool** tab has one USDC row with **Asset**, **Earns From**, **Your Deposit**, **Your Earnings**, **Total Deposited** (the vault's value at its lowest) and an **Action** column with **Deposit** and **Withdraw**.
- **Deposit** and **Withdraw** open a dialog: enter the amount, or select **Withdraw all**. The deposit dialog notes that deposits buy at the vault's highest value and withdrawals pay its lowest, and that the gap closes as positions settle.
- **How It Earns** explains the vault's income, and **Earn Activity** lists your deposits and withdrawals.

If the pieces the vault holds settle against it, it loses money and deposits go down. See [Earn](https://docs.amoeba.farm/earn/02-earn/).

## Oracle

**Oracle** shows the price sources behind each market. Pick a **Market** and **Month**. The summary shows the **Current Stage**, **Source Records**, **Source Links**, and when the data was last **Updated**.

- **Sources** lists one row per source website with its **Link**, **Records**, and **Weight**. Select **View** to see each record's **Status**, **Weight**, and **Source ID**.
- **Contribute** lists, for the current stage (Scramble, Challenge, Opening, or Game Mode), the Petri commands that make each contribution, with the month filled in. Contributions are signed with your own wallet in Petri; the web app does not submit them. It links to **Get Petri** and to **Source discovery**, where you can submit a candidate source without a bond or a signature.

See [How the oracle works](https://docs.amoeba.farm/oracle/01-how-it-is-structured/).

## Leaderboard

**Leaderboard** ranks every trader by profit and loss over **7D**, **30D**, **90D**, or **All**, with their **Return**, **Volume**, **Trades** and **Last Trade**.

- **P&L** covers the positions a trader opened in that period: what they received from sales and settlement, less what they paid to open, plus open positions at what they would sell for now on the book. A position the book cannot price now is valued at what it would pay at the current index, and the trader's P&L is marked *.
- **Return** is P&L divided by the USDC paid.
- Every trader appears under a generated alias and avatar. The page never shows a wallet address, username, or profile picture.
- Signed in, you also see **Your Rank**, pinned below the table while your row is further down than the rows shown.

Amoeba's own accounts, such as the treasury maker and the Earn vault, are not ranked.

## Account

**Account** has four sections:

- **Profile**: your photo, username, and display name.
- **Account**: your provider, wallet address, USDC balance, and contact details. Cash you hold inside markets shows beside the balance, with **Withdraw all**, which withdraws each market's free cash to your wallet. **Add funds** opens Stripe's USDC on-ramp where it is available. To receive USDC from an exchange or another wallet, copy your **Wallet** address and send USDC on Solana to it. If you signed in with Google, **Cash out** sends wallet USDC to another Solana wallet, and Amoeba pays its network fee; it tells you when more is still in markets.
- **Ledger**: the ledger described above.
- **Help & Legal**: terms, privacy policy, support contact, and the CLI download.

## Web app or Petri

The web app and the Petri command line trade the same markets and build the same orders. Petri adds repeatable commands and JSON output for scripts and agents. Its release for Solana mainnet is coming; see [Petri](https://docs.amoeba.farm/getting-started/02-cli/#availability).

> **Caution:**
> Check the market, expiry, range, lots and **You pay** in the ticket, and the transaction in your wallet, before you approve. Read [Risk disclosures](https://docs.amoeba.farm/reference/03-risk-disclosures/).
