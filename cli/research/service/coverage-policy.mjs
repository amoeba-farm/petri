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

// Sources that hold a product's place while a month forms (proposed,
// frozen, opening or live), against those that lost it.
const placed = new Set(['Candidate','Frozen','OpeningPending','Active']);
const DAY_MS = 86400000;

const atomic = value => typeof value === 'string' && /^\d+$/u.test(value) ? value : null;
// A chip month (RAMX and NANDX expiring after 2026-11-01): the API lists its
// commodity buckets (`buckets`) and names them (`bucketName`, "DDR5 16Gb").
// Its sources are comparison series and its prints observations of them
// (oracle/governance/commodity_submission_spec.v2.json).
export const chipMonth = fact => (Array.isArray(fact?.buckets) && fact.buckets.length > 0) ||
  (Array.isArray(fact?.skus) && fact.skus.some(sku => typeof sku?.bucketName === 'string' && sku.bucketName));
const named = (...items) => items.map(item => item?.bucketName).find(name => typeof name === 'string' && name) ?? null;
const seriesId = source => typeof source?.series?.seriesId === 'string' ? source.series.seriesId : null;
// USDC atoms (six decimals) as petri takes them: '1000000' is '1', '2500000' is '2.5'.
export function usdc(atoms) {
  if (atomic(atoms) === null) return null;
  const value = BigInt(atoms), whole = value / 1000000n, part = (value % 1000000n).toString().padStart(6, '0').replace(/0+$/u, '');
  return part ? `${whole}.${part}` : `${whole}`;
}

// What a month takes now, by its phase's rules: new sources while it forms
// and placement is open (SourceSubmission, Scramble days 1-4), opening values
// for its frozen sources in Opening, updates to its live sources in Game,
// its reward claims once Settled with its rewards frozen, otherwise nothing
// (Scramble's challenge and freeze days, a settled month not frozen yet).
// Opening follows the chain's phase, not this clock: the oracle takes
// openings until listing on the chain's own clock, which can run behind.
function stage(fact, now) {
  if (fact.phase === 'SourceSubmission') return 'sources';
  if (fact.phase === 'Scramble') {
    const start = timestamp(fact.schedule?.scrambleStartUtc);
    return start !== null && now < start + 4 * DAY_MS ? 'sources' : 'closed';
  }
  if (fact.phase === 'Opening') return 'openings';
  if (fact.phase === 'Settled' && fact.rewards?.phase === 'EntitlementsFinalized') return 'claims';
  return fact.phase === 'Game' ? 'updates' : 'closed';
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
        oracleMonthAddress: facts.get(key)?.oracleMonthAddress ?? null, bucketId: null, bucketLabel: null, bucketName: null,
        chip: chipMonth(facts.get(key)),
        protocol: snapshot.protocol, assessedAt: now, observedAt: snapshot.freshness?.observedAt ?? null,
        targets: { activeSources: directive.minActiveSources, distinctHosts: directive.minDistinctHosts }, reasons: [], sources: [] };
      const unknown = reason => add({ ...base, status: 'unknown', reason });
      if (!fresh(snapshot.freshness, now, directive.maxObservationAgeMs) || !fresh(catalog.freshness, now, directive.maxObservationAgeMs)) { unknown('Oracle observations are stale or unavailable.'); continue; }
      const fact = facts.get(key);
      // Settled with its rewards frozen: one task to claim them.
      if (fact?.oracleMonthAvailable === true && stage(fact, now) === 'claims') {
        add({ ...base, kind: 'claims', phase: fact.phase, status: 'gap', reasons: ['claims'], reason: null,
          validUntil: Math.min(timestamp(snapshot.freshness.observedAt), timestamp(catalog.freshness.observedAt)) + directive.maxObservationAgeMs });
        continue;
      }
      const settlement = timestamp(expiry.settlementUtc);
      if (settlement === null) { unknown('The exact settlement time is unavailable.'); continue; }
      if (settlement <= now) { add({ ...base, status: 'inactive', reason: 'This series has reached settlement.' }); continue; }
      if (expiry.onChainAvailable !== true || fact?.onChainAvailable !== true || fact?.oracleMonthAvailable !== true) { unknown('The oracle month is not initialized or its state is unavailable.'); continue; }
      if (['Finalized','Settled'].includes(fact.phase) || ['Settled','Finalized'].includes(fact.settlementStatus)) { add({ ...base, status: 'inactive', reason: 'The oracle series is finalized.' }); continue; }
      const inventory = inventories.get(key), observed = timestamp(inventory?.observedAt);
      if (!inventory || inventory.usage !== 'source-display-only' || inventory.oracleMonthAddress !== fact.oracleMonthAddress || inventory.phase !== fact.phase ||
          !Number.isInteger(inventory.sourceCount) || inventory.sourceCount < 0 ||
          // While a month takes sources its count is its supported sources and
          // the inventory lists every proposal; from the freeze on they match.
          (stage(fact, now) === 'sources' ? inventory.sourceCount < fact.sourceCount : inventory.sourceCount !== fact.sourceCount) ||
          !Array.isArray(inventory.sources) || inventory.sources.length !== inventory.sourceCount ||
          observed === null || observed > now || now - observed > directive.maxObservationAgeMs) { unknown('A fresh complete source inventory is unavailable for this exact series.'); continue; }
      const sourceIds = new Set(), buckets = new Map(); let invalid = false;
      for (const source of inventory.sources) {
        if (!hex(source.sourceId) || !hex(source.bucketId) || sourceIds.has(source.sourceId) || !sourceStatuses.has(source.status)) { invalid = true; break; }
        sourceIds.add(source.sourceId);
        const bucket = buckets.get(source.bucketId) || [];
        bucket.push(source); buckets.set(source.bucketId, bucket);
      }
      if (invalid) { unknown('The source inventory has invalid or duplicate identities.'); continue; }
      const validUntil = Math.min(observed + directive.maxObservationAgeMs, timestamp(snapshot.freshness.observedAt) + directive.maxObservationAgeMs,
        timestamp(catalog.freshness.observedAt) + directive.maxObservationAgeMs, settlement);
      const kind = stage(fact, now);
      if (kind === 'closed') {
        add({ ...base, status: 'inactive', phase: fact.phase, reason: fact.phase === 'Scramble' ? 'The month is past placement: its sources are set until Opening.'
          : 'The month takes no research in its phase.' });
        continue;
      }
      if (kind === 'openings') {
        // Opening: each frozen source still without an opening value takes
        // one (bonded with its product's opening bond, archived after the
        // Opening's start); an opened or pending one takes nothing.
        const skus = new Map((Array.isArray(fact.skus) ? fact.skus : []).filter(sku => hex(sku?.bucketId)).map(sku => [sku.bucketId, sku]));
        // The oracle's calendar: Scramble's seven days, then the Opening's one
        // until listing. Archive captures count from the Opening's start.
        const scramble = timestamp(fact.schedule?.scrambleStartUtc);
        const listing = timestamp(fact.schedule?.listingUtc) ?? (scramble !== null ? scramble + 8 * DAY_MS : null);
        const start = listing !== null ? listing - DAY_MS : null;
        for (const source of inventory.sources.filter(source => source.status === 'Frozen' && source.openingSubmitted !== true)) {
          const sku = skus.get(source.bucketId);
          add({ ...base, kind: 'opening', bucketId: source.bucketId,
            bucketLabel: typeof source.bucketLabel === 'string' ? source.bucketLabel : typeof sku?.bucketLabel === 'string' ? sku.bucketLabel : null,
            bucketName: named(source, sku), seriesId: seriesId(source),
            sourceId: source.sourceId, sourceAddress: typeof source.sourceAddress === 'string' ? source.sourceAddress : null,
            url: host(source) ? source.canonicalLocator.url : null, phase: fact.phase, observedAt: inventory.observedAt, validUntil,
            status: 'gap', reasons: ['opening_value'], reason: null,
            openingBondAtomic: atomic(sku?.openingBondAtomic), openingBudgetAtomic: atomic(sku?.openingBudgetAtomic),
            openingStartUtc: start !== null ? new Date(start).toISOString() : null, listingUtc: listing !== null ? new Date(listing).toISOString() : null,
            sources: [] });
        }
        continue;
      }
      if (kind === 'updates') {
        // Game: each live source takes updates (a staked, archived new value).
        const rewards = fact.rewards && typeof fact.rewards.remainingAtomic === 'string' ? fact.rewards.remainingAtomic : '0';
        for (const source of inventory.sources.filter(source => source.status === 'Active')) {
          add({ ...base, kind: 'update', bucketId: source.bucketId, bucketLabel: typeof source.bucketLabel === 'string' ? source.bucketLabel : null,
            bucketName: named(source), seriesId: seriesId(source),
            sourceId: source.sourceId, sourceAddress: typeof source.sourceAddress === 'string' ? source.sourceAddress : null,
            url: host(source) ? source.canonicalLocator.url : null, phase: fact.phase, observedAt: inventory.observedAt, validUntil,
            status: 'gap', reasons: ['game_update'], reason: null, rewardsRemainingAtomic: rewards, sources: [] });
        }
        continue;
      }
      // Taking sources: every product the month lists (its SKU pools), and
      // any bucket a source is in.
      const labels = new Map(), names = new Map();
      for (const sku of Array.isArray(fact.skus) ? fact.skus : []) {
        if (!hex(sku?.bucketId)) continue;
        if (!buckets.has(sku.bucketId)) buckets.set(sku.bucketId, []);
        if (typeof sku.bucketLabel === 'string') labels.set(sku.bucketId, sku.bucketLabel);
        if (named(sku)) names.set(sku.bucketId, named(sku));
      }
      // A truly empty inventory is a confirmed market-wide research gap; no
      // bucket identifier is synthesized from labels, dates or hashes.
      if (!buckets.size) buckets.set(null, []);
      for (const [bucketId, sources] of buckets) {
        const active = sources.filter(source => placed.has(source.status));
        const hosts = [...new Set(active.map(host).filter(Boolean))].sort();
        const missingLocators = active.some(source => host(source) === null), reasons = [];
        if (active.length < directive.minActiveSources) reasons.push('insufficient_active_sources');
        if (!missingLocators && hosts.length < directive.minDistinctHosts) reasons.push('insufficient_distinct_hosts');
        const hostsUncertain = missingLocators && hosts.length < directive.minDistinctHosts;
        const status = reasons.length ? 'gap' : hostsUncertain ? 'unknown' : 'covered';
        add({ ...base, kind: 'source', bucketId, bucketLabel: sources.find(s => typeof s.bucketLabel === 'string')?.bucketLabel ?? labels.get(bucketId) ?? null,
          bucketName: named(...sources) ?? names.get(bucketId) ?? null,
          phase: fact.phase, observedAt: inventory.observedAt, validUntil, status, reasons,
          reason: hostsUncertain ? 'Some active sources lack verified locators; host coverage is unknown.' : null,
          activeSources: active.length, distinctHosts: missingLocators ? null : hosts.length, knownHosts: hosts,
          sources: sources.map(source => ({ sourceId: source.sourceId, status: source.status, url: host(source) ? source.canonicalLocator.url : null })) });
      }
    }
  }
  // Every series the catalog lists, assessed or not (a directive may leave
  // some out): what the workspace's work may still be about.
  const series = [...expiries.values()].map(({ marketId, expiryId }) => ({ marketId, expiryId }));
  return { assessedAt: now, protocol: snapshot.protocol, series, assessments };
}

export function describeGap(assessment) {
  if (assessment.kind === 'claims') {
    return { title: `${assessment.marketId.toUpperCase()} · ${assessment.expiryId} · claims`,
      brief: `${assessment.expiryId} settled and its rewards are frozen: claim what the owner's wallet earned.` };
  }
  const product = assessment.bucketName || assessment.bucketLabel || (assessment.bucketId ? `product ${assessment.bucketId.slice(0, 10)}` : 'source coverage');
  // On a chip month a source is a comparison series and a print one observation of it.
  const chip = assessment.chip === true;
  if (assessment.kind === 'opening') {
    const bond = usdc(assessment.openingBondAtomic) === null ? 'its product\'s opening bond' : `its product's opening bond (${usdc(assessment.openingBondAtomic)} USDC)`;
    return { title: `${assessment.marketId.toUpperCase()} · ${product} · opening`,
      brief: [`Give this frozen source of ${assessment.expiryId} its opening value: ${assessment.url ?? `source ${assessment.sourceId}`} (source address ${assessment.sourceAddress ?? 'unknown'}, product ${product}).`,
        `Read the value the page publishes for this exact product and field. Archive the page in the Wayback Machine; the capture time must be at or after the Opening's start${assessment.openingStartUtc ? ` (${assessment.openingStartUtc})` : ''} and not after the chain's clock.`,
        `Submit it with petri, staking ${bond}; after its challenge window (300 slots) accept it before listing${assessment.listingUtc ? ` (${assessment.listingUtc})` : ''}. A wrong one can be challenged and loses its bond; a source with no opening value by listing sits out the month.`,
        ...(chip ? [`This source is comparison series ${assessment.seriesId ?? '(see its definition)'}: its opening value is one observation of it, sent with petri oracle observe (the observation's details go with it).`] : [])].join('\n') };
  }
  if (assessment.kind === 'update') {
    return { title: `${assessment.marketId.toUpperCase()} · ${product} · update`,
      brief: [`Update this live source of ${assessment.expiryId} with its current value: ${assessment.url ?? `source ${assessment.sourceId}`} (source address ${assessment.sourceAddress ?? 'unknown'}, product ${product}).`,
        'Read the value the page publishes for this exact product and field. Archive the page in the Wayback Machine first; the archived time must be after the month\'s listing.',
        'Submit it as a Game update with petri: commit it with its stake, then reveal it with the archived URL. An update that does not change the value earns nothing; a wrong one can be challenged and loses its stake.',
        ...(chip ? [`This source is comparison series ${assessment.seriesId ?? '(see its definition)'}: each update is one observation of it, sent with petri oracle observe.`] : [])].join('\n') };
  }
  return { title: `${assessment.marketId.toUpperCase()} · ${product}`,
    brief: [`Research additional public sources for ${assessment.expiryId}${assessment.bucketId ? ` / bucket ${assessment.bucketId}` : '. Establish the governed product group before submitting a candidate'}.`,
      'A page behind the owner\'s sign-in (a subscription, a distributor portal) counts too, proven with petri evidence prove after the owner signs in themselves.',
      `Observed active sources: ${assessment.activeSources}; target: ${assessment.targets.activeSources}.`,
      `Observed distinct hosts: ${assessment.distinctHosts ?? 'unknown'}; target: ${assessment.targets.distinctHosts}.`,
      chip ? `This month is on commodity chip buckets: each source is a comparison series of ${product} (a published contract assessment, or a direct contract price), defined field by field as the commodity spec requires. Never guess a field: leave what the source doesn't say unset.`
        : 'Match the exact governed product, region and price definition. Avoid existing source URLs. Human guidance below refines the research scope.',
      'Completion requires a later fresh oracle inventory meeting the coverage targets. Submitting candidates or finishing a run does not resolve this gap.'].join('\n') };
}
