# About Capped Options

Amoeba Farm starts with capped options because hardware input markets are not ideal for liquidation-heavy products.

Hardware prices can update slowly, change in jumps, or require evidence windows. A capped monthly option lets the user know the maximum loss and maximum payout before entry.

## The Basic Idea

A capped option pays from a market movement, but only up to a defined maximum.

For a buyer, the key questions are:

- What market am I trading?
- What month or expiry?
- What direction?
- What premium do I pay?
- What is my maximum loss?
- What is my maximum payout?
- When does it settle?

## Why the Cap Matters

The cap makes the risk easier to inspect.

Buyers know the most they can lose. Writers know the worst-case payout they are backing. The market does not need to pretend that hardware input prices can support constant liquidation checks.

## Spreads

A call spread is a common way to implement a capped upside view. A put spread is a common way to implement a capped downside view.

In both cases, the product should show the user the bounded risk before confirmation.

## Tiny Example

A RAMX call spread might show:

| Field | Example |
| --- | --- |
| Market | RAMX |
| Expiry | Current monthly maturity |
| Direction | Upside |
| Premium paid | 10 USDC |
| Maximum loss | 10 USDC |
| Maximum payout | 50 USDC |
| Liquidation | None |

Those numbers are examples only. A live trade preview should show the actual premium, fees, maximum loss, maximum payout, and settlement date before the user confirms.

## Settlement

At expiry, the contract uses the published oracle result and the product's payoff rule.

The oracle computes the market movement. The contract computes the payout.
