#[cfg(test)]
mod tests {
    use super::get_openapi_spec;

    fn parse_spec() -> serde_json::Value {
        serde_json::from_str(get_openapi_spec()).expect("openapi spec must be valid JSON")
    }

    /// The POST handler for `/v1/policies/check` returns 9 distinct fields.
    /// The OpenAPI response schema must enumerate every one with the right
    /// JSON type, or generated clients will silently strip data.
    #[test]
    fn openapi_documents_all_nine_policy_check_response_fields() {
        let spec = parse_spec();
        let props = &spec["paths"]["/v1/policies/check"]["post"]["responses"]["200"]["content"]
            ["application/json"]["schema"]["properties"];
        let expected = [
            ("id", "string"),
            ("action", "string"),
            ("decision", "string"),
            ("reason", "string"),
            ("risk_level", "string"),
            ("policy_version", "string"),
            ("matched_rule", "string"),
            ("escalation_id", "string"),
            ("rate_limited", "boolean"),
        ];
        for (name, ty) in expected {
            assert_eq!(
                props[name]["type"].as_str(),
                Some(ty),
                "/v1/policies/check response must document field `{name}` as type `{ty}`",
            );
        }
        let actual_count = props.as_object().map(|o| o.len()).unwrap_or(0);
        assert_eq!(
            actual_count, 9,
            "/v1/policies/check response must document exactly 9 fields, got {actual_count}",
        );
    }

    /// `/v1/checkpoints` POST response: `{id, thread_id, state_r2_key}`. The
    /// previous schema documented `{id, status}`, which is wrong and breaks
    /// any client that relies on `state_r2_key` for R2 recovery.
    #[test]
    fn openapi_documents_checkpoint_post_response_shape() {
        let spec = parse_spec();
        let props = &spec["paths"]["/v1/checkpoints"]["post"]["responses"]["200"]["content"]
            ["application/json"]["schema"]["properties"];
        for required in ["id", "thread_id", "state_r2_key"] {
            assert_eq!(
                props[required]["type"].as_str(),
                Some("string"),
                "/v1/checkpoints response must document field `{required}` as string",
            );
        }
        assert!(
            props["status"].is_null(),
            "/v1/checkpoints response must NOT document a `status` field (legacy drift)",
        );
        let required = &spec["paths"]["/v1/checkpoints"]["post"]["responses"]["200"]["content"]
            ["application/json"]["schema"]["required"];
        let required_set: Vec<&str> = required
            .as_array()
            .expect("required must be an array")
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert!(required_set.contains(&"state_r2_key"));
        assert!(required_set.contains(&"thread_id"));
        assert!(required_set.contains(&"id"));
    }

    /// `/mcp/task/next` is documented under POST. GET is documented as
    /// deprecated so generated clients warn / log. Confirms HTTP method
    /// semantics are correct end-to-end (route + spec).
    #[test]
    fn openapi_documents_mcp_task_next_as_post_with_deprecated_get() {
        let spec = parse_spec();
        let path = &spec["paths"]["/mcp/task/next"];
        assert!(
            path["post"].is_object(),
            "/mcp/task/next must be documented as POST (stateful claim operation)"
        );
        // GET kept as a deprecated compatibility shim.
        let get_op = &path["get"];
        assert!(
            get_op.is_object(),
            "/mcp/task/next GET must remain documented as a deprecation shim"
        );
        assert_eq!(
            get_op["deprecated"].as_bool(),
            Some(true),
            "/mcp/task/next GET must be marked deprecated"
        );
        assert!(
            get_op["responses"]["405"].is_object(),
            "/mcp/task/next GET must document its 405 deprecation response"
        );
        // POST must document the required agent_id query parameter so
        // generated clients carry it forward.
        let params = path["post"]["parameters"]
            .as_array()
            .expect("POST must declare query parameters");
        let agent_id_param = params
            .iter()
            .find(|p| p["name"].as_str() == Some("agent_id"))
            .expect("POST /mcp/task/next must document `agent_id` parameter");
        assert_eq!(agent_id_param["required"].as_bool(), Some(true));
        assert_eq!(agent_id_param["in"].as_str(), Some("query"));
    }
}

pub fn get_openapi_spec() -> &'static str {
    r#"{
  "openapi": "3.1.0",
  "info": {
    "title": "Data Fabric API",
    "version": "1.0.0",
    "description": "Cloudflare-native data fabric for autonomous AI agent builder orchestration, logging, auditing, policy checking, and state checkpointing."
  },
  "servers": [
    {
      "url": "/"
    }
  ],
  "paths": {
    "/health": {
      "get": {
        "summary": "Health Check",
        "description": "Returns the status of the Data Fabric service.",
        "responses": {
          "200": {
            "description": "OK",
            "content": {
              "application/json": {
                "schema": {
                  "type": "object",
                  "properties": {
                    "service": { "type": "string", "example": "data-fabric" },
                    "status": { "type": "string", "example": "ok" },
                    "mission": { "type": "string", "example": "velocity-for-autonomous-agent-builders" }
                  }
                }
              }
            }
          }
        }
      }
    },
    "/v1/tenants/provision": {
      "post": {
        "summary": "Provision Tenant",
        "requestBody": {
          "required": true,
          "content": {
            "application/json": {
              "schema": {
                "type": "object",
                "required": ["tenant_id", "display_name"],
                "properties": {
                  "tenant_id": { "type": "string", "example": "lornu-ai" },
                  "display_name": { "type": "string", "example": "Lornu AI" }
                }
              }
            }
          }
        },
        "responses": {
          "200": {
            "description": "Tenant provisioned successfully",
            "content": {
              "application/json": {
                "schema": {
                  "type": "object",
                  "properties": {
                    "tenant_id": { "type": "string" },
                    "status": { "type": "string" },
                    "provisioned_in_ms": { "type": "integer" }
                  }
                }
              }
            }
          }
        }
      }
    },
    "/v1/runs": {
      "post": {
        "summary": "Create Run",
        "requestBody": {
          "required": true,
          "content": {
            "application/json": {
              "schema": {
                "type": "object",
                "required": ["repo"],
                "properties": {
                  "repo": { "type": "string", "example": "stevedores-org/ogre" },
                  "trigger": { "type": "string", "example": "webhook" },
                  "actor": { "type": "string", "example": "user" },
                  "metadata": { "type": "object" }
                }
              }
            }
          }
        },
        "responses": {
          "200": {
            "description": "Run created successfully",
            "content": {
              "application/json": {
                "schema": {
                  "type": "object",
                  "properties": {
                    "id": { "type": "string" },
                    "status": { "type": "string" }
                  }
                }
              }
            }
          }
        }
      },
      "get": {
        "summary": "List Runs",
        "parameters": [
          {
            "name": "repo",
            "in": "query",
            "schema": { "type": "string" }
          },
          {
            "name": "limit",
            "in": "query",
            "schema": { "type": "integer", "default": 20 }
          },
          {
            "name": "cursor",
            "in": "query",
            "schema": { "type": "string" }
          }
        ],
        "responses": {
          "200": {
            "description": "List of runs with next cursor for pagination",
            "content": {
              "application/json": {
                "schema": {
                  "type": "object",
                  "properties": {
                    "runs": { "type": "array", "items": { "type": "object" } },
                    "next_cursor": { "type": "string" }
                  }
                }
              }
            }
          }
        }
      }
    },
    "/v1/checkpoints": {
      "post": {
        "summary": "Save Checkpoint",
        "description": "Persists thread state to R2 and registers a row in D1. The returned state_r2_key is required for downstream R2 recovery and integrity verification.",
        "requestBody": {
          "required": true,
          "content": {
            "application/json": {
              "schema": {
                "type": "object",
                "required": ["thread_id", "node_id", "state"],
                "properties": {
                  "thread_id": { "type": "string" },
                  "node_id": { "type": "string" },
                  "parent_id": { "type": "string" },
                  "state": { "type": "object" },
                  "metadata": { "type": "object" }
                }
              }
            }
          }
        },
        "responses": {
          "200": {
            "description": "Checkpoint saved",
            "content": {
              "application/json": {
                "schema": {
                  "type": "object",
                  "required": ["id", "thread_id", "state_r2_key"],
                  "properties": {
                    "id": {
                      "type": "string",
                      "description": "Unique checkpoint identifier."
                    },
                    "thread_id": {
                      "type": "string",
                      "description": "Thread the checkpoint belongs to (echoed from request)."
                    },
                    "state_r2_key": {
                      "type": "string",
                      "description": "R2 object key where the serialized state was written; required for R2 recovery."
                    }
                  }
                }
              }
            }
          }
        }
      }
    },
    "/v1/checkpoints/{id}": {
      "get": {
        "summary": "Get Checkpoint",
        "parameters": [
          {
            "name": "id",
            "in": "path",
            "required": true,
            "schema": { "type": "string" }
          }
        ],
        "responses": {
          "200": {
            "description": "Checkpoint details",
            "content": {
              "application/json": {
                "schema": {
                  "type": "object"
                }
              }
            }
          }
        }
      },
      "delete": {
        "summary": "Delete Checkpoint",
        "parameters": [
          {
            "name": "id",
            "in": "path",
            "required": true,
            "schema": { "type": "string" }
          }
        ],
        "responses": {
          "200": {
            "description": "Checkpoint deleted"
          }
        }
      }
    },
    "/v1/artifacts/{key}": {
      "put": {
        "summary": "Upload Artifact",
        "parameters": [
          {
            "name": "key",
            "in": "path",
            "required": true,
            "schema": { "type": "string" }
          }
        ],
        "requestBody": {
          "required": true,
          "content": {
            "application/octet-stream": {
              "schema": { "type": "string", "format": "binary" }
            }
          }
        },
        "responses": {
          "200": {
            "description": "Artifact uploaded"
          }
        }
      },
      "get": {
        "summary": "Download Artifact",
        "parameters": [
          {
            "name": "key",
            "in": "path",
            "required": true,
            "schema": { "type": "string" }
          }
        ],
        "responses": {
          "200": {
            "description": "Artifact binary data",
            "content": {
              "application/octet-stream": {}
            }
          }
        }
      }
    },
    "/v1/policies/check": {
      "post": {
        "summary": "Check Policy Decision",
        "requestBody": {
          "required": true,
          "content": {
            "application/json": {
              "schema": {
                "type": "object",
                "required": ["action", "actor"],
                "properties": {
                  "action": { "type": "string", "example": "file_write" },
                  "actor": { "type": "string", "example": "ogre-builder" },
                  "resource": { "type": "string" },
                  "context": { "type": "object" },
                  "run_id": { "type": "string" }
                }
              }
            }
          }
        },
        "responses": {
          "200": {
            "description": "Policy check result. Fields beyond the four required ones are emitted when the policy engine produces them; clients should treat them as optional but stable.",
            "content": {
              "application/json": {
                "schema": {
                  "type": "object",
                  "required": ["id", "action", "decision", "reason"],
                  "properties": {
                    "id": {
                      "type": "string",
                      "description": "Stable policy-decision identifier (audit row id)."
                    },
                    "action": {
                      "type": "string",
                      "description": "Echo of the action evaluated."
                    },
                    "decision": {
                      "type": "string",
                      "description": "Verdict: allow, deny, or escalate.",
                      "example": "allow"
                    },
                    "reason": {
                      "type": "string",
                      "description": "Human-readable rationale for the verdict."
                    },
                    "risk_level": {
                      "type": "string",
                      "description": "Lower-cased risk classification: read, write, destructive, or irreversible.",
                      "example": "write"
                    },
                    "policy_version": {
                      "type": "string",
                      "description": "Identifier of the policy bundle version that produced this decision."
                    },
                    "matched_rule": {
                      "type": "string",
                      "description": "Identifier of the specific rule that matched, when one did."
                    },
                    "escalation_id": {
                      "type": "string",
                      "description": "Set when decision is escalate; identifies the escalation record opened for human review."
                    },
                    "rate_limited": {
                      "type": "boolean",
                      "description": "True when the decision was influenced by a per-tenant or per-actor rate limit."
                    }
                  }
                }
              }
            }
          }
        }
      }
    },
    "/mcp/task/next": {
      "post": {
        "summary": "Claim Next Agent Task",
        "description": "Atomically claims the next pending task for the given agent and capabilities. This is a state-mutating operation: each successful response transfers ownership of a task to the caller. GET is deprecated and will be removed in a future release; existing GET clients receive 405 Method Not Allowed with `Allow: POST`.",
        "parameters": [
          {
            "name": "agent_id",
            "in": "query",
            "required": true,
            "description": "Identifier of the agent claiming the task.",
            "schema": { "type": "string" }
          },
          {
            "name": "cap",
            "in": "query",
            "required": false,
            "description": "Comma-separated list of capabilities the agent supports; used to filter eligible tasks.",
            "schema": { "type": "string" }
          }
        ],
        "responses": {
          "200": {
            "description": "Task successfully claimed. Returns the full AgentTask record; memory_context is populated when MOM is configured and relevant memories are found.",
            "content": {
              "application/json": {
                "schema": {
                  "type": "object",
                  "required": [
                    "id",
                    "job_id",
                    "task_type",
                    "priority",
                    "status",
                    "retry_count",
                    "max_retries",
                    "created_at"
                  ],
                  "properties": {
                    "id": { "type": "string" },
                    "job_id": { "type": "string" },
                    "task_type": { "type": "string" },
                    "priority": { "type": "integer", "format": "int32" },
                    "status": { "type": "string" },
                    "params": { "type": "object" },
                    "result": { "type": "object" },
                    "agent_id": { "type": "string" },
                    "graph_ref": { "type": "string" },
                    "play_id": { "type": "string" },
                    "parent_task_id": { "type": "string" },
                    "retry_count": { "type": "integer", "format": "int32" },
                    "max_retries": { "type": "integer", "format": "int32" },
                    "lease_expires_at": { "type": "string", "format": "date-time" },
                    "created_at": { "type": "string", "format": "date-time" },
                    "completed_at": { "type": "string", "format": "date-time" },
                    "memory_context": {
                      "type": "string",
                      "description": "Optional MOM-augmented memory context for the agent."
                    }
                  }
                }
              }
            }
          },
          "204": {
            "description": "No eligible tasks available for this agent and capability set."
          },
          "400": {
            "description": "Missing required query parameter (agent_id)."
          }
        }
      },
      "get": {
        "deprecated": true,
        "summary": "Claim Next Agent Task (deprecated)",
        "description": "Deprecated. GET semantics are unsafe for this stateful claim operation because caching proxies may cache claimed state and retry middleware can double-claim. Use POST instead. This endpoint currently returns 405 Method Not Allowed with `Allow: POST`; it will be removed in a future release.",
        "responses": {
          "405": {
            "description": "Method Not Allowed. The `Allow` header advertises the supported method (POST).",
            "headers": {
              "Allow": {
                "schema": { "type": "string", "example": "POST" }
              },
              "Deprecation": {
                "schema": { "type": "string", "example": "true" }
              }
            }
          }
        }
      }
    }
  }
}"#
}
