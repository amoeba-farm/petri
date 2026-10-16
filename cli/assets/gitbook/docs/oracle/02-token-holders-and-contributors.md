# Oracle roles: contributors, operator, and council

The oracle has a few distinct roles. Each has its own permissions and its own money at risk. Holding a token, a futures position or an Earn share gives you none of them.

| Role | What it does | What is at risk |
|---|---|---|
| Researcher | Finds public sources and submits them as candidates on **Source discovery**. | Nothing. No USDC or wallet signature is needed. |
| Bonded contributor | Proposes, supports, opens, updates, or challenges sources on chain. | The USDC bond posted with each action. |
| Operator | Sets up oracle months, accepts unchallenged updates, and rules first on challenges. | No bond. |
| Council | Decides cases the operator leaves unresolved and buckets without enough sources. Also governs program upgrades. | No bond. Five fixed seats. |

## Researchers

A researcher registers a permanent payout address and submits candidate sources with their evidence. A candidate is a suggestion in a queue. It has no weight in the oracle and authorizes no payment or transaction. See [Evidence rules](https://docs.amoeba.farm/reference/02-evidence-rules/) for what a candidate must contain.

A candidate becomes an on-chain proposal only when someone posts its USDC listing bond. Sponsorship preparation is currently unavailable, so Source discovery hides its **Sponsor** tab; the researcher, or anyone, can post the bond with `petri oracle sources propose`. See [Bounties and rewards](https://docs.amoeba.farm/oracle/03-bounty-collection/#researchers-and-sponsors) for how a sponsored bounty is split.

## Bonded contributors

On chain, contributors act during the stages of an oracle month:

- **Scramble**: propose a source with a listing bond, or back a proposal with a support bond.
- **Challenge**: dispute a proposed source with a challenge bond.
- **Opening**: submit an accepted source's opening value with a bond.
- **Game Mode**: report a changed value with a bond, or challenge another contributor's update.

Bonds are in USDC and are taken from your on-chain oracle collateral balance. Their sizes are set for each month and bucket, not protocol-wide.

Support capital helps a proposal through review. It does not give the source more weight: every source in a bucket counts equally.

The **Contribute** tab on the Oracle page lists the Petri commands for the current stage's contributions, with the month filled in. The web app does not submit them, and the hosted backend does not prepare them. Petri sends them from your terminal (its mainnet release is coming; see [Availability](https://docs.amoeba.farm/getting-started/02-cli/#availability)): `petri oracle sources propose` and `support`, `petri oracle prints opening` and `petri oracle updates commit` show the transaction for review and sign it with your own wallet. See the [Petri command reference](https://docs.amoeba.farm/petri-cli/06-command-reference/#oracle-writes).

## The operator

The operator holds the protocol's oracle and admin keys, which are separate from the council. On the oracle side these keys can:

- Create oracle months, reward schedules, and product manifests.
- Accept an update after its 24-hour challenge period.
- Rule first on source, opening, and update challenges, or mark a case unresolved so it goes to the council.
- Finalize each contributor's reward entitlement after settlement.

The operator cannot post a settlement price, reject an update nobody challenged, or sign the settlement record. Settlement records need signatures from separately registered settlement signers.

For the launch month, the council approved the source list and its opening values, and the launch recorded them on chain. See [How the oracle works](https://docs.amoeba.farm/oracle/01-how-it-is-structured/#who-set-the-launch-months-sources).

## The council

The oracle council is the same five-seat council that governs program upgrades through the governance controller. For oracle cases:

- Each seat casts one public ballot per round and cannot change it.
- Exactly three matching ballots decide. Fewer, or a split vote, never decide; the case's fallback applies instead.
- If seats rotate during a case, a new round starts with the current seats and the original deadline stays.

The council can only choose between the outcomes each case allows. It cannot enter a new price, change a position's payoff, or edit a frozen source list. The outcomes are listed in [Challenges and disputes](https://docs.amoeba.farm/contributors/03-challenges/#council-resolution).

## Token holders

AMBA staking, sAMBA, the AMBA reward funnel, and token-weighted oracle voting have been removed. There is no staking workflow. No token balance gives a vote, a council seat, or a share of oracle rewards. Oracle bonds and bounties are plain USDC, not staking deposits or staking yield.

## Traders and Earn depositors

A futures position or an Earn share gives no vote over the oracle, program upgrades, pricing or listings. Every position in a market settles on the same oracle result.

For the program and council addresses, see [Deployment](https://docs.amoeba.farm/technical/05-deployment/).
