# Releasing Ameba SDK

This checklist is for maintainers. Repository checks never publish, tag or
push. The package is public on npm as `ameba-sdk` (decided 2026-10-16); its
public source is the mirror `amoeba-farm/petri` (`sdk/`), exported from the
monorepo with `ship/export-public-source.sh`.

1. Make sure the vectors in `test/fixtures/futures` were generated from the
   monorepo's current `futures/` (`npm run vectors`). The
   `futuresCommit` field in each fixture records the commit.
2. Run `npm ci` and `npm run check` from a clean checkout.
3. Update `CHANGELOG.md` and the version in `package.json`,
   `package-lock.json`, `Cargo.toml` and the `ameba-sdk` entries of
   `Cargo.lock` and `../cli/Cargo.lock`.
4. Build the tarball with `npm pack --pack-destination dist-pack`, then
   inspect it with `tar -tzf`. It should contain `dist/`, `docs/`, the README,
   the changelog and the license and policy files only.
5. The CLI links the Rust crate by path (`../sdk`). Publishing to npm needs a
   separate, explicit authorization: the user publishes the inspected
   tarball from their own npm account (`npm login`, then
   `npm publish dist-pack/ameba-sdk-<version>.tgz --access public`). npm
   provenance is off: npm attests only packages published from a public
   repository's CI, and the monorepo is private.
