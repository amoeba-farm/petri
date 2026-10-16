export const coverageNow = Date.UTC(2026, 9, 1, 20);
export function coverageFixture(now = coverageNow, count = 1) {
  const protocol = { programId: 'canonical-program', cluster: 'testnet', namespace: 'test-namespace' };
  const freshness = { observedAt: new Date(now).toISOString(), stale: false, refreshHealthy: true, readReady: true };
  const fact = { marketId: 'ramx', expiryId: 'RAMX-202610', oracleMonthAddress: 'canonical-month', phase: 'Game', sourceCount: count, onChainAvailable: true, oracleMonthAvailable: true };
  return {
    snapshot: { protocol, freshness, state: { months: [fact] }, sourceLinks: [{ ...fact, usage: 'source-display-only', observedAt: freshness.observedAt,
      sources: Array.from({ length: count }, (_, i) => ({ sourceId: i.toString(16).padStart(64, '0'), bucketId: '21'.repeat(32), bucketLabel: 'Example governed product', status: 'Active', canonicalLocator: { verified: true, url: `https://source-${i}.example/product` } })) }] },
    catalog: { protocol, freshness, markets: [{ marketId: 'ramx', onChainAvailable: true, expiries: [{ expiryId: fact.expiryId, settlementUtc: '2026-11-01T00:00:00.000Z' }] }] },
  };
}
