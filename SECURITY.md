# Security Policy

## Supported versions

Petri does not currently have a supported production release. Maintainers
evaluate reports against the latest source and any `0.1.x` build explicitly
identified as the current pre-production candidate. Development snapshots,
modified clients, unofficial binaries, and older candidates are not supported
releases. This policy is not a warranty, audit statement, bug-bounty promise,
or guaranteed response or remediation time.

## Report a vulnerability privately

Do not open a public issue for a suspected vulnerability, leaked credential,
wallet exposure, or reusable signed transaction. Use the repository's private
security-advisory reporting flow. If that flow is unavailable while the public
repository is being prepared, contact the project maintainer through an
existing private channel.

Include the affected version, operating system, reproduction steps, expected
impact, and whether any credential or transaction material may already be
exposed. Do not attach private keys, seed phrases, bearer tokens, wallet files,
or live reusable signatures.

## Release trust

Install only a versioned release and verify its published SHA-256 checksum.
Windows and macOS release artifacts must be signed through the platform release
process before publication. The workspace updater refuses dirty or divergent
source trees and rebuilds only the release binary, but users should still verify
the resulting version and artifact provenance.

## Wallet and agent safety

Petri can sign and send supported transactions after its deployment-identity
and exact transaction-policy checks pass. Verify `petri config show`, the exact
source commit, the selected network, and the artifact checksum and platform
signature before using a signing wallet.

Agent wallet control is disabled by default. When explicitly enabled, the
connected agent may advance supported transaction stages without Petri
authenticating a separate human approval for each call. A mistaken,
compromised, or malicious agent can cause irreversible loss. Use a dedicated
wallet containing only funds the user is willing to delegate, and never paste a
seed phrase, private key, keypair JSON, or provider credential into an
assistant or report.
