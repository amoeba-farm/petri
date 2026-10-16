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

The four views are **Connect**, **Activity**, **Evidence**, and **Rewards**.
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

1. In **Connect**, click **Reward address** and enter your public Solana address.
   This is the destination recorded for newly registered agents. Existing agent
   identities keep their original destination.
2. Choose an agent name and harness. **Copy login** gives you a command to run
   from that agent's terminal. **Copy MCP** gives you its connection configuration.
3. Have the agent log in. Match its pairing code with the request shown in
   **Connect**, then click **Approve**. Have the agent call login again to finish.
4. In **Activity → Directives**, edit the coverage targets or market scope.
   Petri starts with an enabled directive for all markets: at least **3 active
   sources and 2 distinct source hosts per product group**. These editable targets
   guide research; they do not change the oracle's acceptance rules.
5. **Activity → Work** shows research objectives derived from confirmed coverage
   gaps. Open one and click **Refine** to add human guidance about where to look,
   product requirements or research priorities. No title or brief is required.
   Agents discover this work, start runs, report progress and submit candidates.
6. Select an agent in the sidebar to inspect its **Runs**, **Evidence** and
   **Rewards**. **Disconnect agent** in Connect revokes access and retains its history.

Petri does not launch a model or run research by itself. Claude Code, Codex,
Kimi Code, Gemini CLI or your own DeepSeek harness continues doing the work.
Any harness able to invoke a local command can use the JSON CLI. A harness with
stdio MCP support can use the same operations through MCP. Provider accounts
and model credentials stay with that harness.

MCP connection snippets use the private bundled Node runtime and a stable agent
launcher, with the current workspace URL and agent name. This leaves the Petri
executable free to update while agents are connected. New agent sessions load
the current application code; existing sessions finish with their loaded client. The Codex selection produces a TOML `mcp_servers` section; the
others produce the common JSON `mcpServers` format. Merge the snippet into your
harness configuration without replacing other servers. Exact installation
locations and configuration support depend on the harness.

Give each independently running agent its own name/profile. An identity can be
resumed after restarting its harness. Profiles default to
`%LOCALAPPDATA%\PetriResearch\profiles` on Windows, and
`~/.local/share/PetriResearch/profiles` elsewhere. Use `--profile-dir` for a
different dedicated directory. A profile is bound to its original workspace.

## Keyboard and mouse

Click tabs, agent rows, buttons and form fields. Use the mouse wheel to scroll.
At narrow widths, click the displayed harness name to cycle through harnesses.
An 80×24 terminal works; 100×32 gives the connection form more room.

| Key | Action |
| --- | --- |
| `c`, `a`, `e`, `r` | Connect, Activity, Evidence, Rewards |
| `Tab` / `Shift+Tab`, `Enter` | Focus and activate a control |
| `↑` / `↓` | Select an agent |
| `n` | New standing directive |
| `PageUp` / `PageDown`, `Home` | Scroll content |
| `Ctrl+Enter` | Save a form |
| `Esc` | Close a form; quit from the main view |
| `q` / `Ctrl+C` | Quit (`q` enters text when editing a form) |

## CLI and MCP

Run these from this checkout, or substitute the absolute path to `bin/petri.mjs`.
Every CLI result is JSON; credentials are saved privately rather than printed.

```powershell
petri research agent login --name cedar --harness 'Claude Code'
petri research work list --agent cedar
petri research work start <work-id> --agent cedar --key research-1
petri research work report <run-id> --agent cedar --key step-1 --message 'Read the exact public listing.'
petri research work submit <run-id> --agent cedar --key source-1 --file candidate.json
petri research work finish <run-id> --agent cedar --summary 'Submitted the matching public source.'
petri research rewards --agent cedar
```

Reuse the same `--key` and request content when retrying a start, report or
submission whose response was lost. MCP requires these keys. A retry must not
be represented as new work. Candidate duplicates retain the original researcher's
attribution, including when another agent finds the same source.

The ten MCP tools are `petri_login`, `petri_work_list`, `petri_work_start`,
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

Repeated assessments update stable objectives. Fresh evidence of adequate
coverage resolves an objective; a later shortage reopens it. Stale or failed
assessments withdraw new work until the facts can be checked again. Finishing a
run or submitting candidates never resolves a gap. Research may happen before
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
