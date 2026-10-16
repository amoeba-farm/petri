import { randomUUID } from 'node:crypto';
import { digest, fail, object, text } from './security.mjs';
import { describeGap } from './coverage-policy.mjs';

const view = row => ({ id: row.id, marketIds: JSON.parse(row.markets_json), minActiveSources: row.min_sources,
  minDistinctHosts: row.min_hosts, maxObservationAgeMs: row.max_age_ms, enabled: row.enabled === 1, revision: row.revision,
  createdAt: row.created_at, updatedAt: row.updated_at });
export function createDirectiveStore(db, clock = Date.now) {
  function save(body) {
    object(body, ['id','marketIds','minActiveSources','minDistinctHosts','maxObservationAgeMs','enabled'], ['marketIds','minActiveSources','minDistinctHosts']);
    if (!Array.isArray(body.marketIds) || body.marketIds.some(id => typeof id !== 'string' || !/^[a-z0-9][a-z0-9_-]{0,63}$/u.test(id))) throw fail('INVALID_DIRECTIVE', 'Choose canonical market IDs, or leave the list empty for all markets.');
    for (const key of ['minActiveSources','minDistinctHosts']) if (!Number.isInteger(body[key]) || body[key] < 1 || body[key] > 64) throw fail('INVALID_DIRECTIVE', 'Coverage targets must be whole numbers from 1 to 64.');
    if (body.minDistinctHosts > body.minActiveSources) throw fail('INVALID_DIRECTIVE', 'The host target cannot exceed the source target.');
    const age = body.maxObservationAgeMs ?? 600000;
    if (!Number.isInteger(age) || age < 1 || body.enabled !== undefined && typeof body.enabled !== 'boolean') throw fail('INVALID_DIRECTIVE', 'Invalid directive freshness or enabled setting.');
    const old = body.id ? db.get('SELECT * FROM directives WHERE id=?', text(body.id, 36, 'directive ID')) : null;
    if (body.id && !old) throw fail('DIRECTIVE_NOT_FOUND', 'Directive not found.', 404);
    if (!old && db.get('SELECT count(*) AS n FROM directives').n >= 32) throw fail('DIRECTIVE_LIMIT', 'The workspace supports at most 32 standing directives.', 429);
    const id = old?.id || randomUUID(), now = clock();
    db.atomic(() => {
      db.run(`INSERT INTO directives VALUES (?,?,?,?,?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET markets_json=excluded.markets_json,
        min_sources=excluded.min_sources,min_hosts=excluded.min_hosts,max_age_ms=excluded.max_age_ms,enabled=excluded.enabled,
        revision=excluded.revision,updated_at=excluded.updated_at`, id, JSON.stringify([...new Set(body.marketIds)].sort()), body.minActiveSources,
        body.minDistinctHosts, age, (body.enabled ?? (old ? old.enabled === 1 : true)) ? 1 : 0, (old?.revision || 0) + 1, old?.created_at || now, now);
      // Editing targets withdraws old offers until they have been reassessed.
      for (const row of db.all('SELECT id,objective_json FROM work WHERE objective_json IS NOT NULL')) if (JSON.parse(row.objective_json).directiveId === id) db.run("UPDATE work SET availability='unknown',valid_until=NULL WHERE id=?", row.id);
    });
    return view(db.get('SELECT * FROM directives WHERE id=?', id));
  }
  function record(result) {
    db.atomic(() => {
      db.run("UPDATE work SET availability='unknown',valid_until=NULL WHERE discovery_key IS NOT NULL");
      for (const assessment of result.assessments) {
        const key = digest(JSON.stringify([assessment.protocol.programId, assessment.protocol.cluster, assessment.protocol.namespace,
          assessment.directiveId, assessment.marketId, assessment.expiryId, assessment.oracleMonthAddress, assessment.bucketId]));
        const previous = db.get('SELECT id FROM work WHERE discovery_key=?', key);
        if (assessment.status !== 'gap' && !previous) continue;
        if (!previous && db.get('SELECT count(*) AS n FROM work').n >= 5000) throw fail('WORK_LIMIT', 'Workspace research history capacity reached.', 429);
        const { title, brief } = describeGap(assessment);
        const scope = { marketId: assessment.marketId, expiryId: assessment.expiryId, ...(assessment.bucketId ? { bucketId: assessment.bucketId } : {}) };
        db.run(`INSERT INTO work(id,title,brief,created_at,scope_json,objective_json,discovery_key,availability,valid_until)
          VALUES (?,?,?,?,?,?,?,?,?) ON CONFLICT(discovery_key) WHERE discovery_key IS NOT NULL DO UPDATE SET title=excluded.title,
          brief=excluded.brief,objective_json=excluded.objective_json,availability=excluded.availability,valid_until=excluded.valid_until`,
          previous?.id || randomUUID(), title, brief, clock(), JSON.stringify(scope), JSON.stringify(assessment), key,
          assessment.status === 'gap' ? 'available' : assessment.status === 'covered' ? 'resolved' : assessment.status, assessment.validUntil ?? null);
      }
      db.run("INSERT INTO settings VALUES ('discovery',?) ON CONFLICT(key) DO UPDATE SET value=excluded.value", JSON.stringify({ ...result, error: null }));
    });
  }
  return { save, record,
    list: () => db.all('SELECT * FROM directives ORDER BY created_at,id').map(view),
    seed() { if (!db.get("SELECT value FROM settings WHERE key='directives_initialized'")) {
      save({ marketIds: [], minActiveSources: 3, minDistinctHosts: 2 });
      db.run('UPDATE work SET archived_at=coalesce(archived_at,?) WHERE discovery_key IS NULL', clock());
      db.run("INSERT INTO settings VALUES ('directives_initialized','true')");
    } },
    fail() { db.atomic(() => {
      db.run("UPDATE work SET availability='unknown',valid_until=NULL WHERE discovery_key IS NOT NULL");
      const old = this.status();
      db.run("INSERT INTO settings VALUES ('discovery',?) ON CONFLICT(key) DO UPDATE SET value=excluded.value", JSON.stringify({ ...old, error: 'Oracle coverage could not be refreshed. New work is withheld until a fresh complete assessment is available.' }));
    }); },
    status() { const row = db.get("SELECT value FROM settings WHERE key='discovery'"); return row ? JSON.parse(row.value) : { assessedAt: null, assessments: [], error: null }; },
    refine(workId, instruction) {
      const row = db.get('SELECT * FROM work WHERE id=?', text(workId, 36, 'work ID'));
      if (!row?.objective_json) throw fail('WORK_NOT_FOUND', 'Choose a discovered coverage gap.', 404);
      if (typeof instruction !== 'string') throw fail('INVALID_GUIDANCE', 'Provide guidance text.');
      const value = instruction.trim() ? text(instruction, 8000, 'research guidance') : '';
      const old = row.guidance_json ? JSON.parse(row.guidance_json) : null;
      const guidance = { instruction: value, authorType: 'human', revision: (old?.revision || 0) + 1, updatedAt: clock() };
      db.run('UPDATE work SET guidance_json=? WHERE id=?', JSON.stringify(guidance), workId); return guidance;
    },
  };
}
