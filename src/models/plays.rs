use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
pub struct PlayDefinition {
    pub name: String,
    pub goal: String,
    pub tasks: Vec<PlayTaskDefinition>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
pub struct PlayTaskDefinition {
    pub id: String,
    pub task_type: String,
    pub priority: i32,
    pub params: Option<serde_json::Value>,
    pub depends_on: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct PlayLaunchRequest {
    pub play_name: String,
    pub job_id: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[allow(dead_code)]
pub struct PlayLaunchResponse {
    pub run_id: String,
    pub status: String,
}
