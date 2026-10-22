import { usdc } from '../service/coverage-policy.mjs';

// This command as the agent's shell runs it again (PowerShell on Windows).
export function self(words, flags = {}) {
  const quote = value => process.platform === 'win32' ? `'${value.replace(/'/gu, "''")}'` : `'${value.replace(/'/gu, `'"'"'`)}'`;
  const options = [['--agent', flags.agent || flags.name || 'researcher'], ...(flags['profile-dir'] ? [['--profile-dir', flags['profile-dir']]] : [])];
  return `${process.platform === 'win32' ? '& ' : ''}${[process.execPath, process.argv[1]].map(quote).join(' ')} ${words} ${options.map(([key, value]) => `${key} ${quote(value)}`).join(' ')}`;
}

// A page behind a sign-in (a subscription, a distributor portal): the owner
// signs in to a browser the agent drives (browser use), and the agent proves
// the JSON the page loads with that session (`petri evidence`,
// evidence/README.md). Credentials stay in the browser: petri never sees them.
const MANIFEST = '{"format":"amoeba-extraction-manifest-v1","name":"<source>: <product>","evidenceType":"catalog_price","origin":"<host>","path":"<path, no query>","content":"json","records":"/items","match":{"/sku":"<product id>"},"fields":{"price":"/unit_price","currency":"/currency","product":"/sku"}}';
function signedIn(petri, { page, json, find, submit }) {
  return [
    `- Open it with browser use: ${petri} evidence browser ${page}. It prints a DevTools address (http://127.0.0.1:<port>); your own browser tool can attach to that window (chrome-devtools-mcp --browserUrl, Playwright connectOverCDP).`,
    '- Ask the owner to sign in there themselves. Never ask for, type or store their password.',
    find,
    `- Save that response to a file and write an extraction manifest for it, e.g. ${MANIFEST}`,
    `- Test it: ${petri} evidence check-manifest <manifest file> --body <saved response>`,
    `- Prove it in the signed-in browser: ${petri} evidence prove ${json} --manifest <manifest file> --browser --out <bundle file> --json`,
    '- Show the owner the preview it prints (what is disclosed) and go on only if they agree.',
    submit,
  ];
}
// An opening or update of a frozen source proves the source's own URL: the
// on-chain evidence record names the source's registered locator.
const sourceProof = (petri, url, command) => [
  'If the page needs a sign-in, it has no public archive: prove it instead of step 2.',
  ...signedIn(petri, { page: url, json: url,
    find: `- The proof is of the source's own URL, ${url}: its whole JSON response is disclosed. Read the same row and price field the source was found with.`,
    submit: command }),
];

// What the agent does with a research run: the exact commands, in order,
// by what the month takes now (sources while it forms, opening values in
// Opening, updates in Game, reward claims once its rewards are frozen).
// `petri research work next` prints it and `petri_work_next` returns it.
export function assignmentSteps(result, flags = {}) {
  if (result.status !== 'assigned') return { ...result, nextStep: `Nothing needs research right now. In a few minutes, run: ${self('work next', flags)}` };
  const run = result.run.id, assignment = result.assignment, objective = assignment.objective || {};
  const directive = assignment.guidance?.instruction, petri = result.petri || 'petri';
  const month = objective.expiryId || assignment.candidateScope?.expiryId;
  const owner = directive ? `The owner's directive for it comes first: ${directive}` : 'The owner has given no directive for it.';
  // The owner's live feed in the TUI: one short line per step, as it happens.
  const say = self(`work report ${run}`, flags), wrote = { opening: 'opening value submitted', update: 'update committed' }[objective.kind] ?? 'sponsored 30d7e0f9…';
  const report = ['Report as you go, one short line each:',
    `- each search: ${say} --kind search --message "<query>"`,
    `- each page you open: ${say} --kind read --source <page URL> --message "<page title>" (a signed-in page too)`,
    `- each value you read: ${say} --kind value --source <page URL> --message "<value> <unit>, <product>"`,
    `- each petri write: ${say} --kind submit --message "<what it did, e.g. ${wrote}>"`,
    ...(objective.kind === 'opening' || objective.kind === 'update' ? [] : [`- each price page worth submitting: ${say} --kind source_found --source <page URL> --message "<product, price>"`]),
    `- anything else: ${say} --message "<text>" (--kind blocked if stuck)`].join('\n');
  const finish = `When you are done with it, run: ${self(`work finish ${run}`, flags)} --summary "<what you did>"`;
  const next = `Then run ${self('work next', flags)} again for the next one. Keep going until it says nothing needs research.`;
  // A chip month (RAMX and NANDX expiring after 2026-11-01): its sources are
  // comparison series and its prints observations of them, sent through
  // petri's series and observe commands (they check the commodity spec first).
  if (objective.chip === true && ['opening', 'update', 'source'].includes(objective.kind)) {
    return { ...result, directive: directive || null, nextStep: chipSteps(result, flags, { report, finish, next, owner, month, petri }) };
  }
  if (objective.kind === 'opening') {
    // The oracle's opening rules: the stake is exactly the product's opening
    // bond; the archive is of the source's own URL, captured from the
    // Opening's start up to the chain's clock; the claim is accepted 300
    // slots after it lands, and only before listing.
    const source = objective.sourceAddress ?? objective.sourceId ?? '<source address>', url = objective.url ?? '<page URL>';
    const bond = usdc(objective.openingBondAtomic), start = objective.openingStartUtc, listing = objective.listingUtc;
    const until = listing ? ` (${listing})` : '';
    return { ...result, directive: directive || null, nextStep: [
      `Give this frozen source its opening value now: ${assignment.title} (${month}). Its page: ${objective.url ?? 'see "assignment"'}.`,
      owner,
      `The month is in Opening until listing${until}, on the chain's clock: each frozen source needs an opening value, the price its page shows now, where the source starts in Game. Anyone submits it, bonded from their petri wallet with the product's opening bond; a source with none by listing sits out the month.`,
      `1. Open the page and read the value it publishes for this exact product (${objective.bucketLabel ?? 'see the brief'}), in the page's own units.`,
      `2. Archive the page: open https://web.archive.org/save/${url} and note the 14-digit UTC capture time it gives. The archive URL is https://web.archive.org/web/<capture time>/${url}, with the page URL exactly as written here. The capture time must be at or after the Opening's start${start ? ` (${start}, ${start.replace(/\D/gu, '').slice(0, 14)})` : ''} and not after the chain's clock (${petri} status shows it as Clock). If the chain's clock is behind the capture, use an earlier capture of the page from that range (its Wayback calendar lists them) or wait until the chain's clock passes it.`,
      ...sourceProof(petri, url, `- In step 3, give --evidence <bundle file> in place of --archive-url. If ${petri} oracle prints opening --help does not list --evidence, report blocked.`),
      `3. Submit it with the owner's wallet: ${petri} oracle prints opening ${month} --source ${source} --value <value> --archive-url <archive URL> --stake ${bond ?? '<opening bond>'} --yes (${bond ? 'the stake must be exactly this product\'s opening bond' : 'the stake must be exactly the product\'s opening bond: petri names it if you give another'}). Submit it some minutes before listing: it can only be accepted 300 slots after it lands. The bond comes from the owner's oracle collateral; if petri says it is short, deposit the bond first: ${petri} oracle collateral deposit ${bond ?? '<opening bond>'} --yes`,
      `4. Anyone can challenge it for 300 slots (about two minutes). Then accept it: ${petri} oracle prints finalize ${month} --source ${source} --yes. If petri says it can't be accepted yet, wait a minute and run it again. It must be accepted before listing${until}. If it was challenged, the oracle's council decides it: report that and move on.`,
      `5. Return the bond to the owner's collateral: ${petri} oracle stakes settle ${month} --kind opening-claim --source ${source} --yes`,
      `At listing the month enters Game, once every frozen source has its opening value or is set aside (anyone sets an unopened one aside: ${petri} oracle prints expire ${month} --source <source address> --yes) and the oracle has weighted its sources. Then anyone can start Game: ${petri} oracle prints finalize-phase ${month} --yes.`,
      report, finish, next,
    ].join('\n') };
  }
  if (objective.kind === 'claims') {
    return { ...result, directive: directive || null, nextStep: [
      `Claim the owner's rewards of ${month}. They go to the owner's oracle collateral.`,
      owner,
      `1. List them: ${petri} oracle rewards ${month}`,
      `2. If any are to claim: ${petri} oracle rewards claim ${month} --yes`,
      `3. For each sponsored source to claim: ${petri} oracle sponsorships claim <source> --yes`,
      report, finish, next,
    ].join('\n') };
  }
  if (objective.kind === 'update') {
    const url = objective.url ?? '<page URL>';
    return { ...result, directive: directive || null, nextStep: [
      `Update this live source now: ${assignment.title} (${month}). Its page: ${objective.url ?? 'see "assignment"'}.`,
      owner,
      'The month is in Game: its sources are set, and each update is a staked new value from the owner\'s petri wallet.',
      `1. Open the page and read the value it publishes for this exact product (${objective.bucketLabel ?? 'see the brief'}).`,
      `2. Archive the page: open https://web.archive.org/save/${url} and keep the snapshot URL it gives (https://web.archive.org/web/<14-digit time>/...).`,
      ...sourceProof(petri, url, '- In step 3, give --evidence <bundle file> in place of --archive-url <snapshot URL>.'),
      `3. Commit the update: ${petri} oracle updates commit ${month} --source ${objective.sourceAddress ?? '<source address>'} --value <value> --archive-url <snapshot URL> --stake 1 --yes (if petri names another stake for this product, use that).`,
      `4. Reveal it within a minute, with the claim address commit printed: ${petri} oracle updates reveal <CLAIM> --yes`,
      'Skip the update if the value has not changed: an unchanged value earns nothing.',
      report, finish, next,
    ].join('\n') };
  }
  const submit = self(`work submit ${run}`, flags);
  return { ...result, directive: directive || null, nextStep: [
    `Research this now: ${assignment.title}. Its brief and exact scope (candidateScope) are in "assignment". The month takes new sources now.`,
    owner,
    report,
    `Submit each page you can verify: write its candidate JSON (marketId, expiryId, bucketId from candidateScope; canonicalUrl, sourceType, productDefinition, priceField, region, currency, evidence) to a file, then run: ${submit} --file <file>`,
    'A page behind a sign-in (a subscription, a distributor portal) counts too, with a web proof:',
    ...signedIn(petri, { page: '<page URL>', json: '<JSON URL>',
      find: '- Find the JSON the page loads with the price (the browser\'s network panel). Its whole response is disclosed: pick one with nothing in it the owner would not share.',
      submit: `- Submit it with the bundle: ${submit} --file <file> --evidence-bundle <bundle file>, with the JSON URL as canonicalUrl. Only a catalog_price or published_benchmark can be a source; a quotation or transaction_record cannot.` }),
    `A submitted find goes on the oracle once a sponsor posts its bond. If the owner asks you to sponsor finds with their wallet: ${petri} oracle sponsor <contentHash> --yes. A product counts as covered once a source of it has a supporter: ${petri} oracle sources support ${month} --source <source address> --stake 1 --yes. When every product is covered, the owner decides when to close the source window (Scramble's challenge days follow): ${petri} oracle sources close ${month} --yes.`,
    finish, next,
  ].join('\n') };
}
// The fields a comparison series must define (the commodity spec's
// comparison_series_manifest): petri oracle series check names what is missing.
const SERIES_FIELDS = `seriesId and definitionVersion; bucket; evidenceClass (published_assessment, or direct_price for one exact part) and priceBasis (contract); origin.publisher, origin.locator (the page) and origin.row (the exact row or series label on it); physicalScope; nativeUnit; statistic (fixed before any observation); currency; procurementSegment and geography; referencePeriod and cadence; revisionPolicy and missingDataPolicy; the template's product scope block (for RAMX dram: memoryFamily, generation, dieDensity, dieCount, condition, grade, organization, speed; for NANDX nand: architecture, cellType, dieDensity, testingScope; another product's as its template names them); a direct price also direct.manufacturer, mpn, quantityTier, deliveryTerms, pricingPeriod, taxFreight and paymentTerms`;
function chipSteps(result, flags, { report, finish, next, owner, month, petri }) {
  const run = result.run.id, assignment = result.assignment, objective = assignment.objective || {};
  const product = objective.bucketName ?? objective.bucketLabel ?? 'see the brief', bucket = objective.bucketLabel ?? objective.bucketId ?? '<bucket>';
  const source = objective.sourceAddress ?? objective.sourceId ?? '<source address>', url = objective.url ?? '<page URL>';
  if (objective.kind === 'source') {
    return [
      `Research this now: ${assignment.title}. Its brief and exact scope (candidateScope) are in "assignment". The month takes new sources now.`,
      owner,
      `This month is on commodity chip buckets: each source is one comparison series of ${product}, a repeatable contract price (a published contract assessment, or a direct contract price of one exact part) defined field by field. Never guess a field: leave what the source doesn't say null, and the check says what is missing.`,
      report,
      `1. Get the series template: ${petri} oracle series template --bucket ${bucket} --out series.json (add --direct for a direct price of one exact part).`,
      `2. Fill it from the source itself. The spec requires: ${SERIES_FIELDS}.`,
      `3. Check it: ${petri} oracle series check series.json ${month}. Fix what it names until it says Fits the bucket. Wrong product, Not a contract price or Research only means the page doesn't belong in this bucket: move on.`,
      `4. Submit it: write its candidate JSON (marketId, expiryId, bucketId from candidateScope; evidence: what the page shows; series: the checked series object) to a file, then run: ${self(`work submit ${run}`, flags)} --file <file>. The page, source type and definition come from the series.`,
      'A page behind a sign-in (a subscription assessment, a distributor portal) counts too, with a web proof:',
      ...signedIn(petri, { page: '<page URL>', json: '<JSON URL>',
        find: '- Find the JSON the page loads with the price (the browser\'s network panel). Its whole response is disclosed: pick one with nothing in it the owner would not share.',
        submit: `- Put the JSON URL in the series' origin.locator, check it again, and submit it with the bundle: ${self(`work submit ${run}`, flags)} --file <file> --evidence-bundle <bundle file>.` }),
      `A submitted find goes on the oracle once a sponsor posts its bond. If the owner asks you to sponsor finds with their wallet: ${petri} oracle sponsor <contentHash> --yes. A product counts as covered once a source of it has a supporter: ${petri} oracle sources support ${month} --source <source address> --stake 1 --yes. When every product is covered, the owner decides when to close the source window (Scramble's challenge days follow): ${petri} oracle sources close ${month} --yes.`,
      finish, next,
    ].join('\n');
  }
  const opening = objective.kind === 'opening', bond = usdc(objective.openingBondAtomic), start = objective.openingStartUtc, listing = objective.listingUtc;
  const until = listing ? ` (${listing})` : '';
  return [
    opening ? `Give this frozen source its opening value now: ${assignment.title} (${month}). Its page: ${objective.url ?? 'see "assignment"'}.`
      : `Update this live source now: ${assignment.title} (${month}). Its page: ${objective.url ?? 'see "assignment"'}.`,
    owner,
    `This source is comparison series ${objective.seriesId ?? '(see its definition)'} of ${product}; its ${opening ? 'opening value' : 'update'} is one observation of it: the price, its period and its evidence.`,
    ...(opening ? [`The month is in Opening until listing${until}, on the chain's clock: each frozen source needs an opening value, bonded from the owner's petri wallet with the product's opening bond; a source with none by listing sits out the month.`]
      : ['The month is in Game: each update is a staked new observation from the owner\'s petri wallet.']),
    `1. Get its observation template: ${petri} oracle observe template ${month} --source ${source} --out obs.json`,
    `2. Open the page and read this series' exact row in the page's own units as rawPrice. Fill in the reference period, publishedAt, retrievedAt, publicationVintage and releaseId from the page; leave what it doesn't say null.`,
    `3. Archive the page: open https://web.archive.org/save/${url} and put the archive URL it gives (https://web.archive.org/web/<14-digit capture time>/${url}, the page URL exactly as written here) in evidenceReference.${opening ? ` The capture time must be at or after the Opening's start${start ? ` (${start})` : ''} and not after the chain's clock (${petri} status shows it as Clock).` : ''}`,
    'If the page needs a sign-in, it has no public archive: prove it instead of step 3.',
    ...signedIn(petri, { page: url, json: url,
      find: `- The proof is of the source's own URL, ${url}: its whole JSON response is disclosed. Read this series' row and its price as rawPrice.`,
      submit: '- Put the bundle file in evidenceReference in place of the archive URL: the print cites the proof instead (its price must be rawPrice).' }),
    `4. Send it with the owner's wallet: ${petri} oracle observe ${month} --source ${source} --file obs.json --yes. Petri checks it against the spec first, takes the product's exact ${opening ? `opening bond${bond ? ` (${bond} USDC)` : ''}` : 'update stake'}, and refuses one that would not count, saying why. If petri says the collateral is short: ${petri} oracle collateral deposit ${bond ?? '<amount>'} --yes`,
    ...(opening ? [
      `5. Anyone can challenge it for 300 slots (about two minutes). Then accept it: ${petri} oracle prints finalize ${month} --source ${source} --yes, before listing${until}. If it was challenged, the oracle's council decides it: report that and move on.`,
      `6. Return the bond to the owner's collateral: ${petri} oracle stakes settle ${month} --kind opening-claim --source ${source} --yes`,
    ] : [
      `5. Reveal it within a minute, with the claim address observe printed: ${petri} oracle updates reveal <CLAIM> --yes. Its observation is recorded then.`,
      'Skip it if the price has not changed: an unchanged value earns nothing.',
    ]),
    report, finish, next,
  ].join('\n');
}
