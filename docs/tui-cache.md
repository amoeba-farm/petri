# TUI display cache

Petri uses a process-local, typed cache API, not a disk database or an HTTP
proxy. It stores reusable display results; it does not cache every response.
Closing Petri drops these entries. Private commitment and operation persistence
remain in their existing dedicated modules.

## Ownership

- `src/cache.rs`: generic `DisplayCache<K, V>`, expiry, LRU eviction, protected
  entries, and duplicate active-selection read suppression. No I/O or signing.
- `src/lab/cache.rs`: feature capacities/lifetimes, backend/network/packaged
  release scope, and invalidation of stored results and request generations.
- `src/lab/requests.rs`: fetch launch and validated completion admission.
- `src/lab/help.rs`: help revision refresh and visible-article protection.

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

1. Use a typed read-only result and a bounded policy in `src/lab/cache.rs`.
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
