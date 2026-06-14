use crate::db;
use crate::integrations::gemini::{BatchRequest, GenerateContentRequest, Content, Part, GeminiClient};
use wasm_bindgen::JsValue;
use worker::*;

/// Summarize telemetry for a tenant using Gemini Batch API.
#[allow(dead_code)]
pub async fn summarize_telemetry_batch(
    env: &Env,
    tenant_id: &str,
    api_key: &str,
    model: &str,
) -> Result<String> {
    let d1 = env.d1("DB")?;
    
    // 1. Gather telemetry events that need summarization.
    // For now, let's just get the last 1000 events.
    // In a real scenario, we might want to filter by run_id or time range.
    let events = d1.prepare("SELECT event_type, payload FROM graph_events WHERE tenant_id = ?1 ORDER BY created_at DESC LIMIT 1000")
        .bind(&[JsValue::from_str(tenant_id)])?
        .all()
        .await?;
    
    let rows: Vec<serde_json::Value> = events.results()?;
    if rows.is_empty() {
        return Ok("No events to summarize".to_string());
    }

    // 2. Prepare JSONL data.
    let mut jsonl = String::new();
    for (i, row) in rows.into_iter().enumerate() {
        let event_type = row["event_type"].as_str().unwrap_or("unknown");
        let payload = row["payload"].to_string();
        
        let req = BatchRequest {
            key: format!("event_{}_{}", tenant_id, i),
            request: GenerateContentRequest {
                contents: vec![Content {
                    role: Some("user".to_string()),
                    parts: vec![Part {
                        text: Some(format!("Summarize this telemetry event (type: {}): {}", event_type, payload)),
                        inline_data: None,
                        file_data: None,
                        function_call: None,
                        function_response: None,
                    }],
                }],
                system_instruction: Some(Content {
                    role: None,
                    parts: vec![Part {
                        text: Some("You are a telemetry analyst. Provide a concise summary of the event.".to_string()),
                        inline_data: None,
                        file_data: None,
                        function_call: None,
                        function_response: None,
                    }],
                }),
                tools: None,
                tool_config: None,
                safety_settings: None,
                generation_config: None,
            },
        };
        
        jsonl.push_str(&serde_json::to_string(&req).unwrap());
        jsonl.push('\n');
    }

    // 3. Upload to Gemini.
    let client = GeminiClient::new(api_key.to_string());
    let input_file_uri = client.upload_file(
        jsonl.into_bytes(),
        "application/jsonl",
        Some(&format!("telemetry_summary_{}.jsonl", tenant_id))
    ).await.map_err(|e| Error::RustError(e))?;

    // 4. Start Batch Job.
    let job = client.create_batch_job(
        model,
        &input_file_uri,
        Some(&format!("Telemetry Summary for {}", tenant_id))
    ).await.map_err(|e| Error::RustError(e))?;

    // 5. Track Job in D1.
    let mut random_bytes = [0u8; 8];
    getrandom::getrandom(&mut random_bytes).map_err(|e| Error::RustError(e.to_string()))?;
    let job_id = format!("gb-{}", hex::encode(random_bytes));
    
    db::create_gemini_batch_job(
        &d1,
        tenant_id,
        &job_id,
        &job,
        model,
        &input_file_uri
    ).await?;

    Ok(job.name)
}

/// Poll all pending Gemini batch jobs and update their status.
pub async fn poll_gemini_jobs(env: &Env) -> Result<()> {
    let d1 = env.d1("DB")?;
    let pending_jobs = db::list_pending_gemini_batch_jobs(&d1).await?;
    
    if pending_jobs.is_empty() {
        return Ok(());
    }

    // We need the API key. In a real app, it would be in env secrets.
    let api_key = env.secret("GEMINI_API_KEY")?.to_string();
    let client = GeminiClient::new(api_key);

    for (tenant_id, job_name, _model) in pending_jobs {
        match client.get_batch_job(&job_name).await {
            Ok(job) => {
                db::update_gemini_batch_job(&d1, &tenant_id, &job).await?;
                
                if job.state == crate::integrations::gemini::BatchJobState::Succeeded {
                    // TODO: Retrieve results and process them (e.g. save summaries to gold layer)
                    console_log!("Gemini Batch Job {} succeeded for tenant {}", job_name, tenant_id);
                }
            }
            Err(e) => {
                console_error!("Failed to poll Gemini job {}: {}", job_name, e);
            }
        }
    }

    Ok(())
}
