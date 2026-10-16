// Internal bootstrap: stdout is a private pipe owned by the Rust parent.
import { loadOwner, stateDirectory } from '../service/security.mjs';
import { ensureService } from '../service/lifecycle.mjs';
try {
  const config = loadOwner(stateDirectory(), process.env.PETRI_RESEARCH_UPSTREAM,
    process.env.PETRI_RESEARCH_PORT ? Number(process.env.PETRI_RESEARCH_PORT) : undefined,
    process.env.PETRI_RESEARCH_ORACLE_URL);
  await ensureService(config);
  process.stdout.write(JSON.stringify({ baseUrl: config.baseUrl, ownerToken: config.ownerToken }));
} catch (error) {
  process.stderr.write(`${String(error.message).replace(/[\x00-\x1f\x7f-\x9f\u202a-\u202e\u2066-\u2069]/gu, ' ')}\n`);
  process.exitCode = 1;
}
