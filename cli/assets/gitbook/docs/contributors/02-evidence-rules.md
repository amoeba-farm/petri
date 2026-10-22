# Evidence rules

Every source and every price observation must be reproducible from public information. Someone else has to be able to open the same page, read the same field, and reach the same number. A number typed into a form is not evidence.

These rules apply to source proposals, opening values, update claims, and challenges against any of them. They also apply to research you submit on the **Source discovery** page. For how sources turn into the settlement index, see [How the oracle works](https://docs.amoeba.farm/oracle/01-how-it-is-structured/).

> **Note:**
> This page has two versions of the rules. Version 1 covers public evidence. It is everything from here to [Authenticated evidence (rules v2)](#authenticated-evidence-rules-v2), and it is what the oracle applies today. Rules v2 is a prospective extension for prices behind your own sign-in. It is not yet on production, and it changes nothing for existing sources or months.

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

From the months expiring after 2026-11-01, RAMX and NANDX count only contract prices: a published contract-price assessment, or a recurring direct contract quote. BATX can also count spot prices and firm offers, but never listings or forecasts. Each source is then a comparison series, defined as below.

## Evidence that does not establish a source

These can describe a real transaction, but they do not create a public series that others can check over time:

- Private invoices, broker messages, and emails.
- Personalized, login-only, or quote-on-request prices.
- Search-result snippets.
- One-off auctions from a single seller.
- An aggregator that only republishes another feed.

Rules v2 changes one item on this list, for login-only pages only: such a page can establish a source when every observation from it is a web proof. Personalized and quote-on-request prices stay excluded. See [Authenticated evidence (rules v2)](#authenticated-evidence-rules-v2).

## Comparison series

From the months expiring after 2026-11-01, RAMX and NANDX settle on chip and wafer buckets, and BATX settles on battery-cell buckets from its first month (see [Products, buckets, and weights](https://docs.amoeba.farm/reference/04-products-indexes-buckets/)). A source for one of those months is a **comparison series**: one repeatable priced object or published assessment, with its terms and statistic fixed before it reports. Its definition (`amoeba.comparison-series.v1`) is stored on chain with the source and must state:

- its series ID, definition version and bucket;
- where it comes from: the publisher, the page and the exact row;
- its evidence class, a published assessment or a direct price, and its price basis: contract for RAMX and NANDX; contract, spot or a firm offer for BATX;
- its technical scope: for RAMX the DRAM generation, die density, die count, condition, grade, organization, speed and package; for NANDX the 3D architecture, cell type, die density, testing scope and layer scope; for BATX the cell chemistry, casing, application, condition, capacity, nominal voltage, performance class, tax basis and delivery terms;
- its physical scope and native unit, for example USD per chip, or CNY per Wh for a BATX cell;
- the price statistic, for example the average, and the currency;
- the procurement segment and the geography;
- the reference period and how often it is published;
- how it handles revisions, corrections and missing data.

Petri's `petri oracle series template`, `check`, `propose` and `show` commands and `petri oracle observe` work with these definitions and their observations.

### Published assessments and direct quotes

| Evidence class | What it is | What it must also define |
|---|---|---|
| Published assessment | A recurring contract-price assessment, such as one row of a publisher's monthly contract price table. | The exact assessment and row. It can take its scope from the publisher's methodology; it need not name each underlying customer, part number or order size. |
| Direct price | A recurring contract price for one exact part. | The manufacturer, exact part number and specification revision, quantity tier, delivery and pricing period, market, tax and freight, and payment, rebate or bundling terms. |

NANDX uses published assessments only. A listing or a firm offer is not an agreed contract price.

For BATX, a direct price is a seller's recurring quote for one exact cell. Its model, capacity, voltage, performance class and terms must be exact. A BATX series keeps one price basis; a change reads **Definition changed**.

### Broad scope is fine; missing information is not

Each scope field says whether it is `exact`, a `documented_mix`, `not_disclosed` or `not_applicable`. A documented mix, such as an assessment that covers several speed bins or layer generations, is a defined scope. What keeps a series from counting is unresolved core information: the product, the contract basis, what the price unit means, the period or the statistic. Undisclosed details of the underlying transactions do not, by themselves.

Nothing is assumed. An unknown condition is not new, an unknown grade is not standard production, and an unknown unit is not USD per chip. Never guess a unit, or turn a wafer price into a price per chip. For BATX, a price per cell becomes CNY per Wh only with the exact cell's documented rated energy. Never guess a cell's voltage or capacity.

### Observations

Each published price of a series for one period is an observation. It records the raw label, price, currency and unit, the statistic, the reference period, when it was published and when it was retrieved, and its evidence. Many screenshots of one assessment row are one observation, and capturing an unchanged monthly assessment again adds no weight. Reporting a different statistic, such as the high instead of the average, changes the definition; it is not an update.

## Routing statuses

Every series and observation for those months gets one routing status. BATX uses the same statuses. The Oracle page shows it in plain words under each source and submission.

| On the Oracle page | Status | Meaning |
|---|---|---|
| Counts this month | `ELIGIBLE_FOR_CONTRACT` | In the month's committed series, so it counts toward settlement. |
| Fits the bucket | `CLASSIFIED_CANDIDATE` | Fits a bucket. It counts once a month commits it at Freeze. |
| Already counted | `DUPLICATE_NO_EXTRA_WEIGHT` | Repeats a series or a release already counted, so it adds no weight. |
| Missing details | `NEEDS_METADATA` | A required product or pricing field is unresolved, such as a price with no stated unit. |
| Wrong product | `WRONG_PRODUCT` | Not the bucket's product, such as a 16GB module offered as a 16Gb chip, or two 8Gb dies offered as a 16Gb chip, or a battery pack offered as a BATX cell. |
| Not a contract price | `WRONG_PRICE_BASIS` | A spot price, listing, offer or forecast. For BATX, a listing or a forecast. |
| Research only | `RESEARCH_ONLY` | Useful evidence the settlement method does not use, such as a quote for one packaged NAND chip, a DDR5 32Gb series (a RAMX research candidate), or a BATX price in a currency other than yuan, or from outside China. |
| Not in this month | `OUTSIDE_COMMITTED_SERIES` | Relevant, but not in the month's committed series. It can be proposed for a later month. |
| Definition changed | `SCOPE_CHANGE` | The product, terms, unit, statistic or method changed from the series' opening, such as standard-production chips replaced with eTT parts. |
| Wrong period | `STALE_OR_WRONG_PERIOD` | Not the period the series reports, or older than what it already reported. |

A month's committed series are its sources from Freeze on. A routing status refuses nothing: evidence that does not count is kept with its reason, and carries no settlement weight.

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

## Authenticated evidence (rules v2)

> **Caution:**
> Rules v2 is prospective and not yet on production. It applies only where the oracle accepts the new evidence record. That needs a production notary and an upgrade of the oracle program, and neither exists yet; the extension is tried on Amoeba's development chain first. Until then, version 1 above is the whole rule set.

Version 1 excludes login-only prices because nobody else can check them. Rules v2 adds one case: a page behind your own sign-in, such as a paid price service or a distributor portal, is admissible when every observation from it is a **web proof**. A web proof shows what the site sent you, and anyone can re-verify it from the published proof bundle, without signing in, under the published verifier policy. Without a web proof, login-only pages stay excluded. To make one, see [Authenticated evidence (web proofs)](https://docs.amoeba.farm/contributors/05-authenticated-evidence/).

### What changes and what does not

| Case | Version 1 | Rules v2 |
|---|---|---|
| Public pages | Admissible. Openings and updates cite a Wayback Machine capture of the registered URL. | Unchanged |
| A page behind your own sign-in, with a web proof for every observation | Excluded | Admissible for catalog prices and published benchmarks |
| A login-only page without a web proof | Excluded | Still excluded |
| Quotations and transaction records, such as a price offered to you or an invoice | Excluded | Still excluded as sources, with or without a web proof |
| Weight of a source | Equal within its bucket | Equal. Authenticated evidence gets no extra weight. |
| Disputes | Challenges, then the council | The same |

Existing sources, frozen months and their methodology do not change. Rules v2 applies only to new sources and new observations, and only where the oracle accepts the new evidence record.

### What the record is

Authentication shows where a record came from. It does not change what the record is. Every web proof declares one of four evidence types:

| Evidence type | What it is | Can it be a source? |
|---|---|---|
| `catalog_price` | A price a seller lists for everyone with access, such as a catalog or price list | Yes |
| `published_benchmark` | A published index, benchmark or report value | Yes |
| `quotation` | A price offered to one buyer, possibly negotiated | No. It is recorded as a quotation, but it is not a public, repeatable series. |
| `transaction_record` | A completed order, invoice or fill | No, for the same reason |

Authenticated is not the same as a true market price. A genuine server can return a negotiated price, or content shaped by your account or your own choices. Every source rule of version 1 still applies: one defined item, the price field, region, currency, and any qualifier that changes the price.

### The source and its locator

- The registered locator is the exact HTTPS address the proofs are made for. It follows the locator rules of version 1: `https://`, at most 384 characters, a public lowercase domain name, and no username, password, port, fragment or secret-looking query parameter.
- A proof makes the whole response public. Choose an address whose response holds nothing private.
- An extraction manifest, a short file you write yourself, names the record and fields the proof is read from. It needs no approval. Later observations of a source must read the same series: a new manifest can change the label or add context fields, but a proof that reads another record, or another price, currency, unit or product field, is refused for that source.

### Opening values and updates with a web proof

The web proof takes the place of the Wayback Machine capture:

| Field | What it must contain |
|---|---|
| Frozen source ID | The accepted source being observed. |
| Opening value or New observed value | The value the proof shows in the defined field, exactly: a plain decimal with at most six decimal places, never rounded. |
| Observed at | The proof's witnessed time (see [Times](#times)). |
| Evidence | The proof bundle. Its digest is recorded on chain. |
| Claim stake | The USDC bond posted with the claim. |

The evidence record stored on chain has this form:

```text
amoeba-evidence-v1;m=web;t=TIME;d=DIGEST;u=LOCATOR
```

`TIME` is the witnessed time in Unix seconds, `DIGEST` the proof bundle's digest, and `LOCATOR` the source's registered locator. The program rejects the claim unless `LOCATOR` is exactly the registered locator and `TIME` equals **Observed at**, as it does for a Wayback capture. The other checks on an update's time and value stay as in version 1.

The program does not read the proof itself. The futures API verifies every proof when you submit it and ignores anything your client says about it. Anyone can fetch the bundle by its digest and verify it again. A proof that does not verify, or does not show the claimed value, can be challenged: see [Challenges and disputes](https://docs.amoeba.farm/contributors/03-challenges/#challenging-a-web-proof). As for Wayback evidence, challenges and the council remain the way to dispute it.

### What a proof makes public

| Part | Shown | Hidden |
|---|---|---|
| Request | The request line and the `Host` header, and the names of the other headers | The values of the other headers, including cookies and authorization |
| Response | The status line, the headers and the whole body | The values of `Set-Cookie` headers |

The whole body is always shown; hiding part of it is not supported. You see a preview of exactly this before anything is exported or submitted.

### Times

A web proof involves three different times. They are never interchangeable.

| Time | What it means |
|---|---|
| Witnessed time | When the source served the record, from the notary's clock. It is not when the price was set or a sale took place. |
| The source's own date | A date the source printed in the record. It is the source's statement, not an independently checked time. |
| Submission time | When the proof was submitted. |

**Observed at** is the witnessed time. A fresh proof of an unchanged record is not a new observation and, as in version 1, earns nothing.

### Duplicates

An observation is the published record itself: the site and address, the record and its revision when the source prints one, the value, currency, unit, quantity tier and the source's own date. It never depends on the proof, the session, the wallet or the account. The same record proven twenty times, in any sessions and by any subscribers, counts once in each accounting scope: once per bucket and month for source candidates, and once per source and month for updates. A new value, or a new revision of the record, is a new observation.

Rules v2 claims no account-level identity. Nothing proves that two proofs came from different people, or that a person uses only one account.

### Trust

Verifiers trust three things: the notary keys listed in the published verifier policy, Mozilla's root certificates, and the notary's clock. The policy lists keys, never websites, so it does not choose sources. Anyone can run a notary, and adding a key to the policy is a reviewed release. The production policy lists one notary, Amoeba's own, whose signing key is held in Google Cloud KMS. See [What you trust](https://docs.amoeba.farm/contributors/05-authenticated-evidence/#what-you-trust).

### Not supported

Sites that only use TLS 1.3, compressed responses, hiding part of the response body, HTML, PDF and spreadsheet pages (JSON only for now), account nullifiers, and production trust. See [What is not supported](https://docs.amoeba.farm/contributors/05-authenticated-evidence/#what-is-not-supported).

### Planned: email evidence

Email evidence, checked through the sender's DKIM signature, is planned as a second way to authenticate evidence, through the same steps. It is not available yet.

See also [Challenges and disputes](https://docs.amoeba.farm/contributors/03-challenges/).
