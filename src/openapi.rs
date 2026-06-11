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
        "requestBody": {
          "required": true,
          "content": {
            "application/json": {
              "schema": {
                "type": "object",
                "required": ["run_id", "state"],
                "properties": {
                  "run_id": { "type": "string" },
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
                  "properties": {
                    "id": { "type": "string" },
                    "status": { "type": "string" }
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
            "description": "Policy check result",
            "content": {
              "application/json": {
                "schema": {
                  "type": "object",
                  "properties": {
                    "id": { "type": "string" },
                    "action": { "type": "string" },
                    "decision": { "type": "string", "example": "approved" },
                    "reason": { "type": "string" }
                  }
                }
              }
            }
          }
        }
      }
    }
  }
}"#
}
