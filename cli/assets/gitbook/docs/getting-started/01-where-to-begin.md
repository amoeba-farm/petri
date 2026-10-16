# Getting started

This page gets you from zero to a first trade, then points you to the pages for your role.

## Before you start

- **Solana mainnet, real money.** Capped futures run on Solana mainnet and settle in real USDC. The programs have not been audited.
- **An account.** Sign in to the web app at [amoeba.farm](https://amoeba.farm) with Google (a Solana wallet is created for you) or with Phantom, Solflare or Backpack.
- **USDC in your wallet.** You pay for positions in USDC on Solana (mint `EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v`) from your wallet. Send USDC to your wallet address (copy it from **Account**) from an exchange or another wallet, or use **Add funds** on **Account**, which opens Stripe's USDC on-ramp where it is available. A token with the same name and another mint is not USDC.
- **No SOL needed.** Amoeba's sponsor pays the network fee of every web app transaction.
- **The risks.** Read [Risk disclosures](https://docs.amoeba.farm/reference/03-risk-disclosures/).

## Your first trade

1. **Open a market.** Go to the RAMX or NANDX trade page from **Components** or **Products**, or from **Trading** in the phone menu.
2. **Pick a direction.** **Long** gains if the index settles higher, **Short** if it settles lower. Leave **Market** selected to trade now.
3. **Set the leverage.** The slider sets the losing end of your range: the entry minus entry ÷ leverage for a long, plus it for a short. The default is 10×.
4. **Set the winning end.** **Up to** for a long, **Down to** for a short. Leave it empty to use the market's suggested range, which reaches about as far as the index is likely to move by expiry.
5. **Choose the expiry and the amount.** Pick the **Expiry** and enter the **Amount** of USDC to put in.
6. **Check the ticket.** The payoff shape shows your range. **Entry** is the price you get, **Per point** is what the position gains for each index point inside the range, and **You pay** is what goes in now: the most you can lose.
7. **Open.** Select **Open long** or **Open short** and approve in your wallet. The position shows under **Portfolio** in your ledger.
8. **Close or hold.** **Exit** sells the position at the best price on the book and sends the USDC to your wallet. If you hold it, it settles at expiry on the oracle's final index. The payout lands in your cash in that market: **Withdraw** under **Claims** moves it to your wallet, and **Withdraw all** on **Account** collects every market's free cash.

The [Web app](https://docs.amoeba.farm/getting-started/03-web-client/) page describes every control, and [Capped futures](https://docs.amoeba.farm/trading/03-capped-futures/) explains the payoff.

## Choose your path

**Traders.** Start with [Capped futures](https://docs.amoeba.farm/trading/03-capped-futures/) for payoffs, [RAMX and NANDX markets](https://docs.amoeba.farm/overview/02-markets-ramx-nandx-future/) for what you are trading, and [Fees and transaction costs](https://docs.amoeba.farm/trading/02-fees/). Then read [Trading and liquidity](https://docs.amoeba.farm/markets/02-liquidity-and-trading/) and [Market lifecycle](https://docs.amoeba.farm/reference/01-market-lifecycle/).

**Liquidity providers.** You can deposit USDC into the Earn vault on **Earn**, which funds the treasury maker in every market and shares its result, or place your own limit orders. Earn withdrawals pay your shares at the vault's lowest value, at once. Read [Providing liquidity](https://docs.amoeba.farm/earn/01-providing-liquidity/), [Earn](https://docs.amoeba.farm/earn/02-earn/), and [Earn settlement](https://docs.amoeba.farm/earn/03-earn-settlement/).

**Oracle contributors.** Read [How the oracle works](https://docs.amoeba.farm/oracle/01-how-it-is-structured/), [Evidence rules](https://docs.amoeba.farm/reference/02-evidence-rules/), and [Challenges and disputes](https://docs.amoeba.farm/contributors/03-challenges/). Check the bond, deadline, and reward rules before you commit funds. The **Oracle** page in the web app shows sources and, under **Contribute**, the Petri commands for the current stage. Contributions are signed and sent with Petri's `petri oracle` commands.

**Developers.** Start with the [API reference](https://docs.amoeba.farm/developer/01-api/) and the [Quickstart](https://docs.amoeba.farm/developer/03-sdk-quickstart/). [Deployment](https://docs.amoeba.farm/technical/05-deployment/) lists the chain and program addresses.

**Terminal and agent users.** [Petri](https://docs.amoeba.farm/getting-started/02-cli/) gives you repeatable commands and JSON output. To have an AI agent read markets, see [Connect an AI agent](https://docs.amoeba.farm/getting-started/04-claude-code-codex/). Keep the final approval of any transaction in your own terminal or wallet.
