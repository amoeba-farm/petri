# Oracle research in Petri

The canonical runtime is now bundled in the `petri` executable. Open it with
`petri tui`; use `petri research` for research automation. Installed preview apps
automatically update the CLI, TUI, and service together. Source builds require
Node 24 and update only through an explicit command.

The existing `%LOCALAPPDATA%/PetriResearch` workspace (or `PETRI_RESEARCH_HOME`)
is preserved across application updates. The app can restart its authenticated
local service when its embedded code changes; requests in progress finish first.
A pre-integration standalone service must be closed once before first launch.

For local development, `PETRI_RESEARCH_UPSTREAM`, `PETRI_RESEARCH_ORACLE_URL`, and
`PETRI_RESEARCH_PORT` configure a new workspace. Existing workspace upstream
bindings remain protected.

The four views are **Connect**, **Research**, **Evidence**, and **Rewards**.
**Fund** is the other side: it lists the price pages research agents found
(candidates, not yet on the oracle), your oracle balance that bonds and
stakes come from (**Add funds**, **Withdraw**, reviewed and signed by your
wallet), the sources your wallet listed, and your sponsorships. **Sponsor
it** on a find posts its product's bond from your oracle balance and puts it
on the oracle under the researcher's name: if it goes live, the researcher
gets half its reward and you the rest (`petri oracle sponsor`).
Agents keep their own model and harness. The same authenticated operations are
available through the JSON CLI and a local stdio MCP server.

The TUI opens directly into the workspace. Its layout uses a small text header,
neutral colors, fixed borders, and spaced controls. Activity reports appear as
a timestamped data feed, with event and detail columns when space allows.
Conversations stay in the connected agent's harness.

## Boundaries

- The workspace service owns pairing, revocable agent credentials, sessions,
  standing research directives, discovered work, and a durable history of actions.
- The Amoeba candidate API owns researcher registration, source validation,
  deduplication, attribution, and verified oracle outcomes. This workspace never
  calculates eligibility, grants an oracle acceptance, or creates a reward.
- Agents can research and submit evidence. There are no signing, funding,
  trading, withdrawal, or transaction-broadcast tools.
- An agent's progress report is distinct from a backend-confirmed submission
  or oracle outcome. Research completion does not mean a bounty was earned.
- The local service binds loopback. Remote upstream candidate APIs require HTTPS;
  plain HTTP is accepted only for loopback development.

## Source layout

- `research/service/`: local workspace authentication, persistence, API, upstream client.
- `research/client/`: shared agent client, private profile storage and MCP adapter.
- `research/bin/`: research CLI, service startup, and private desktop bootstrap.
- `src/research/`: Rust TUI; all mouse and keyboard actions share one handler.
- `research/test/`: authentication, attribution, persistence and protocol regressions.

## Open Petri

Installed preview builds include Node 24 and all research application code.
From a terminal:

```sh
petri tui
```

The launcher starts the local workspace service and opens the TUI. Closing the
TUI leaves the service running so connected agents can continue working.
The service listens on `http://127.0.0.1:4185`. The development candidate API is
`http://127.0.0.1:4184`; that separate Amoeba backend must be running before
agent approval, candidate submission or reward observation can succeed.

For a source build on another machine, install Node.js 24 and Rust, run
`cargo build --locked --bin petri`, then `target/debug/petri tui`. There are no npm
dependencies. This is a local application; a hosted service is not included.

## Connect an agent and direct research

1. In **Connect**, click **Connect wallet**. It offers this Petri's own wallet;
   Enter connects it, or enter another public Solana address. Connect then
   shows it as the **Active wallet** (**Change** connects another). Rewards go
   to it: it is recorded for newly registered agents, and existing agent
   identities keep their original one, which their own page shows.
2. Click **Agent** to choose your harness from the list, and **Name** to name
   the agent. **Copy agent setup** copies a short message with the agent's
   login command and shows, in three steps, how to paste it into a chat
   with your agent (**Copy again** copies it once more).
3. The agent runs the command and shows a pairing code. Match it with the
   request in **Connect**, then click **Approve**. The agent runs the command
   again to finish, then `work next`, and keeps researching from there.
4. The sidebar lists your agents, each its harness over its name (a dot while
   it researches). Click one for its page in **Connect**: what it is doing,
   its wallet and **Disconnect agent**. An agent that is connected but idle
   offers **Copy start message**: paste it into the agent's chat to get it
   researching again (the same steps show after the copy). **+ New agent**
   sets up another one.
5. **Research** is a navigator beside the selected agent's page. The
   navigator drills down: **Products** (one row per oracle product, with its
   months and open tasks; products and months are the oracle's own, so work
   of a month it no longer lists shows nowhere), then a product's months
   (each with its phase as five dots, done ●, now ◉, to come ○, and its
   rewards left), then a
   month's categories (its products, each with its source count, its
   temperature box, hot to cold, and the agent on it), then one category:
   what it still needs, who is on it, your directive, and its tasks (an
   update or opening per source; click one to pick it). Every crumb of
   `Products › RAMX › Dec 2026 › …` goes back up, as Backspace does.
   **Research this** puts the work first in your agents' list (they can
   still take any; **Stop researching** undoes it), and **Directive** tells
   them where to look. The list checks again every minute by itself.
6. The agent's page says what it is on, then its live feed, newest at the
   bottom: ⌕ a search, ↗ a page it read, $ a value it read, ✓ a source found
   or received, ⇪ a write it made, · a note, ! blocked, ▸ and ■ a run's start
   and end, and `you:` your directives. Scroll up to read back; it keeps its
   place until you scroll down again or click **▾ newer**. A line of a run
   shows that run's work in the navigator. Selecting an agent follows it:
   the navigator moves to its work as it moves. A click in the navigator
   stops following; **Follow** resumes it. Each agent keeps its own page.
7. Select an agent in the sidebar for its **Evidence** and **Rewards** too:
   what it earned, as a chart of each month, and each of its finds a sponsor
   put on the oracle.

Petri does not launch a model or run research by itself. Claude Code, Codex,
Kimi Code, Gemini CLI or your own DeepSeek harness continues doing the work.
Any harness able to invoke a local command can use the JSON CLI. A harness with
stdio MCP support can use the same operations through MCP. Provider accounts
and model credentials stay with that harness.

The setup message's command uses the private bundled Node runtime and a stable
agent launcher, with the current workspace URL and agent name. This leaves the
Petri executable free to update while agents are connected. New agent sessions
load the current application code; existing sessions finish with their loaded
client.

Give each independently running agent its own name/profile. An identity can be
resumed after restarting its harness. Profiles default to
`%LOCALAPPDATA%\PetriResearch\profiles` on Windows, and
`~/.local/share/PetriResearch/profiles` elsewhere. Use `--profile-dir` for a
different dedicated directory. A profile is bound to its original workspace.

## Keyboard and mouse

Click tabs, agent rows, buttons and form fields; a click outside a pop-up
closes it. Use the mouse wheel to scroll. **Agent** in Connect opens the list
of harnesses.
An 80×24 terminal works; 100×32 gives the connection form more room.

| Key | Action |
| --- | --- |
| `c`, `a`, `e`, `r`, `f` | Connect, Research, Evidence, Rewards, Fund |
| `Tab` / `Shift+Tab`, `Enter` | Focus and activate a control |
| `↑` / `↓` | Select an agent |
| `n` | New agent |
| `Backspace` | Research: up one level |
| `PageUp` / `PageDown`, `Home` | Scroll content |
| `Ctrl+Enter` | Save a form |
| `Esc` | Close a form; quit from the main view |
| `q` / `Ctrl+C` | Quit (`q` enters text when editing a form) |

## CLI and MCP

Run these from this checkout, or substitute the absolute path to `bin/petri.mjs`.
Every CLI result is JSON; credentials are saved privately rather than printed.

```powershell
petri research agent login --name cedar --harness 'Claude Code'
petri research work next --agent cedar
petri research work list --agent cedar
petri research work start <work-id> --agent cedar --key research-1
petri research work report <run-id> --agent cedar --kind read --source https://example.com/ram --message 'Crucial 64GB DDR5 RDIMM'
petri research work submit <run-id> --agent cedar --key source-1 --file candidate.json
petri research work finish <run-id> --agent cedar --summary 'Submitted the matching public source.'
petri research rewards --agent cedar
```

`work next` starts (or resumes) the agent's next run: your picks first, then
what needs the most, skipping what another agent is on while there is other
work. It prints the run, its assignment with your directive, and the exact
commands to report, submit and finish; then the agent runs `work next` again.
An agent counts as researching while it has checked in within 20 minutes.

Agents report each step as they go, one short line each (`--kind`, default
`note`); the agent's page in the TUI shows them live:

| Kind | Message | `--source` |
| --- | --- | --- |
| `search` | the query | optional |
| `read` | the page opened, e.g. its title | required |
| `value` | a value read off a page, e.g. `289.99 USD, DDR5 RDIMM 64GB` | optional |
| `submit` | a petri write, e.g. `update committed` | optional |
| `note`, `source_found`, `blocked` | anything else, a price page, a blocker | optional |

References are public HTTPS URLs. When you add or change a directive for a
product, each agent on it gets it in its feed (`owner`, `directive`), and a
run that starts later opens with it.

Reuse the same `--key` and request content when retrying a start, report or
submission whose response was lost. MCP requires these keys. A retry must not
be represented as new work. Candidate duplicates retain the original researcher's
attribution, including when another agent finds the same source.

The eleven MCP tools are `petri_login`, `petri_work_next`, `petri_work_list`, `petri_work_start`,
`petri_report_progress`, `petri_submit_candidate`, `petri_run_finish`,
`petri_activity`, `petri_rewards`, `petri_session_open`, and `petri_session_close`.
Start the server with `petri research mcp --agent cedar --harness 'Claude Code'`.
It uses newline-delimited JSON-RPC over stdio and protocol version `2025-06-18`.

A candidate file contains the backend's source fields:

```json
{
  "marketId": "canonical market ID from the research objective",
  "expiryId": "canonical expiry ID from the research objective",
  "bucketId": "canonical 64-character lowercase hexadecimal bucket ID",
  "canonicalUrl": "https://public-source.example/product",
  "sourceType": "retail product listing",
  "productDefinition": "Exact product and specifications",
  "priceField": "Identifiable public price field",
  "region": "United States",
  "currency": "USD",
  "evidence": "Concrete product, price and source evidence"
}
```

These placeholders are not a valid submission. Work generated for a product
group binds the exact market, expiry and bucket on every submission. For a
confirmed empty inventory, the objective is market-wide: the agent must first
establish the canonical product group before submitting. Petri never invents
bucket identifiers from labels. The backend owns source validation.

## Standing directives and work discovery

The loop is **directive → oracle assessment → research objective → agent run →
evidence → later oracle assessment**. Titles and descriptions on work items are
generated summaries of structured observations, rather than user-authored inputs.
Human guidance is a lower-level directive attached to a particular objective.
Runs retain the exact objective, directive revision and guidance they started
with. Refining guidance does not rewrite completed or running research history.

The background service reads `/v1/oracle/public-snapshot` and
`/v1/oracle/catalog` from the futures API (Petri passes the API it uses for
markets: `--api`, `PETRI_API_URL` or the saved `api`, else the profile's,
`https://api.amoeba.farm` in production and `http://127.0.0.1:4500` with
`--profile dev`) once per minute. **Scan** requests an immediate
assessment. Concurrent scans are coalesced. These are prepared public GETs;
Petri performs no per-source RPC scans or source-page scraping in this loop.
Source candidates go to the same futures API's candidate store
(`/oracle/agents`, `/oracle/source-candidates`). Use `--upstream` and
`--oracle-url` (or `PETRI_RESEARCH_UPSTREAM` / `PETRI_RESEARCH_ORACLE_URL`) on
first launch to pin a dedicated workspace elsewhere; a workspace stays bound to
the endpoints it was created with, except that one bound to the retired local
candidate API (`127.0.0.1:4184`) is rebound to the futures API. The oracle reads receive no
researcher or owner credentials.

Discovery checks exact program, cluster, market, series and oracle-month
identity; settlement time; observation freshness; complete source inventories;
and unique source IDs. It counts only `Active` records. Host coverage uses
verified HTTPS locators and normalizes a leading `www.`. Different hosts are a
research signal, not proof of independent ownership. Pending candidates and
agent progress reports never count as active oracle sources.

A missing oracle month, stale snapshot, missing inventory or uncertain host
coverage stays unknown. Missing data never becomes a zero-source claim. An
explicitly complete empty inventory can create market-wide research work.
Settled series are excluded. Product-group assessments cover the bucket IDs
actually present in the published inventory; absent governed groups cannot yet
be exhaustively discovered without a canonical bucket catalog. Source-observation
freshness is checked; the snapshot does not establish that a publisher's page
or quoted price is current. This version does not detect website-price staleness.

Each month's oracle phase sets its work. While it takes sources
(SourceSubmission, Scramble's placement days 1-4) every product it lists gets a
source gap. In Opening each frozen source still without an opening value gets
one opening task, with its product's opening bond and budget when the snapshot
lists them; it follows the chain's phase, since the oracle takes openings until
listing on the chain's own clock. In Game each live source gets an update task.
A Settled month whose rewards are frozen gets one claims task, if this
workspace's agents worked on that month. It comes up until a run on it
finishes, and again when the owner picks it: `petri oracle rewards <month>`,
then `petri oracle rewards claim <month> --yes` and `petri oracle sponsorships
claim <source> --yes` for what that lists. Scramble's challenge and freeze
days take nothing. `work next` hands out the
owner's picks first, then openings (the Opening lasts a day), then what needs
the most, and prints that phase's petri steps for the owner's wallet: for an
opening, read the value, archive the source's exact URL with a capture from the
Opening's start up to the chain's clock, `petri oracle prints opening` with the
product's opening bond, `prints finalize` 300 slots later and before listing,
and `petri oracle stakes settle --kind opening-claim` for the bond. An opening
or update that earns nothing this month comes up only when the owner picks it.

Repeated assessments update stable objectives. Fresh evidence of adequate
coverage resolves an objective; a later shortage reopens it. Stale or failed
assessments withdraw new work until the facts can be checked again. A
successful assessment archives the work of a series the catalog no longer
lists (a month no longer served), with its runs kept; it comes back if the
series is listed again. Work of a listed series stays, whatever its status, and
a failed assessment archives nothing. Finishing a run or submitting
candidates never resolves a gap. Research may happen before
an admission window: a coverage objective is not permission to activate a source.

Edit a directive without a title or brief:

```json
{
  "marketIds": ["ramx", "nandx"],
  "minActiveSources": 3,
  "minDistinctHosts": 2,
  "maxObservationAgeMs": 600000,
  "enabled": true
}
```

Save with `petri research owner directive save --file directive.json`.
Include an existing directive's `id` to update it; omit it to create another.
An empty market list selects all published markets. Pause/resume is available
in the TUI. Add human guidance with
`petri research owner refine <work-id> --message 'Prioritize manufacturer sources.'`.
Agent credentials cannot edit directives or forge human guidance. A future Farm
model can be integrated as an explicitly authorized director; none is connected
or autonomously modifying directives in this version.

Earlier manual assignments are archived during migration, retaining their runs,
submissions and attribution. The manual assignment creation route is retired.

## Storage and development

Owner state defaults to `%LOCALAPPDATA%\PetriResearch` on Windows and
`~/.local/share/PetriResearch` elsewhere. Set `PETRI_RESEARCH_HOME` or use
`--state-dir` to choose another empty, dedicated directory. This contains the
SQLite work history, owner credential, encryption key and private profiles.
Back it up as private data. Never paste `owner.json` or profile contents into an
agent conversation. The owner credential is not included in copied setup.

An isolated instance can use `--state-dir <empty-directory> --port 4186`.
On first launch, `--upstream https://your-candidate-api.example` configures the
candidate API. It must implement the Amoeba agent and candidate endpoints.
Use a separate state directory for a different backend. The service supports
same-machine clients; remote workspace hosting and remote login are not packaged.

```powershell
node --test research/test/*.test.mjs
cargo fmt -- --check
cargo test --locked --bin petri
cargo build --locked --bin petri
```

The optional real Lean integration test requires a backend checkout with its
`oracle_sponsorship_decision` executable already built. Set
`PETRI_TEST_LEAN_BACKEND` to that checkout and rerun the Node tests. It creates
an ephemeral local candidate service and in-memory database.

Tests cover coverage discovery, unknown versus confirmed shortage, stable work
identity, resolution/reopening, directive scope and pause, immutable run context,
human guidance, agent isolation, revocation, session recovery, submission retries,
duplicate attribution, private storage, the HTTP boundary, a launched MCP
process, and mouse/keyboard interaction at several terminal sizes. Individual
vendor harness connections have not been exercised in those applications.
Reward display depends on real backend observations; this app does not activate
settlement or payments, and a local source submission proves neither.
