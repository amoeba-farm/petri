# How the Oracle Is Structured

The Amoeba Farm oracle turns accepted source observations into a settlement-grade market movement.

It does not simply average random prices. It follows a monthly recipe.

## Monthly Flow

```text
monthly source selection
  -> source support and challenge
  -> source freeze
  -> opening prints
  -> live source updates
  -> final oracle output
```

| Stage | What happens |
| --- | --- |
| Source selection | Users propose repeatable public sources and support the ones they believe should count. |
| Challenge | Bad, duplicate, stale, private, or wrong-bucket sources can be challenged. |
| Freeze | The surviving source map and weights are fixed for the month. |
| Opening print | Each frozen source gets a starting value. |
| Live updates | Users submit evidence when frozen sources change. |
| Settlement output | The oracle reports the final market movement for the contract. |

## Source Recipe

A source recipe defines which sources count for the month and how they are weighted.

For RAMX, the source tree is organized around the RAMX module spot index. The tree lets users drill from the market to product rows and then to terminal source pins.

## Opening Prints

Opening prints set the starting value for accepted sources.

After that, a source is measured against its own opening value. This avoids pretending that every source is directly comparable as a raw price.

## Live Updates

During the live update stage, users can submit evidence when a frozen source changes.

Game Mode updates source states. It does not add new sources, remove sources, or change frozen weights.

## Evidence Packet

A useful evidence packet should make the claim reproducible.

It should include:

- source definition;
- canonical locator;
- claimed product bucket or row;
- observed value;
- source timestamp;
- archive or evidence pointer;
- enough context for another user to verify the claim.

Private quotes, broker messages, login-only screenshots, search snippets, and wrong-product evidence should not be treated as normal public sources.

## Final Output

The oracle combines source-local movement through the frozen recipe.

```text
source movement
  -> row or bucket movement
  -> market index movement
  -> contract settlement
```

The important user question is simple: why did the index move? A good oracle page should answer that before showing deeper audit detail.

The oracle computes the movement. The market contract applies the payoff.
