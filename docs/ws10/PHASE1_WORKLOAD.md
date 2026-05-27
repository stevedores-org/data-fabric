# Phase 1 Oxidizedgraph Pilot Workload

Issue: #104
Parent: #50
Status: accepted baseline scope

## Goal

Define the concrete Phase 1 workload for the WS10 pilot so completion rate,
MTTR, and baseline sample size have measurable denominators before the
baseline-collection week starts.

## Workload Identity

Phase 1 uses an MCP-driven `oxidizedgraph.test` task executed by the
oxidizedgraph test-suite agent role.

The integration boundary is data-fabric's MCP task queue:

- submit work through `POST /v1/mcp-tasks`
- claim work through `GET /mcp/task/next?agent_id=...&cap=oxidizedgraph.test`
- maintain ownership with heartbeat
- finish with complete or fail

The oxidizedgraph-side executor should map this task to the repository's normal
Rust test command for the selected crate or package. Until oxidizedgraph
publishes a stable agent module name, the data-fabric contract treats the agent
identity as:

```text
agent_id: oxidizedgraph-test-suite
capability: oxidizedgraph.test
```

## Task Shape

One Phase 1 unit of work is one test invocation for a crate, package, or
explicit test filter.

```json
{
  "task_type": "oxidizedgraph.test",
  "priority": 5,
  "params": {
    "repo": "stevedores-org/oxidizedgraph",
    "ref": "main",
    "package": "oxidizedgraph",
    "test_filter": null,
    "command": "cargo test -p oxidizedgraph",
    "timeout_seconds": 900,
    "retry_budget": 1,
    "baseline_mode": "fabric_disabled"
  },
  "capabilities_required": ["oxidizedgraph.test", "rust.cargo-test"]
}
```

## Completion Semantics

A task is `completed` when the agent runs the configured command to termination
within `timeout_seconds` and uploads the test summary artifact.

Completion rate measures agent execution reliability, not test-suite health:

- Passing tests: `completed`
- Failing tests with a parsed test summary: `completed` with failure details in
  the artifact and event log
- Build error with captured compiler output: `completed` with failure details
- Agent crash, lease expiry, missing artifact, invalid task params, or timeout:
  `failed`

Infrastructure flakes can be excluded from the denominator only when the event
log classifies the failure as infrastructure-owned. Examples:

- GitHub runner unavailable
- dependency registry outage
- network outage confirmed outside the agent
- runner OOM unrelated to repository test behavior

## MTTR Clock

MTTR is measured on the same `run_id` or task thread.

- Start event: first `mcp_tasks.status = 'failed'` event for the work unit.
- End event: next `mcp_tasks.status = 'completed'` event for the same
  workload identity and failure signature.

If a retry succeeds after one retry, the MTTR is the elapsed time between the
failed terminal event and the successful terminal event. If no retry succeeds
inside the measurement window, the task is counted as unresolved for the MTTR
sample.

## Expected Volume

Baseline collection should run for seven days before Phase 1 pilot evaluation.

Recommended starting volume:

| Window | Tasks |
| --- | ---: |
| Hourly smoke run | 24/day |
| PR-triggered targeted run estimate | 10/day |
| Nightly full-package run | 1/day |
| Total expected volume | 35/day |

At this rate the baseline week yields roughly 245 task samples, which is enough
to make an 80% completion-rate gate meaningful without overloading the first
pilot.

## Baseline Mode Decision

Use `fabric_disabled` as the baseline mode.

In this mode the same oxidizedgraph test-suite agent executes the same command
shape, but task dispatch and lifecycle tracking are recorded without using the
fabric as the scheduling authority. This keeps the comparison closer to
apples-to-apples than comparing against unrelated non-agent CI runs.

## Go/No-Go Inputs

The Phase 1 go/no-go should use:

- completion rate for `task_type = oxidizedgraph.test`
- MTTR for retryable failures
- unresolved task count
- infrastructure-flake exclusion count
- sample size over the seven-day baseline

The pilot should not start until the baseline week has at least 200 task samples
or an explicit architecture review accepts a smaller sample.

