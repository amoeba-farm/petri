# Reviewed dependency compatibility patches

These source copies preserve Petri's existing public dependency APIs while the
upstream release lines still require vulnerable transitive generations. They
are selected through `Cargo.toml` `[patch.crates-io]` entries and are not Petri
product forks.

All four upstream packages declare Apache-2.0. Keep their package metadata,
copyright notices, and attribution intact. Remove each local patch as soon as a
compatible upstream release contains the same remediation.

| Package | Upstream crate checksum | Local compatibility change |
| --- | --- | --- |
| `light-prover-client 8.0.0` | `1a9b434c4819c575c53b439a04cef23e14a7359728d9438a10e6de3641a6d3ba` | Uses Reqwest 0.12 with Rustls and no default features, removing the vulnerable Reqwest 0.11 / Rustls 0.21 / WebPKI 0.101 line. Its public API exposes no Reqwest types. |
| `solana-signature 2.3.0` | `64c8ec8e657aecfc187522fc67495142c12f35e55ddeca8698edbb738b8dbd8c` | Keeps the Solana 2.3 signature type and verification API while using `ed25519-dalek 2.2`. |
| `solana-keypair 2.2.3` | `bd3f04aa1a05c535e93e121a95f66e7dcccf57e007282e8255535d24bf1e98bb` | Keeps the Solana 2.2 keypair/file/signer API while storing an `ed25519-dalek 2.2` `SigningKey`; BIP-32 moves to the compatible 0.3 line. |
| `solana-ed25519-program 2.2.3` | `a1feafa1691ea3ae588f99056f4bdd1293212c7ece28243d7da257c443e84753` | Keeps the existing instruction wire format and verifier API while using `ed25519-dalek 2.2`. |

The corresponding RustSec findings are `RUSTSEC-2024-0344`,
`RUSTSEC-2022-0093`, `RUSTSEC-2026-0098`, `RUSTSEC-2026-0099`, and
`RUSTSEC-2026-0104`. Patch-only lock updates separately address
`RUSTSEC-2026-0204`, `RUSTSEC-2026-0185`, and `RUSTSEC-2026-0049`.

Maintenance requirements:

- Preserve the RFC 8032 signature semantics and the existing Solana Ed25519
  instruction wire layout when changing these copies.
- Review the locked dependency graph against current RustSec advisories before
  release.
- Keep the locked public Petri build and MCP wrapper checks green, and remove a
  local patch once a compatible upstream release carries the same fix.
