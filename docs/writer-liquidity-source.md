# Writer liquidity source interface

This September 9 release consumes published SDK source
`ac30e32bbd151d8819e4a06eafdf33570bab0eab` and Lean source
`d20e12ab03d5c0c20df04b21d6054dfffa29d311`. Its deployment identity binds the
finalized current Devnet program capture in `release/current-governance-status.json`.
The release uses focused checks and production builds. Every live operation
still requires fresh permission and account validation.

## Commands and TUI

`writers liquidity --sleeve <ADDRESS> --owner <WALLET> --series-index <0..19>`
reads the exact selected series with sleeve A/W/B/R, actual cash, writer pool
quote, issuer inventory, external open interest, and frozen buyback budgets.
Unavailable cash remains unavailable; B is historical premium after primary sale
fees but before buyback spending and other expenses, not cash.

The frozen manager uses `writers liquidity-initialize`, `liquidity-add`, and
`liquidity-remove`. All take `--sleeve` and `--series-index`. Add also requires
`--issue-amount <CONTRACT_ATOMS>`; add/remove take repeated
`--bin <ID:OPTION_ATOMS:QUOTE_ATOMS>`. Supply one through eight strictly ascending
unique bin IDs in 1..2048; each entry must contain a positive amount. A position
has at most 32 populated bins. Add issuance must equal the option allocation
sum and be a whole number of contracts (1_000_000 atoms each).

Remove returns quote to sleeve cash and burns unsold issuer options. No manager
destination or fungible LP share is accepted. `writers liquidity-sweep` accepts
the common sleeve/series fields and permissionlessly returns uncommitted proceeds
to the canonical sleeve vault. Flat ownership does not grant management rights.
The Writers TUI provides these inputs within its existing form/review flow.

Remove is also permissionless during a pending close, CloseStaging, Expired,
or SettlementFinalized sleeve state, while the pool admits removal. This cleanup
does not grant add/initialize authority or pay the caller. The fresh operation
mask and native validation determine eligibility.

`writers withdraw --sleeve <ADDRESS> --amount <USDC_ATOMS>` preserves principal
withdrawal eligibility. Existing close and Flat-transfer flows remain separate.
`writers claim --variant collective-long` now binds the exact selected series.

## Historical refunds

`writers refunds --owner <WALLET> [--cursor <CURSOR>] [--limit <1..32>]` performs
bounded discovery independent of active auctions. The default page is 16 rows.
Unknown/blocked rows remain visible. An expired cursor requires manual restart;
the client does not automatically retry or infer ownership from a global list.

`writers refund --auction <ADDRESS> --bid <ADDRESS>` reconstructs and revalidates
the exact native refund operation before signing. The bidder and canonical
stored destination are immutable inputs to validation. The TUI additionally
asks for the discovered historical sleeve to obtain its action mask; the signed
operation independently binds the actual sleeve. New auction bidding is removed
from onboarding, while its historical grammar remains hidden.

## Economics and execution

Writer liquidity shares the market's existing native DLMM with segregated
issuer accounting. Ordinary third-party liquidity remains independent. Only
eligible single-series buybacks releasing exact reserve are admitted. Joint
multi-series execution is unavailable; hypothetical other-leg retirement grants
no reserve credit. Frozen cash, monthly/per-series/per-transaction budgets,
conservative value, fee separation, and reserve-release limits apply.

Every direct mutation uses the SDK's validated semantic and native account
binding, a fresh operation-specific wallet mask, the established signer review,
and the existing prepare/submit/status admission context. Readiness for trading
does not disable an independently eligible historical refund. Policy setup is
a separate operator API, and MCP remains read-only/semantic-draft only.

The direct transaction adapter selects the SDK's versioned signer path for new
writer liquidity and the updated collective swap. Optional table bytes come
only from the validated plan and remain covered by fresh raw-account observation.
Legacy operations retain their SDK legacy message and validator. Candidate
liquidity compute setup is reconstructed by the SDK helper; it is not a measured
compute or packet-capacity attestation. Classic refund/withdrawal ATA setup is
rebuilt independently and checked by the same writer-plan validator.

Future release repinning must cover Cargo.toml/Cargo.lock, current release and
governance JSON/constants, source capability and MCP descriptions, packaging and
governance checker inputs, and dated evidence fixtures. The source export
allowlist now includes the new writer_liquidity module and this document. No
export, package checker, or public publication was executed. The later local
build stage is recorded separately in `local-candidate-build-20260909.md`.
