# TUI display cache

Petri uses a process-local, typed cache API, not a disk database or an HTTP
proxy. It stores reusable display results; it does not cache every response.
Closing Petri drops these entries. Private commitment and operation persistence
remain in their existing dedicated modules.

## Ownership

- `src/cache.rs`: generic `DisplayCache<K, V>`, expiry, LRU eviction, protected
  entries, and duplicate active-selection read suppression. No I/O or signing.
- `src/lab/cache.rs`: the app-owned `TuiCache` internal API, feature capacities/
  lifetimes, backend/network/packaged release scope, and invalidation of stored
  results and request generations. Storage is private to this module.
- `src/lab/requests.rs`: fetch launch and validated completion admission.
- `src/lab/help.rs`: chooses the visible article and updates preview state;
  `TuiCache::store_help_page` protects that article, advances the content revision
  and invalidates the rendered article together.

All TUI feature and rendering modules reach the same instance through
`app.cache`: `details()`, `charts()`, `settlements()` and `help_pages()` expose
typed read access. The corresponding market `_mut()` accessors share the same
TTL/LRU and pending-fetch engine; completion admission stays in the reducers.
Help writes use `store_help_page`, not an independently mutable page store.
`help_render()` and `store_help_render()` also keep the existing single rendered
article in this API, with the same width, color, motion and revision keys.
Rendering only borrows data; it does not acquire transport or signing effects.

## Policies

| Data | Entry limit | Reuse lifetime | Identity within the read scope |
| --- | ---: | --- | --- |
| Market details | 32 | 10 seconds | Market |
| Contract charts | 32 | 30 seconds | Market, exact contract, range, point limit |
| Settlement evidence | 24 | 15 seconds | Market and exact contract |
| Help pages | 12 | Existing help revision/refresh lifecycle | Page ID |

Reads update eviction recency, not freshness. Expired entries are misses and
are reclaimed on admission or eviction. Failed reads, empty chart histories,
and partial settlement bundles are not admitted as reusable successes. Existing
last-view UI state is distinct from a fresh cache hit.

Forced refresh bypasses and removes the selected entry. Full market refresh,
scope changes, and integrated trade/writer/action submission completion paths
invalidate market caches. The same invalidation advances request generations:
older background results cannot refill cleared storage. Repeated non-forced
reads of the active key share the pending job; explicit refresh supersedes it.

## Adding another cached view

1. Add a typed read-only store, bounded policy and accessors to `TuiCache` in
   `src/lab/cache.rs`. Do not add another cache field to `LabApp` or a global
   response store.
2. Include every selector in its key. Wallet-dependent displays must include
   the owner; never key private data solely by market or route.
3. Check scope before reading, call `begin_fetch` on a miss, and admit only
   validated results from the current request generation. Always finish the
   current pending read on success or error.
4. Connect mutations and scope changes to invalidation. Do not reuse cached
   availability as permission to prepare, sign, or submit.

Wallet balances, oracle live state, action masks, executable quotes, signed
packets, approvals and prepared plans are not moved into this engine. Some
screens retain their last view independently; that is not a claim of freshness.
Trading stays available through its existing live identity, admission and
execution checks. Cache TTL never substitutes for backend freshness evidence.

Immutable animation/projection memoization remains with its renderer, not in the
market response cache. It has no backend identity, freshness or approval role.
