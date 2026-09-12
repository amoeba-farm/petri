# Fees

Fees should be visible before a user enters a trade or submits an oracle action.

This page describes fee categories. Exact live values should be shown on the market, trade, or action screen for the active product version.

## Trading Fees

A trading flow may include:

- premium paid to enter a position;
- venue or pool fees;
- network fees;
- route or execution costs;
- claim or settlement transaction costs.

The user should see total cost, maximum loss, and maximum payout before confirmation.

## Writer and Liquidity Fees

Writers and liquidity providers may earn premium or fees for taking bounded payout risk.

They should review:

- locked collateral;
- maximum payout;
- expiry;
- closeout rules;
- claim path;
- fee share, if any.

## Oracle and Contributor Fees

Oracle actions may require bonds, stakes, or challenge deposits.

Those are not ordinary trading fees. They exist to discourage bad submissions and fund review or reward flows.

Before submitting oracle work, a user should see:

- required stake or bond;
- what can be lost;
- what can be earned;
- when the action finalizes;
- how challenges work.

## Exact Values

Do not rely on this page for exact live numbers.

Exact fees, rewards, bonds, and network costs should come from the active product page, CLI output, or transaction preview.

Use this order of authority:

1. signed transaction preview;
2. active product or market page;
3. CLI output for the exact command being run;
4. static documentation for category explanations only.
