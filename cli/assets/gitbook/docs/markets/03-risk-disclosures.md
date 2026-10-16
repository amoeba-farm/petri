# Risk disclosures

You can lose the full amount you put into an Amoeba Farm position or the Earn vault. Paying for positions in full up front removes margin calls and liquidations. It does not guarantee a profit, protect an Earn deposit, guarantee that you can exit, or remove technology and legal risks.

This page summarizes the main risks. The web app links its Risk Disclosure and Terms of Use from the menu; they apply to your use of it. Nothing here is investment, legal or tax advice.

## The deployment

- Capped futures run on Solana mainnet and settle in real USDC. You can lose real money. The programs have not been audited.
- The programs' rules can change through governance upgrades. See [Governance and upgrades](#governance-and-upgrades).
- If a market's oracle period has not settled after expiry, the governance council can set the market's settlement value instead of the published index result, at any time after expiry.
- Each market is one account, so all trading in a market happens one transaction at a time, and each market has fixed capacities: 64 resting orders, 128 trader accounts and 1,024 open positions. An order beyond a capacity fails.

## Position losses

- **Longs.** A long loses everything it paid if the index settles at or below its range's low end. A payout smaller than what you paid is still a loss.
- **Shorts.** A short loses everything it put in if the index settles at or above its range's high end.
- **Higher leverage** puts the losing end closer to your entry: a smaller move against you loses the whole position.
- **No liquidation, but no guaranteed exit.** There are no margin calls. That does not stop a position from losing value, and a pause or failure can keep you from acting.
- **Ranges cut both ways.** A long stops gaining at its high end and a short at its low end. A position on a range is not the same as holding the hardware or an open-ended future.

## Liquidity and execution

- A range can have thin depth, a wide gap between bids and offers, or nothing to trade against at your size. A displayed price does not guarantee you can trade your full quantity at it.
- Your limit and slippage settings stop bad fills. They do not create liquidity, and closing a position can find no buyer at a price you accept.
- Resting orders fill only if someone trades with them. They stop filling at expiry.
- Nobody, including Amoeba's treasury maker, has to quote at a price you want. The maker quotes only with the cash it has; when that runs out, only other traders' orders remain.
- Trading can stop because of a governance freeze, an infrastructure failure, the oracle period not being live, or expiry. You may have to hold a position to settlement.

## Earn

- The Earn vault holds the maker's positions in every market it funds. If they settle against it, the vault loses money and every deposit goes down. A loss in one market is shared by all depositors.
- Deposits are priced at the vault's highest possible value and withdrawals at its lowest, so a deposit withdrawn before its markets settle returns less while the vault holds open positions.
- A withdrawal pays at the vault's lowest value, from its idle cash and then its cash in its markets; it takes down the vault's quotes in a market where it needs their cash. The vault's last shares cannot leave while it holds open positions.
- The vault's operator trades for it and decides which markets its cash goes to. Nothing limits how it quotes; depositors rely on its quoting, as with any managed market maker.

See [Earn](https://docs.amoeba.farm/earn/02-earn/).

## Oracle and basis risk

- Price sources can be wrong, stale, duplicated, manipulated, unavailable or mis-matched to a product. Medians, public evidence and challenges reduce these risks but do not remove them, especially when several sources share an error. In the launch month every bucket has a single source, all on one website, so each bucket's settlement value is confirmed by the council from the values recorded for it.
- The final value is each source's median over a five-weekday window before expiry, then the median across sources. It can differ from the last live print, an intraday extreme, or a value you saw outside that window.
- The index measures movement from each source's opening value. A wrong opening value can distort everything after it.
- RAMX and NANDX are defined benchmarks. They are not complete indexes of the memory or storage market. What you actually pay or receive for hardware can move differently because of specification, volume, region, supplier terms or timing. A position is not a perfect hedge.
- Positions settle in USDC. They give you no right to RAM, drives, inventory or physical delivery.

## Settlement timing and payouts

- The oracle can finalize no earlier than about 24 hours after expiry. Disputes or missing evidence can delay it further. Payment is not immediate at expiry.
- Amoeba's keeper settles markets and positions after the oracle settles. That depends on Amoeba's services running, although the program lets anyone do it.
- Settlement pays into your free cash in the market. It reaches your wallet only when you withdraw it.

## Software, wallets and custody

- Smart contracts, the futures API, RPC nodes, wallets and Amoeba's own services can fail or be compromised.
- A transaction can fail at preparation, signing, submission or confirmation. Earlier steps are not undone when a later step fails. Check status before retrying, and keep your transaction signatures.
- Your cash in a market is held by the market's account, not by your wallet, until you withdraw it.
- A stolen key, a malicious approval or a phishing site can cause losses that no range limits. Check the site address and every wallet approval.
- Nothing is insured by Amoeba or by any deposit-protection scheme.

## Governance and upgrades

- The futures and oracle programs can be upgraded. Upgrades need approval from a five-seat council (three approvals) and wait out a delay before they execute. The council can also freeze them.
- That makes control concentrated. Council decisions can be wrong, and an upgrade can change how the program behaves.
- A market's index, expiry and oracle month are fixed once set, but that is not the same as the software or the API being unchangeable.

## Costs

Amoeba charges no trading fee and no funding. The spread between bids and offers, price movement and losses all reduce your result. See [Fees and transaction costs](https://docs.amoeba.farm/trading/02-fees/).

## Legal and eligibility

How these products are treated legally and for tax depends on where you are and what role you play. Being able to connect a wallet does not mean you are eligible. Do not get around access restrictions. You are responsible for your own tax and reporting obligations.

## Before you commit funds

- Check that you are on the Amoeba site, and that you are trading the market, expiry, side, range, lots and amount you intend.
- Check **You pay** on the ticket and the transaction in your wallet.
- Use only money you can afford to lose and can leave in the market until settlement.
