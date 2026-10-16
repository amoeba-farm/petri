# Evidence rules

Every source and every price observation must be reproducible from public information. Someone else has to be able to open the same page, read the same field, and reach the same number. A number typed into a form is not evidence.

These rules apply to source proposals, opening values, update claims, and challenges against any of them. They also apply to research you submit on the **Source discovery** page. For how sources turn into the settlement index, see [How the oracle works](https://docs.amoeba.farm/oracle/01-how-it-is-structured/).

## Acceptable sources

A source is a public, repeatable price series for one defined item. The web app offers five source types:

| Source type | Example |
|---|---|
| Retailer | A store's product page with a listed unit price |
| Distributor | A distributor catalog row for one part number |
| Manufacturer | A manufacturer's published price page |
| Benchmark | A recurring public price assessment |
| Public API | A documented public data series |

Define the source precisely enough that there is only one possible reading:

- **Product definition**: the exact item and specification.
- **Price field**: which price on the page, for example the unit price excluding tax.
- **Region** and **Currency**: where the price applies and in which currency.
- Any quantity tier, condition, or other qualifier that changes the price.

An API that mirrors a product page is another way to read the same source, not a second independent source. Two different hosts are not proof of independent ownership either.

## Evidence that does not establish a source

These can describe a real transaction, but they do not create a public series that others can check over time:

- Private invoices, broker messages, and emails.
- Personalized, login-only, or quote-on-request prices.
- Search-result snippets.
- One-off auctions from a single seller.
- An aggregator that only republishes another feed.

## Submitting a candidate

A candidate on **Source discovery** names exactly where it belongs and what it measures:

| Field | Rule |
|---|---|
| Market ID, Expiry ID | The exact market and oracle month the source is for. |
| Bucket ID | The 64-character ID of the product bucket. Do not guess it from a label. |
| Public source URL | See the locator rules below. |
| Source type | One of the types above. |
| Product definition | The exact item and specification, up to 3,072 characters. |
| Price field, Region | Which price, and where it applies. |
| Currency | A three-letter code such as `USD`. |
| Evidence | 12 to 4,096 characters explaining how the source matches the bucket and what reviewers should check. |

The locator is the public URL of the source page. The backend accepts only URLs that:

- Start with `https://` and are at most 384 characters long.
- Use a public, lowercase domain name. Local or test names are rejected.
- Contain no username, password, port, fragment (`#`), or secret-looking query parameter such as a token.

Submitting a candidate never fetches the page. Reviewers open it themselves. If someone already submitted the same source, the original submission keeps the credit.

## Opening values and updates

An opening value fixes the denominator for all of a source's later movement. An update claim reports a changed value during Game Mode. Both need the same kind of evidence, and the program checks it on chain.

| Field in the web app | What it must contain |
|---|---|
| Frozen source ID | The accepted source being observed. |
| Opening value or New observed value | The value of the defined price field, not another variant or tier. |
| Observed at | The moment the price was on the page. |
| Archived evidence URL | A Wayback Machine capture of the source page. |
| Claim stake | The USDC bond posted with the claim. |

The program rejects the claim unless the archive URL:

- Has the form `https://web.archive.org/web/14-digit UTC timestamp/source URL`.
- Archives exactly the source's registered locator.
- Has a capture timestamp equal to **Observed at**.
- Is at most 384 bytes.

For an update, the observed time must also fall between the month's listing and its expiry, and cannot be in the future. The value cannot be zero.

> **Caution:**
> A capture that does not show the claimed value in the defined field can still be challenged, even though it passes the format check. Before you submit, open the capture and confirm the value, the variant, and the time.

## Permanent evidence records

Source definitions, locators, and archive URLs are stored on chain in full, as sealed records. Once sealed, the program offers no way to close or edit them. A hash or a short label alone is not the evidence. When you review a record, check the original URL, the archive, the capture time, and the source it is bound to.

A stored evidence record does not mean the claim was accepted. Acceptance, challenge outcomes, and the observation history are separate states.

## Unchanged and unavailable sources

Reporting that a source has not changed is not a new observation and earns no reward. When no fresh value exists, the source keeps its last accepted value.

If a source page is temporarily unavailable, do not replace it with another page. The source list is frozen for the life of the month. Missing coverage at settlement follows the rules in [How the oracle works](https://docs.amoeba.farm/oracle/01-how-it-is-structured/#settlement).

See also [Challenges and disputes](https://docs.amoeba.farm/contributors/03-challenges/).
