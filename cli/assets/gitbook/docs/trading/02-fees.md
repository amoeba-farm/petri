# Fees and transaction costs

Amoeba takes **no trading, settlement, Earn or protocol fee**, and capped futures have **no funding**. Every USDC that goes into a market comes out of it as positions, sales, settlement payouts or withdrawals.

You still need Solana network fees paid. Who pays them depends on how you trade:

- **In the web app**, Amoeba's sponsor pays the network fee of every transaction the app prepares. Your wallet needs no SOL, and the app refuses to sign a transaction that the sponsor does not pay for.
- **With Petri**, `--route` decides: `local` pays from your wallet in SOL, `sponsored` uses Amoeba's sponsor, and `auto`, the default, uses your wallet when it has SOL for fees. See [Who pays network fees](https://docs.amoeba.farm/petri-cli/02-configuration-and-wallets/#who-pays-network-fees).

## Amoeba charges

| Action | Amoeba fee |
|---|---|
| Open a position, market or limit | None |
| Close a position (**Exit**, `petri close`) | None |
| Cancel an order | None |
| Settlement payout | None |
| Deposit into or withdraw from a market | None |
| Earn deposit or withdrawal | None |

## Sponsored transactions

When the sponsor pays, it also pays the rent of the accounts a transaction creates for you: your USDC token account when a withdrawal needs one, and your Earn position the first time you deposit. Nothing is taken from your USDC for it.

The sponsor co-signs only a transaction it prepared itself, byte for byte, after your wallet has signed it. See [Prepared transactions](https://docs.amoeba.farm/developer/06-operation-plans/).

## When your wallet pays

With Petri's `local` route, your wallet pays each transaction's network fee in SOL, so it needs some SOL. Without SOL, Petri's default route, `auto`, uses the sponsor.

## What is not a fee

- **The price of a position.** A long's cost and a short's deposit go into the market behind the position, against the other side of the trade. Amoeba does not take them.
- **The spread between bids and offers, and price movement.** These come from the market. They are not hidden charges. Closing a position right after opening it sells at the bid, so it returns less than it cost.
- **Cash a resting order holds.** It is released when you cancel the order.

## Your total cost

A position's result is what it pays at settlement, or what closing it returns, minus what you paid to open it. A position that settles at zero loses everything you paid.

Before signing, compare the wallet review with the ticket. If the transaction asks for anything you did not request, stop and do not sign.

See [Capped futures](https://docs.amoeba.farm/trading/03-capped-futures/) and [Risk disclosures](https://docs.amoeba.farm/reference/03-risk-disclosures/).
