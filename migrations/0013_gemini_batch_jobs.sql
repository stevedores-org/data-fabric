-- Migration: 0013_gemini_batch_jobs.sql
-- Description: Track Gemini Batch Jobs for large-scale processing

CREATE TABLE IF NOT EXISTS gemini_batch_jobs (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    job_name TEXT NOT NULL, -- The "name" returned by Gemini API (e.g. "batchJobs/123")
    display_name TEXT,
    model TEXT NOT NULL,
    status TEXT NOT NULL, -- PENDING, RUNNING, SUCCEEDED, FAILED, CANCELLED
    input_file TEXT NOT NULL, -- URI of the input file in Gemini File API
    output_file TEXT, -- URI of the output file in Gemini File API
    error_json TEXT,
    created_at TEXT NOT NULL,
    completed_at TEXT
);

CREATE INDEX IF NOT EXISTS idx_gemini_batch_jobs_tenant ON gemini_batch_jobs(tenant_id);
CREATE INDEX IF NOT EXISTS idx_gemini_batch_jobs_status ON gemini_batch_jobs(status);
