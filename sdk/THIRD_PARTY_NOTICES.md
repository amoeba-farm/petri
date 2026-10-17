# Third-party notices

## TypeScript runtime dependencies

- `@solana/web3.js@1.98.4` is licensed under the MIT License. Its transitive
  packages keep their own package metadata and license files. The tree
  includes `rpc-websockets` (`LGPL-3.0-only`) and `text-encoding-utf-8`
  (Unlicense). The futures module uses only `PublicKey`,
  `TransactionInstruction` and the RPC filter types.
- `buffer@6.0.3` is licensed under the MIT License.

To inspect the installed production tree, run `npm ls --omit=dev` and
`npm audit --omit=dev` from a clean checkout.

## Rust dependencies

- `ameba-futures-client` and the crates it links from this monorepo
  (`ameba-futures`, `ameba-futures-math`, `ameba-oracle-interface`,
  `ameba-governance-interface`) are licensed under Apache-2.0 and linked by
  path.
- `solana-program@2.3.0` and its dependencies keep their own licenses. To
  list them, run `cargo tree` or `cargo metadata`.
