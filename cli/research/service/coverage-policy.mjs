// Research prioritization over published oracle facts. This module cannot admit
// sources, advance oracle phases, or determine rewards.
import { fail } from './security.mjs';

const hex = value => typeof value === 'string' && /^[0-9a-f]{64}$/u.test(value);
const sourceStatuses = new Set(['Candidate','Frozen','Inactive','Rejected','OpeningPending','Active','Merged','TimedOut']);
const identity = value => value && typeof value.programId === 'string' && typeof value.cluster === 'string' && typeof value.namespace === 'string';
const sameProtocol = (a, b) => identity(a) && identity(b) && ['programId','cluster','namespace'].every(key => a[key] === b[key]);
function timestamp(value) { const n = typeof value === 'string' ? Date.parse(value) : NaN; return Number.isSafeInteger(n) ? n : null; }
function fresh(value, now, maximumAge) {
  const observed = timestamp(value?.observedAt);
  return value?.stale === false && value?.refreshHealthy === true && value?.readReady === true &&
    observed !== null && observed <= now && now - observed <= maximumAge;
}
function host(source) {
  if (source.canonicalLocator?.verified !== true) return null;
  try { const url = new URL(source.canonicalLocator.url); return url.protocol === 'https:' && !url.username && !url.password ? url.hostname.toLowerCase().replace(/^www\./u, '') : null; }
  catch { return null; }
}

export function assessCoverage({ snapshot, catalog }, directives, now) {
  if (!sameProtocol(snapshot?.protocol, catalog?.protocol) || !Array.isArray(snapshot?.state?.months) ||
      !Array.isArray(snapshot.sourceLinks) || !Array.isArray(catalog?.markets)) throw fail('ORACLE_DATA_INVALID', 'The oracle coverage response is incomplete.', 503);
  const expiries = new Map();
  for (const market of catalog.markets) {
    if (!Array.isArray(market.expiries)) throw fail('ORACLE_DATA_INVALID', 'The market catalog is incomplete.', 503);
    for (const expiry of market.expiries) {
      const key = `${market.marketId}:${expiry.expiryId}`;
      if (expiries.has(key)) throw fail('ORACLE_DATA_INVALID', 'The market catalog contains conflicting identities.', 503);
      expiries.set(key, { ...expiry, marketId: market.marketId, onChainAvailable: market.onChainAvailable });
    }
  }
  const facts = new Map();
  for (const fact of snapshot.state.months) {
    const key = `${fact?.marketId}:${fact?.expiryId}`;
    if (!/^[a-z0-9][a-z0-9_-]{0,63}$/u.test(fact?.marketId) || !/^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$/u.test(fact?.expiryId) || facts.has(key)) throw fail('ORACLE_DATA_INVALID', 'The oracle series identities are invalid.', 503);
    facts.set(key, fact);
  }
  const inventories = new Map();
  for (const inventory of snapshot.sourceLinks) {
    const key = `${inventory?.marketId}:${inventory?.expiryId}`;
    if (inventories.has(key)) throw fail('ORACLE_DATA_INVALID', 'The oracle source inventories conflict.', 503);
    inventories.set(key, inventory);
  }
  const assessments = [];
  const add = assessment => { assessments.push(assessment); };
  for (const directive of directives.filter(d => d.enabled)) {
    for (const [key, expiry] of expiries) {
      if (directive.marketIds.length && !directive.marketIds.includes(expiry.marketId)) continue;
      const base = { directiveId: directive.id, directiveRevision: directive.revision, marketId: expiry.marketId, expiryId: expiry.expiryId,
        oracleMonthAddress: facts.get(key)?.oracleMonthAddress ?? null, bucketId: null, bucketLabel: null,
        protocol: snapshot.protocol, assessedAt: now, observedAt: snapshot.freshness?.observedAt ?? null,
        targets: { activeSources: directive.minActiveSources, distinctHosts: directive.minDistinctHosts }, reasons: [], sources: [] };
      const unknown = reason => add({ ...base, status: 'unknown', reason });
      if (!fresh(snapshot.freshness, now, directive.maxObservationAgeMs) || !fresh(catalog.freshness, now, directive.maxObservationAgeMs)) { unknown('Oracle observations are stale or unavailable.'); continue; }
      const settlement = timestamp(expiry.settlementUtc);
      if (settlement === null) { unknown('The exact settlement time is unavailable.'); continue; }
      if (settlement <= now) { add({ ...base, status: 'inactive', reason: 'This series has reached settlement.' }); continue; }
      const fact = facts.get(key);
      if (expiry.onChainAvailable !== true || fact?.onChainAvailable !== true || fact?.oracleMonthAvailable !== true) { unknown('The oracle month is not initialized or its state is unavailable.'); continue; }
      if (['Finalized','Settled'].includes(fact.phase) || ['Settled','Finalized'].includes(fact.settlementStatus)) { add({ ...base, status: 'inactive', reason: 'The oracle series is finalized.' }); continue; }
      const inventory = inventories.get(key), observed = timestamp(inventory?.observedAt);
      if (!inventory || inventory.usage !== 'source-display-only' || inventory.oracleMonthAddress !== fact.oracleMonthAddress || inventory.phase !== fact.phase ||
          !Number.isInteger(inventory.sourceCount) || inventory.sourceCount < 0 ||
          inventory.sourceCount !== fact.sourceCount || !Array.isArray(inventory.sources) || inventory.sources.length !== inventory.sourceCount ||
          observed === null || observed > now || now - observed > directive.maxObservationAgeMs) { unknown('A fresh complete source inventory is unavailable for this exact series.'); continue; }
      const sourceIds = new Set(), buckets = new Map(); let invalid = false;
      for (const source of inventory.sources) {
        if (!hex(source.sourceId) || !hex(source.bucketId) || sourceIds.has(source.sourceId) || !sourceStatuses.has(source.status)) { invalid = true; break; }
        sourceIds.add(source.sourceId);
        const bucket = buckets.get(source.bucketId) || [];
        bucket.push(source); buckets.set(source.bucketId, bucket);
      }
      if (invalid) { unknown('The source inventory has invalid or duplicate identities.'); continue; }
      // A truly empty inventory is a confirmed market-wide research gap; no
      // bucket identifier is synthesized from labels, dates or hashes.
      if (!buckets.size) buckets.set(null, []);
      const validUntil = Math.min(observed + directive.maxObservationAgeMs, timestamp(snapshot.freshness.observedAt) + directive.maxObservationAgeMs,
        timestamp(catalog.freshness.observedAt) + directive.maxObservationAgeMs, settlement);
      for (const [bucketId, sources] of buckets) {
        const active = sources.filter(source => source.status === 'Active');
        const hosts = [...new Set(active.map(host).filter(Boolean))].sort();
        const missingLocators = active.some(source => host(source) === null), reasons = [];
        if (active.length < directive.minActiveSources) reasons.push('insufficient_active_sources');
        if (!missingLocators && hosts.length < directive.minDistinctHosts) reasons.push('insufficient_distinct_hosts');
        const hostsUncertain = missingLocators && hosts.length < directive.minDistinctHosts;
        const status = reasons.length ? 'gap' : hostsUncertain ? 'unknown' : 'covered';
        add({ ...base, bucketId, bucketLabel: sources.find(s => typeof s.bucketLabel === 'string')?.bucketLabel ?? null,
          phase: fact.phase, observedAt: inventory.observedAt, validUntil, status, reasons,
          reason: hostsUncertain ? 'Some active sources lack verified locators; host coverage is unknown.' : null,
          activeSources: active.length, distinctHosts: missingLocators ? null : hosts.length, knownHosts: hosts,
          sources: sources.map(source => ({ sourceId: source.sourceId, status: source.status, url: host(source) ? source.canonicalLocator.url : null })) });
      }
    }
  }
  return { assessedAt: now, protocol: snapshot.protocol, assessments };
}

export function describeGap(assessment) {
  const product = assessment.bucketLabel || (assessment.bucketId ? `product ${assessment.bucketId.slice(0, 10)}` : 'source coverage');
  return { title: `${assessment.marketId.toUpperCase()} · ${product}`,
    brief: [`Research additional public sources for ${assessment.expiryId}${assessment.bucketId ? ` / bucket ${assessment.bucketId}` : '. Establish the governed product group before submitting a candidate'}.`,
      `Observed active sources: ${assessment.activeSources}; target: ${assessment.targets.activeSources}.`,
      `Observed distinct hosts: ${assessment.distinctHosts ?? 'unknown'}; target: ${assessment.targets.distinctHosts}.`,
      'Match the exact governed product, region and price definition. Avoid existing source URLs. Human guidance below refines the research scope.',
      'Completion requires a later fresh oracle inventory meeting the coverage targets. Submitting candidates or finishing a run does not resolve this gap.'].join('\n') };
}
