# Oracle USDC rewards

Current oracle work can earn USDC rewards when the canonical Spread state marks the work claimable.

The current reward lane is tied to the active Oracle month, source or update claim, and the canonical reward receipt. Petri reads those facts from the current backend projection and never invents a claim from a phase or a static catalog row.

## Current claim flow

1. select a current market and full series label;
2. review the source or update claim and its exact reward amount;
3. create the `ClaimOracleUsdcReward` tag-174 draft with the current receipt accounts;
4. sign and submit only after the current account and reward projection pass validation.

The available reward kind, source id, claim id, and amount come from the current projection. No reward is shown while the authoritative current state is empty or the projection is unavailable.

USDC rewards are separate from option settlement and never change the option contract's payout.
