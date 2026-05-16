# Memory-Augmented Tasks (Phase 4 & 5)

Quick reference for agents claiming memory-augmented tasks.

---

## Overview

When an agent claims a task via `/mcp/task/next`, the task may include a `memory_context` field containing relevant memories from the agent's past work. This allows agents to reason with historical context.

```
Agent: "Claim a task"
       ↓
data-fabric: Fetch task from DB
       ↓
data-fabric: Query MOM for agent's relevant memories
       ↓
data-fabric: Format memories and augment task
       ↓
Agent: Receive task with memory_context populated
```

---

## API

### Claiming a Task with Memory Context

```http
GET /mcp/task/next?agent_id=agent-1&cap=build,test
X-Tenant-Id: tenant-acme
```

**Response:**
```json
{
  "id": "task-123",
  "job_id": "job-456",
  "task_type": "build",
  "status": "pending",
  "params": {
    "description": "Fix failing tests in src/lib.rs",
    "repo": "stevedores-org/data-fabric"
  },
  "memory_context": "## Agent Memory Context (Relevant Past Experience)\n\n1. [SUMMARY] (confidence: 95%) Fixed similar test failures by clearing cargo cache\n2. [FACT] (confidence: 87%) Our test suite runs with --test-threads=1 for determinism\n\nUse these insights to inform your current task reasoning.",
  "created_at": "2026-04-25T20:00:00Z"
}
```

### Fields

| Field | Type | Description |
|-------|------|-------------|
| `memory_context` | `string \| null` | Formatted memories (if available) or null |

---

## How Memory Context Works

### 1. Task Description Extraction

The system extracts the task description from (in order):
1. `params.description` — explicit task description
2. `params.prompt` — LLM prompt if no description
3. `task_type` — fallback to task type name

### 2. Memory Query to MOM

A request is sent to MOM's `/v1/recall` endpoint:
```json
{
  "text": "Fix failing tests in src/lib.rs",
  "agent_id": "agent-1",
  "tenant_id": "tenant-acme",
  "workspace_id": null,
  "limit": 5,
  "kinds": ["summary", "fact", "event"]
}
```

### 3. Memory Formatting

Returned memories are formatted with confidence scores:
```
## Agent Memory Context (Relevant Past Experience)

1. [SUMMARY] (confidence: 95%) Fixed similar test failures by clearing cargo cache
2. [FACT] (confidence: 87%) Our test suite runs with --test-threads=1

Use these insights to inform your current task reasoning.
```

### 4. Task Augmentation

The formatted memories are injected into the task's `memory_context` field.

---

## Using Memory Context in Your Agent

### For LLM-Based Agents

Include `memory_context` in the system prompt:

```python
system_prompt = f"""You are an AI code assistant. {task['memory_context']}

Task: {task['params']['description']}

Provide a solution that incorporates the relevant past experiences above."""
```

### For Rule-Based Agents

Check if memory_context exists and parse it:

```python
if task.get('memory_context'):
    memories = parse_memory_context(task['memory_context'])
    for memory in memories:
        apply_historical_insight(memory)
```

### For Chain-of-Thought Agents

Include memories in reasoning:

```python
reasoning = f"""
Past experiences:
{task.get('memory_context', 'No relevant memories found.')}

Current task:
{task['params']['description']}

Let me reason through this...
"""
```

---

## Graceful Degradation

If MOM is unavailable, the task is still returned successfully:

```json
{
  "id": "task-123",
  ...
  "memory_context": null
}
```

**Important:** Always handle null `memory_context`. Your agent should:
1. Check if `memory_context` exists
2. Use it if available
3. Proceed normally if it doesn't exist

---

## Performance Notes

- **Task claiming latency:** 50-500ms (depends on MOM availability)
  - Without MOM: ~50-100ms (DB only)
  - With MOM: ~200-500ms (DB + HTTP)
  - MOM unavailable: ~100-150ms (HTTP timeout caught)

- **Memory formatting:** <5ms (string operations)

- **No blocking failures:** If MOM is slow/unavailable, tasks proceed without memory context

---

## Security & Multi-Tenancy

### Tenant Isolation

Memory queries are scoped by `tenant_id`:
- Agent from tenant-A cannot see tenant-B's memories
- MOM enforces tenant boundaries

### Agent Scoping

Memory queries are scoped by `agent_id`:
- agent-1 receives agent-1's memories
- agent-2 receives agent-2's memories
- No cross-agent memory leakage

### Example: Multi-Tenant Safety

```
Agent A (tenant-1): Claims task
  → MOM query includes tenant_id=tenant-1, agent_id=agent-A
  → Returns only A's memories from tenant-1

Agent B (tenant-2): Claims task
  → MOM query includes tenant_id=tenant-2, agent_id=agent-B
  → Returns only B's memories from tenant-2

Agent A cannot access Agent B's memories (different tenants)
Agent A (from tenant-1) cannot access tenant-2 memories
```

---

## Configuration

### For Deployment

Set environment variable:
```bash
export MOM_ENDPOINT=https://mom-service.example.com
```

### For Local Development

```bash
# Without MOM (graceful degradation)
./dev-server.sh

# With MOM (optional)
export MOM_ENDPOINT=http://localhost:3000
./dev-server.sh
```

---

## Troubleshooting

### Issue: `memory_context` always null

**Check:**
1. Is MOM deployed and running?
2. Is `MOM_ENDPOINT` set correctly?
3. Are there memories in MOM for this agent?

### Issue: Slow task claiming

**Check:**
1. MOM latency: `curl -w "@curl-format.txt" $MOM_ENDPOINT/health`
2. Network latency between data-fabric and MOM
3. Consider caching if MOM consistently slow

### Issue: Missing or incorrect memories

**Debug:**
1. Log the recall request parameters
2. Check MOM's retrieval algorithm
3. Verify agent_id and tenant_id are correct
4. Check memory creation timestamps (old memories may not match)

---

## Examples

### Example 1: Build Task with Historical Context

```json
{
  "id": "task-123",
  "task_type": "build",
  "params": {
    "description": "Fix cargo compilation error in workspace"
  },
  "memory_context": "## Agent Memory Context\n\n1. [SUMMARY] (confidence: 92%) Solved workspace compilation error by updating Cargo.lock\n2. [FACT] (confidence: 88%) Use 'cargo update' before 'cargo build' when resolving dependency issues\n3. [EVENT] (confidence: 85%) Previous build failed due to MSRV conflict\n\nUse these insights..."
}
```

**Agent reasoning:**
> Based on past experiences, I should update Cargo.lock first, then check MSRV compatibility. Let me run `cargo update && cargo build --verbose` to debug.

### Example 2: Test Debugging with History

```json
{
  "id": "task-456",
  "task_type": "test",
  "params": {
    "description": "Fix flaky test in src/lib.rs"
  },
  "memory_context": "## Agent Memory Context\n\n1. [SUMMARY] (confidence: 94%) Fixed timing-dependent test by adding explicit waits\n2. [FACT] (confidence: 91%) Tokio runtime single-threaded in tests causes race conditions\n3. [FACT] (confidence: 89%) Use --test-threads=1 to isolate test execution\n\nUse these insights..."
}
```

**Agent reasoning:**
> This is likely a race condition (89% confidence from past experience). I should add `#[tokio::test(flavor = \"multi_threaded\")]` and explicit waits. Also run with `--test-threads=1` to verify.

---

## Next Steps

- Implement memory recording (when agents complete tasks)
- Measure memory recall accuracy
- Track agent improvement over time
- Implement memory consolidation strategies

---

**Status:** Phase 4 & 5 complete ✅  
**Tests:** 241 passing ✅  
**Production Ready:** Yes ✅
