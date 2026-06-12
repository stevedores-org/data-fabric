# WS10: Pilot and Rollout

Parent: [#50](https://github.com/stevedores-org/data-fabric/issues/50)

WS10 defines the pilot plan and metrics needed to validate data-fabric velocity
gains before broad adoption.

## Status

| Area | State | Notes |
|------|-------|-------|
| Phase 1 workload scope | **Done** | [#104](https://github.com/stevedores-org/data-fabric/issues/104) → [PHASE1_WORKLOAD.md](PHASE1_WORKLOAD.md) |
| Baseline KPI endpoint | **Done** | [#105](https://github.com/stevedores-org/data-fabric/issues/105) → [METRICS_ENDPOINT.md](METRICS_ENDPOINT.md) |
| Baseline collection week | Pending | Requires staffed oxidizedgraph agent runs |
| Dashboards / go-no-go / rollback | Pending | Ops + documentation, not a single code feature |
| Pilot phases 1–3 execution | Pending | Human-driven rollout |

## BTF deprecated

**Do not use `btf` (build this feature) on #50.** The buildable prep sub-issues
are closed; remaining work is operational pilot execution (baseline week, phase
gates, dashboards, rollback drills). Open scoped sub-issues for any new code
instead of attempting an end-to-end build of the parent epic.

## Docs

- [Phase 1 workload](PHASE1_WORKLOAD.md)
- [Baseline metrics endpoint](METRICS_ENDPOINT.md)
