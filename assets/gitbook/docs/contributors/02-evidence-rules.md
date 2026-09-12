# Evidence Rules

Evidence makes the oracle challengeable.

V1 should prefer public, repeatable sources and public archive evidence. Private evidence can become a future specialization, but it should not be the default launch path.

## Accepted source categories

| Category | Meaning |
| --- | --- |
| Retailer product page | Public page with a fixed SKU or product and an identifiable posted price. |
| Distributor catalog page | Public or reproducible catalog page with fixed part number and price or tier price. |
| Manufacturer product or store page | Manufacturer page with fixed product and posted value. |
| Market assessment or benchmark | Recurring public assessment for a fixed RAM/NAND specification or bucket. |
| Public API endpoint | API exposing the same public series. It is not independent if it duplicates another source. |

## Rejected source categories

| Category | Reason |
| --- | --- |
| Private invoices, broker DMs, emails | Not reproducible enough for V1 challengers. |
| Single seller auctions | Seller, condition, and inventory can shift underneath the source. |
| Search snippets | Not stable source records. |
| Pure aggregators | Usually duplicate an underlying source. |
| Login-only or personalized pricing | Not publicly reproducible. |
| Copy-pasted numbers | No independent evidence trail. |

## Wayback and archives

For public web sources, the cleanest evidence object is a Wayback or archive URL.

The archive should:

- resolve;
- point to the frozen locator;
- show the claimed value;
- match the source definition;
- have a timestamp inside the allowed window.

## Source failure

If a source is temporarily unavailable, the last accepted state carries forward.

If a source becomes permanently non-reproducible, the next monthly source selection process can remove or replace it. Game Mode should not rewrite frozen weights mid-month.
