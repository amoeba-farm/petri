# Petri CLI Metrics

## Metrics Contract

This repo should expose or emit observable signals for agent and human feedback. Metrics names below are target names; implement them incrementally as code surfaces become ready.

## Target Signals

- petri_command_runs_total
- petri_command_failures_total
- petri_backend_request_duration_seconds
- petri_mock_backend_checks_total

## Rules

- Metrics must not include secrets, wallet material, raw credentials, or private user payloads.
- Use bounded labels only; avoid unbounded IDs, URLs, or free-form error strings as labels.
- Runtime services should prefer Prometheus-compatible `/metrics`; CLI/harness/research repos may emit structured JSON summaries.
- Durable event summaries should go to Postgres or report artifacts, not high-cardinality Prometheus labels.

## Dashboard Notes

See root `OBSERVABILITY.md` and `observability/docs/metrics-taxonomy.html` for workspace-wide metric naming and dashboard expectations.
