# Deployment Guide — MOM Integration Ready

data-fabric is configured to deploy to development, staging, and production environments with MOM memory service integration enabled.

---

## Environment Configuration

```toml
# Development
MOM_ENDPOINT=https://mom-service.lornu.com

# Staging
MOM_ENDPOINT=https://mom-service.stevedores.org

# Production
MOM_ENDPOINT=https://mom-service.lornu.com
```

---

## Deployment Commands

### Development

```bash
# Deploy to development environment
bunx wrangler@3 deploy --env development

# Test locally
bunx wrangler@3 dev --env development

# View logs
bunx wrangler@3 tail --env development
```

**Endpoint:** https://data-fabric.stevedores.org (or local dev)  
**MOM Service:** https://mom-service.lornu.com

### Staging

```bash
# Deploy to staging environment
bunx wrangler@3 deploy --env staging

# Verify staging deployment
curl -H "X-Tenant-Id: test" \
  https://data-fabric-staging.stevedores.org/health

# Monitor staging
bunx wrangler@3 tail --env staging
```

**Endpoint:** https://data-fabric-staging.stevedores.org  
**MOM Service:** https://mom-service.stevedores.org

### Production

```bash
# Deploy to production environment
bunx wrangler@3 deploy --env production

# Verify production deployment
curl -H "X-Tenant-Id: test" \
  https://data-fabric.stevedores.org/health

# Monitor production
bunx wrangler@3 tail --env production

# Get alerts/logs
bunx wrangler@3 tail --env production --follow
```

**Endpoint:** https://data-fabric.stevedores.org  
**MOM Service:** https://mom-service.lornu.com (primary)

---

## Pre-Deployment Checklist

- [ ] All 241 unit tests passing: `cargo test --lib`
- [ ] Zero clippy warnings: `cargo clippy --lib`
- [ ] Local verification: `./scripts/test-mom-integration.sh`
- [ ] Review CHANGELOG for breaking changes
- [ ] Verify MOM endpoints are healthy
- [ ] Check D1 database backups
- [ ] Notify team of deployment window

---

## Deployment Process

### 1. Development → Staging

```bash
# Build and test locally
cargo test --lib
./scripts/test-mom-integration.sh

# Deploy to staging
bunx wrangler@3 deploy --env staging

# Verify health
curl -H "X-Tenant-Id: test" https://data-fabric-staging.stevedores.org/health

# Test memory augmentation
curl -H "X-Tenant-Id: test-tenant" \
  "https://data-fabric-staging.stevedores.org/mcp/task/next?agent_id=test-agent&cap=test"

# Monitor for 5-10 minutes
bunx wrangler@3 tail --env staging
```

### 2. Staging → Production

```bash
# Verify staging is stable
bunx wrangler@3 tail --env staging | head -20

# Run final checks
./scripts/test-mom-integration.sh

# Deploy to production
bunx wrangler@3 deploy --env production

# Verify health
curl -H "X-Tenant-Id: test" https://data-fabric.stevedores.org/health

# Monitor for errors
bunx wrangler@3 tail --env production --follow

# Check metrics (if available)
# - Task claiming latency
# - MOM HTTP success rate
# - Memory context population rate
```

---

## Post-Deployment Verification

### Health Check

```bash
curl -v -H "X-Tenant-Id: test-tenant" \
  https://data-fabric.stevedores.org/health
```

Expected response:
```json
{
  "service": "data-fabric",
  "status": "ok",
  "mission": "Enable agent-to-agent communication through memory"
}
```

### Task Claiming Test

```bash
curl -v -H "X-Tenant-Id: test-tenant" \
  "https://data-fabric.stevedores.org/mcp/task/next?agent_id=test-agent&cap=test"
```

Expected:
- ✅ HTTP 200 OK (or 204 if no tasks)
- ✅ Response includes `memory_context` field (may be null if no memories)
- ✅ Response time < 1 second

### Monitor Logs

```bash
# Production logs
bunx wrangler@3 tail --env production

# Look for:
# - No errors from MOM HTTP calls
# - Normal response times
# - Graceful degradation if MOM unavailable
```

---

## Rollback Plan

If issues occur post-deployment:

```bash
# Identify last stable commit
git log --oneline | head -5

# Redeploy previous version
git checkout <previous-commit>
bunx wrangler@3 deploy --env production

# Verify rollback
curl https://data-fabric.stevedores.org/health
```

---

## Monitoring & Observability

### Key Metrics

Track in your monitoring system:
- **Task claiming latency** (p50, p95, p99)
- **MOM HTTP success rate** (%)
- **Memory context population** (% of tasks with non-null memory_context)
- **Agent memory accuracy** (% of useful memories vs. noise)

### Alerts to Set

- [ ] data-fabric deployment failed
- [ ] MOM endpoint unreachable (HTTP errors > 5%)
- [ ] Task claiming latency p99 > 2 seconds
- [ ] Database errors (D1 failures)

### Logs to Monitor

```bash
# Watch for errors
bunx wrangler@3 tail --env production | grep -i error

# Track MOM queries
bunx wrangler@3 tail --env production | grep -i "mem\|recall\|mom"
```

---

## Configuration Reference

| Environment | MOM Endpoint | Purpose |
|-------------|--------------|---------|
| Development | mom-service.lornu.com | Local testing, fast feedback |
| Staging | mom-service.stevedores.org | Pre-production validation |
| Production | mom-service.lornu.com | Live agents with full memory |

---

## Troubleshooting Deployments

### Issue: `MOM_ENDPOINT` not set

**Check:**
```bash
bunx wrangler@3 env list
bunx wrangler@3 secret list --env production
```

**Fix:**
```bash
# Update wrangler.toml or use secrets
bunx wrangler@3 secret put MOM_ENDPOINT --env production
```

### Issue: Memory context always null

**Check:**
1. Is `MOM_ENDPOINT` configured? `bunx wrangler@3 tail --env production`
2. Is MOM reachable? `curl $MOM_ENDPOINT/health`
3. Are there memories for the agent?

**Fix:** Verify MOM service is running and healthy.

### Issue: Slow task claiming (> 1 second)

**Check:**
1. MOM latency: `curl -w "@curl-format.txt" $MOM_ENDPOINT/v1/recall`
2. D1 latency: Check database query logs
3. Network latency: Between data-fabric and MOM

**Fix:** Optimize MOM query or add caching layer.

---

## Deployment History

Document deployments here:

| Date | Environment | Version | Status | Notes |
|------|-------------|---------|--------|-------|
| 2026-04-25 | Development | 1.0.0 | ✅ | Initial MOM integration |
| | Staging | | | |
| | Production | | | |

---

## Next Steps After Deployment

1. **Monitor** MOM recall accuracy and agent performance
2. **Measure** improvement in agent task completion
3. **Iterate** on memory consolidation strategies
4. **Plan** Phase 6: Observability dashboard
5. **Prepare** Phase 7: Agent specialization tracking

---

**Ready to deploy!** 🚀

Use: `bunx wrangler@3 deploy --env [development|staging|production]`
