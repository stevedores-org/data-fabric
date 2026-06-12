use crate::{db, generate_id, models, storage, tenant_security};
use serde_json::Value;
use std::future::Future;
use std::pin::Pin;
use worker::{Bucket, D1Database, Error, Result};

pub const TRACE_ARCHIVE_THRESHOLD_BYTES: usize = 1024;
const DEFAULT_MAX_RETRIES: u8 = 3;
const R2_POINTER_PREFIX: &str = "r2://ARTIFACTS/";

static DEFAULT_TRACE_REDACTOR: DefaultTraceRedactor = DefaultTraceRedactor;

pub trait TraceSink {
    fn emit<'a>(
        &'a self,
        tenant_id: &'a str,
        trace: models::CreateReasoningTrace,
    ) -> Pin<Box<dyn Future<Output = Result<models::ReasoningTraceAck>> + 'a>>;
}

pub trait TraceRedactor {
    fn redact(&self, payload: &Value) -> Value;
}

pub struct DefaultTraceRedactor;

impl TraceRedactor for DefaultTraceRedactor {
    fn redact(&self, payload: &Value) -> Value {
        redact_trace_payload(payload)
    }
}

pub struct DataFabricTraceSink<'a> {
    db: &'a D1Database,
    bucket: Option<&'a Bucket>,
    redactor: &'a dyn TraceRedactor,
    max_retries: u8,
}

impl<'a> DataFabricTraceSink<'a> {
    pub fn new(db: &'a D1Database, bucket: Option<&'a Bucket>) -> Self {
        Self {
            db,
            bucket,
            redactor: &DEFAULT_TRACE_REDACTOR,
            max_retries: DEFAULT_MAX_RETRIES,
        }
    }

    async fn persist_once(
        &self,
        tenant_id: &str,
        id: &str,
        idempotency_key: &str,
        explicit_idempotency: bool,
        trace: &models::CreateReasoningTrace,
    ) -> Result<models::ReasoningTraceAck> {
        trace
            .validate()
            .map_err(|err| Error::RustError(format!("invalid reasoning trace: {err}")))?;

        if explicit_idempotency {
            if let Some(existing_id) =
                db::get_reasoning_trace_id_by_key(self.db, tenant_id, idempotency_key).await?
            {
                return Ok(models::ReasoningTraceAck {
                    id: existing_id,
                    accepted: true,
                    duplicate: true,
                    schema_version: trace.schema_version,
                    archived_inputs: false,
                    archived_outputs: false,
                });
            }
        }

        let inputs = self
            .prepare_payload(
                tenant_id,
                &trace.job_id,
                id,
                "inputs",
                trace.inputs.as_ref(),
            )
            .await?;
        let outputs = self
            .prepare_payload(
                tenant_id,
                &trace.job_id,
                id,
                "outputs",
                trace.outputs.as_ref(),
            )
            .await?;

        let record = models::ReasoningTraceRecord {
            id: id.to_string(),
            schema_version: trace.schema_version,
            idempotency_key: idempotency_key.to_string(),
            agent_id: trace.agent_id.trim().to_string(),
            job_id: trace.job_id.trim().to_string(),
            parent_span_id: trace
                .parent_span_id
                .as_ref()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
            step_number: trace.step_number,
            step_type: trace.step_type.trim().to_string(),
            inputs,
            outputs,
            token_cost: trace.token_cost.clone(),
            started_at: trace.started_at.trim().to_string(),
            completed_at: trace.completed_at.trim().to_string(),
            metadata: trace.metadata.as_ref().map(|v| self.redactor.redact(v)),
        };

        let inserted = db::insert_reasoning_trace(self.db, tenant_id, &record).await?;
        Ok(models::ReasoningTraceAck {
            id: record.id,
            accepted: true,
            duplicate: !inserted,
            schema_version: record.schema_version,
            archived_inputs: record.inputs.is_archived(),
            archived_outputs: record.outputs.is_archived(),
        })
    }

    async fn prepare_payload(
        &self,
        tenant_id: &str,
        job_id: &str,
        trace_id: &str,
        payload_name: &str,
        payload: Option<&Value>,
    ) -> Result<models::TracePayloadStorage> {
        let Some(payload) = payload else {
            return Ok(models::TracePayloadStorage::empty());
        };

        let redacted = self.redactor.redact(payload);
        let bytes = serde_json::to_vec(&redacted)
            .map_err(|err| Error::RustError(format!("failed to serialize trace payload: {err}")))?;
        let size_bytes = bytes.len() as u64;

        if bytes.len() <= TRACE_ARCHIVE_THRESHOLD_BYTES {
            return Ok(models::TracePayloadStorage {
                inline: Some(redacted),
                archive_url: None,
                size_bytes,
            });
        }

        let bucket = self.bucket.ok_or_else(|| {
            Error::RustError(
                "ARTIFACTS R2 binding is required for trace payloads over 1 KiB".into(),
            )
        })?;
        let key = trace_archive_key(tenant_id, job_id, trace_id, payload_name);
        storage::put_blob(bucket, &key, bytes).await?;

        Ok(models::TracePayloadStorage {
            inline: None,
            archive_url: Some(format!("{R2_POINTER_PREFIX}{key}")),
            size_bytes,
        })
    }
}

impl<'a> TraceSink for DataFabricTraceSink<'a> {
    fn emit<'b>(
        &'b self,
        tenant_id: &'b str,
        trace: models::CreateReasoningTrace,
    ) -> Pin<Box<dyn Future<Output = Result<models::ReasoningTraceAck>> + 'b>> {
        Box::pin(async move {
            let explicit_idempotency = trace
                .idempotency_key
                .as_ref()
                .map(|s| !s.trim().is_empty())
                .unwrap_or(false);
            let id = if explicit_idempotency {
                stable_trace_id(tenant_id, trace.idempotency_key.as_deref().unwrap_or(""))
            } else {
                generate_id()?
            };
            let idempotency_key = trace
                .idempotency_key
                .as_ref()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| id.clone());

            let mut last_error = None;
            for attempt in 1..=self.max_retries {
                match self
                    .persist_once(
                        tenant_id,
                        &id,
                        &idempotency_key,
                        explicit_idempotency,
                        &trace,
                    )
                    .await
                {
                    Ok(ack) => return Ok(ack),
                    Err(err) => {
                        let message = err.to_string();
                        worker::console_log!(
                            "WARN: reasoning trace sink attempt {}/{} failed: {}",
                            attempt,
                            self.max_retries,
                            message
                        );
                        last_error = Some(message);
                    }
                }
            }

            Err(Error::RustError(format!(
                "reasoning trace sink failed after {} attempts: {}",
                self.max_retries,
                last_error.unwrap_or_else(|| "unknown error".into())
            )))
        })
    }
}

pub fn redact_trace_payload(payload: &Value) -> Value {
    match payload {
        Value::Array(items) => {
            Value::Array(items.iter().map(redact_trace_payload).collect::<Vec<_>>())
        }
        Value::Object(obj) => {
            let mut recursed = serde_json::Map::new();
            for (key, value) in obj {
                recursed.insert(key.clone(), redact_trace_payload(value));
            }
            tenant_security::redact_sensitive_fields(&Value::Object(recursed))
        }
        other => other.clone(),
    }
}

pub fn trace_payload_size(payload: &Value) -> Result<usize> {
    serde_json::to_vec(payload)
        .map(|bytes| bytes.len())
        .map_err(|err| Error::RustError(format!("failed to size trace payload: {err}")))
}

pub fn trace_archive_key(
    tenant_id: &str,
    job_id: &str,
    trace_id: &str,
    payload_name: &str,
) -> String {
    format!(
        "{}/reasoning-traces/{}/{}/{}.json",
        path_segment(tenant_id),
        path_segment(job_id),
        path_segment(trace_id),
        path_segment(payload_name)
    )
}

fn stable_trace_id(tenant_id: &str, idempotency_key: &str) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in tenant_id
        .as_bytes()
        .iter()
        .chain([b':'].iter())
        .chain(idempotency_key.trim().as_bytes().iter())
    {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("trace-{hash:016x}")
}

fn path_segment(raw: &str) -> String {
    let cleaned = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '-'
            }
        })
        .collect::<String>();
    let trimmed = cleaned.trim_matches('-');
    if trimmed.is_empty() {
        "unknown".into()
    } else {
        trimmed.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn redacts_nested_sensitive_fields() {
        let payload = json!({
            "tool": "deploy",
            "auth": {
                "api_key": "sk-test",
                "email": "person@example.com"
            },
            "items": [{"token": "plain"}]
        });

        let redacted = redact_trace_payload(&payload);
        assert_eq!(redacted["tool"], "deploy");
        assert_eq!(redacted["auth"]["api_key"], "***REDACTED***");
        assert_eq!(redacted["auth"]["email"], "pe***");
        assert_eq!(redacted["items"][0]["token"], "***REDACTED***");
    }

    #[test]
    fn archive_key_sanitizes_segments() {
        assert_eq!(
            trace_archive_key("tenant/a", "job b", "trace:c", "inputs"),
            "tenant-a/reasoning-traces/job-b/trace-c/inputs.json"
        );
    }

    #[test]
    fn stable_trace_id_is_deterministic() {
        assert_eq!(
            stable_trace_id("tenant-1", "job-1:step-1"),
            stable_trace_id("tenant-1", "job-1:step-1")
        );
        assert_ne!(
            stable_trace_id("tenant-1", "job-1:step-1"),
            stable_trace_id("tenant-1", "job-1:step-2")
        );
    }

    #[test]
    fn payload_size_crosses_archive_threshold() {
        let small = json!({"value": "x"});
        let large = json!({"value": "x".repeat(TRACE_ARCHIVE_THRESHOLD_BYTES)});

        assert!(trace_payload_size(&small).unwrap() <= TRACE_ARCHIVE_THRESHOLD_BYTES);
        assert!(trace_payload_size(&large).unwrap() > TRACE_ARCHIVE_THRESHOLD_BYTES);
    }
}
