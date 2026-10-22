# How the oracle works

The oracle produces one number per monthly cohort: the settlement index that decides what every position in that month's RAMX, NANDX, or BATX market pays. This page explains where that number comes from, who can influence it, and how you can check it.

## The short version

- Each cohort tracks a fixed list of public price sources for memory (RAMX), storage (NANDX) or battery-cell (BATX) products.
- Every source's movement is measured against its own opening value. The index starts at **100**.
- Sources are grouped into buckets with fixed weights that add up to 100%. The index move is the weighted sum of the bucket moves.
- At expiry, each bucket settles on a median of its sources' prices over the last five weekdays.
- The operator rules on ordinary disputes first. Unresolved disputes, and buckets without enough sources, go to the five-seat council, where three matching votes decide.

## What the index measures

Each source is one repeatable public price for one defined product. From the months expiring after 2026-11-01, a source is a **comparison series**: one defined price-assessment row, for example a DDR5 16Gb chip contract-price row on a price-tracking page, or one recurring direct quote. Its definition fixes the bucket, the statistic, the unit, the currency, the period and the terms before it reports, and each price it publishes for a period is an **observation**. In the launch month a source is a category price on memorymarket.com, and its definition fixes the product, variant, price field, region, and currency. See [Evidence rules](https://docs.amoeba.farm/reference/02-evidence-rules/).

A prospective extension, [rules v2](https://docs.amoeba.farm/reference/02-evidence-rules/#authenticated-evidence-rules-v2), would also admit a price behind a contributor's own sign-in when every observation of it is a web proof that anyone can re-verify without signing in. It gives such a source no extra weight, and it is not yet on production. See [Authenticated evidence (web proofs)](https://docs.amoeba.farm/contributors/05-authenticated-evidence/).

For an accepted opening value `P_open` and a later accepted value `P_t`, the source's movement is:

```text
delta = P_t / P_open - 1
```

Measuring each source against its own opening lets two sources that quote different price levels, such as two publishers' assessments of the same chip, still be compared fairly. The oracle never averages raw prices from different sources.

Sources are grouped into buckets. Within a bucket every source counts equally, and the bucket moves by the median of its sources' movements. Support capital behind a source does not give it more weight. Each bucket has a fixed weight from the product's definition, and the weights add up to 100%. From the months expiring after 2026-11-01 the buckets are commodity memory chips and battery cells, listed in [Products, buckets, and weights](https://docs.amoeba.farm/reference/04-products-indexes-buckets/):

```text
index move = sum over buckets of (bucket weight × bucket move)
settlement index = 100 × (1 + index move)
```

The program works in basis points and rounds the final value half up. A 3% weighted move settles the index at 103. The exact formulas are in the [Math appendix](https://docs.amoeba.farm/reference/math-appendix/).

## Who set the launch month's sources

The first RAMX and NANDX months on Solana mainnet, the launch month, did not use open, bonded source formation. The governance council approved a launch document instead: a fixed registry of public price pages with each one's bucket, weight and printed opening value, recorded on chain in one launch per index. RAMX and NANDX each launched this way once; every later month forms through the ordinary process below. BATX has no launch month: its first month, BATX Nov 26, forms through the ordinary process too.

| Market | Sources | Buckets | Source pages |
|---|---|---|---|
| RAMX | 13 | One per source | memorymarket.com RDIMM, UDIMM, and SODIMM price pages |
| NANDX | 22 | One per source | memorymarket.com channel SSD, OEM SSD, flash wafer, eMMC, and UFS price pages |

All sources come from one website, so a problem with that site affects the whole index. Each bucket holds a single source, which also changes how the month settles. See [Settlement](#settlement).

Game Mode updates and challenges run through the ordinary steps below.

## The ordinary monthly process

The program also supports open, bonded source formation. An oracle month then moves through these stages, which the Oracle page shows as **Current Stage**:

| Stage | Length | What happens |
|---|---|---|
| Scramble | 4 days | Contributors propose public sources with a listing bond and back proposals with support bonds. |
| Challenge | 2 days | Proposals can be disputed with a challenge bond. New proposals and support stop. |
| Freeze | 1 day | Disputes are resolved and the source list is locked. |
| Opening | 1 day | Each source's opening value is submitted with archived evidence and a bond. |
| Game Mode | Until expiry | Contributors report changed values with evidence and a bond. Sources and weights stay fixed. |

All bonds are in USDC, and every product bucket must end Freeze with at least one surviving source. Bond sizes and reward budgets are set separately for each month and bucket. See [Bounties and rewards](https://docs.amoeba.farm/oracle/03-bounty-collection/).

From the months expiring after 2026-11-01, every proposed series and every observation also gets one **routing status**, which says whether and why it counts. Only **Counts this month** counts toward settlement: it marks a source in the month's committed series, which are the month's sources from Freeze on. A relevant source that arrives later reads **Not in this month** and can be proposed for a later month. The statuses are listed in [Evidence rules](https://docs.amoeba.farm/reference/02-evidence-rules/#routing-statuses).

### Game Mode updates

During Game Mode, a contributor can report that a source's price changed:

1. The contributor commits the update with its bond, then reveals it shortly after, so others cannot copy it before it is public.
2. For at least 24 hours after the reveal, anyone else can challenge it with a corrected value and their own evidence.
3. After 24 hours, the operator can accept an unchallenged update. A challenged update is resolved as described in [Challenges and disputes](https://docs.amoeba.farm/contributors/03-challenges/).

A source with no accepted update keeps its last accepted value. Reporting an unchanged price is not an update and earns nothing. Nobody can add or remove sources, change weights, or swap a broken source for a new one during Game Mode.

## Settlement

Every cohort expires at 00:00 UTC on its expiry date. Each bucket's settlement value is computed in two steps:

1. **Over time**: for each source, take the median of its accepted values observed in the five UTC weekdays (Monday to Friday) up to expiry.
2. **Across sources**: take the median of those results within the bucket.

The fixed bucket weights then combine the bucket moves into the index.

If a source has no accepted value in that window, it carries forward its latest accepted value from before the window. Carry-forward is not a new observation and earns no reward.

A bucket needs enough eligible sources to settle by itself: at least three, or at least half of its sources if that is more. If it falls short, the lookback is extended once by another five weekdays. If it still falls short, the bucket goes to the council. The council can only choose between values already recorded on chain for that bucket. It cannot enter a new price, and the oracle never drops a bucket and re-weights the rest.

> **Caution:**
> In the launch month every bucket holds one source, below the three-source minimum, so every bucket goes to the council. The council confirms each bucket's measured median, computed from its source's recorded values as above, or keeps its last accepted value; it cannot enter a value of its own. For the launch month it can rule as soon as three seats agree, from the day after expiry.

After expiry there is a 24-hour grace period. New update challenges can still be filed during it, and disputes already admitted can finish afterwards. When every dispute is closed, anyone can finalize the oracle month. The settlement record is then written on chain. It must carry signatures from the program's registered settlement signers, who cannot be the operator's keys, and its price must equal the value computed from the finalized month. Payouts follow the [market lifecycle](https://docs.amoeba.farm/reference/01-market-lifecycle/).

## Checking the oracle yourself

Open **Oracle** in the web app and choose a **Market** and **Month**.

- The summary shows **Current Stage**, **Source Records**, **Source Links**, and when the data was **Updated**. "Last known" in front of a stage means the latest refresh could not confirm it.
- The **Sources** tab lists one row per website with its **Link**, number of **Records**, and combined **Weight**. Select **View** to see each record's bucket, **Status**, **Weight**, and **Source ID**. For a month on chip or wafer buckets, the tab first shows each bucket's weight, and each record shows the row its series reads and its routing status. Hover a routing status to see what it means; when reasons decided it, select it to see them.
- The **Submissions** tab lists the sources research agents and wallets have sent in, newest first, with each one's bucket, status and routing status.
- The **Contribute** tab lists the Petri commands for the current stage's contributions. Contributions are signed with your own wallet in Petri; the web app does not submit them.

Each on-chain source record holds its locator, opening value, current value, and observation count. Opening and update evidence is stored on chain permanently.

Read [Evidence rules](https://docs.amoeba.farm/reference/02-evidence-rules/), [Challenges and disputes](https://docs.amoeba.farm/contributors/03-challenges/), and [Oracle roles](https://docs.amoeba.farm/oracle/02-token-holders-and-contributors/) for the details behind each step.
