use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use data_fabric_client::{
    types::{CreateCheckpoint, CreateRun, PolicyCheckRequest},
    Client, ClientConfig,
};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "dfctl")]
#[command(
    about = "Command-line control interface for Stevedores Data Fabric",
    long_about = None,
)]
struct Cli {
    /// Base URL of the Data Fabric service.
    #[arg(long, env = "DATA_FABRIC_URL", default_value = "http://localhost:8787")]
    url: String,

    /// Tenant identifier (sent as `x-tenant-id`).
    #[arg(long, env = "DATA_FABRIC_TENANT_ID", default_value = "lornu-ai")]
    tenant_id: String,

    /// Tenant role (sent as `x-tenant-role`).
    #[arg(long, env = "DATA_FABRIC_TENANT_ROLE", default_value = "builder")]
    tenant_role: String,

    /// Cloudflare Access service token client id.
    #[arg(long, env = "CF_ACCESS_CLIENT_ID")]
    cf_client_id: Option<String>,

    /// Cloudflare Access service token client secret.
    #[arg(long, env = "CF_ACCESS_CLIENT_SECRET")]
    cf_client_secret: Option<String>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Check health of the Data Fabric service.
    Health,

    /// Manage execution runs.
    Runs {
        #[command(subcommand)]
        cmd: RunCommands,
    },

    /// Manage state checkpoints.
    Checkpoints {
        #[command(subcommand)]
        cmd: CheckpointCommands,
    },

    /// Evaluate policy rules.
    Policy {
        #[command(subcommand)]
        cmd: PolicyCommands,
    },

    /// Manage agents.
    Agents {
        #[command(subcommand)]
        cmd: AgentCommands,
    },
}

#[derive(Subcommand)]
enum RunCommands {
    /// List runs for the tenant.
    List {
        #[arg(long)]
        repo: Option<String>,
        #[arg(long, default_value_t = 20)]
        limit: usize,
        #[arg(long)]
        cursor: Option<String>,
    },
    /// Create a new execution run.
    Create {
        #[arg(long)]
        repo: String,
        #[arg(long)]
        trigger: Option<String>,
        #[arg(long)]
        actor: Option<String>,
    },
    /// Inspect a run by id.
    Inspect { run_id: String },
}

#[derive(Subcommand)]
enum CheckpointCommands {
    /// Save a checkpoint for a thread/node.
    ///
    /// The Data Fabric checkpoint API is keyed on `thread_id` + `node_id`, not
    /// on `run_id`. Callers wanting to associate a checkpoint with a run should
    /// pass the run id as the thread id (the orchestrator's convention).
    Save {
        /// Thread the checkpoint belongs to.
        #[arg(long)]
        thread_id: String,
        /// Node within the graph that produced this state.
        #[arg(long)]
        node_id: String,
        /// Optional parent checkpoint id.
        #[arg(long)]
        parent_id: Option<String>,
        /// Path to a JSON file containing the state.
        state_file: PathBuf,
    },
    /// Get details of a checkpoint.
    Get { id: String },
    /// Delete a checkpoint.
    Delete { id: String },
}

#[derive(Subcommand)]
enum PolicyCommands {
    /// Evaluate a policy request.
    Check {
        #[arg(long)]
        action: String,
        #[arg(long)]
        actor: String,
        #[arg(long)]
        resource: Option<String>,
        #[arg(long)]
        run_id: Option<String>,
    },
}

#[derive(Subcommand)]
enum AgentCommands {
    /// List registered agents.
    List,
}

fn build_client(cli: &Cli) -> Client {
    let config = ClientConfig {
        base_url: cli.url.clone(),
        tenant_id: cli.tenant_id.clone(),
        tenant_role: cli.tenant_role.clone(),
        cf_client_id: cli.cf_client_id.clone(),
        cf_client_secret: cli.cf_client_secret.clone(),
    };
    Client::new(config)
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let client = build_client(&cli);

    match cli.command {
        Commands::Health => {
            let res = client.health().await.context("Failed to check health")?;
            println!("Service: {}", res.service);
            println!("Status:  {}", res.status);
            println!("Mission: {}", res.mission);
        }

        Commands::Runs { cmd } => match cmd {
            RunCommands::List {
                repo,
                limit,
                cursor,
            } => {
                let res = client
                    .list_runs(repo.as_deref(), Some(limit), cursor.as_deref())
                    .await
                    .context("Failed to list runs")?;
                print_runs(&res)?;
            }
            RunCommands::Create {
                repo,
                trigger,
                actor,
            } => {
                let req = CreateRun {
                    repo,
                    trigger,
                    actor,
                    metadata: None,
                };
                let created = client
                    .create_run(&req)
                    .await
                    .context("Failed to create run")?;
                println!("Run created successfully!");
                println!("ID:     {}", created.id);
                println!("Status: {}", created.status);
            }
            RunCommands::Inspect { run_id } => {
                let run = client
                    .get_run(&run_id)
                    .await
                    .context("Failed to fetch run")?;
                println!("ID:         {}", run.id);
                println!("Repository: {}", run.repo);
                println!("Status:     {:?}", run.status);
                println!("Trigger:    {}", run.trigger.as_deref().unwrap_or("-"));
                println!("Actor:      {}", run.actor);
                println!("Created:    {}", run.created_at);
                println!("Updated:    {}", run.updated_at);
                if let Some(meta) = run.metadata {
                    println!("Metadata:\n{}", serde_json::to_string_pretty(&meta)?);
                }
            }
        },

        Commands::Checkpoints { cmd } => match cmd {
            CheckpointCommands::Save {
                thread_id,
                node_id,
                parent_id,
                state_file,
            } => {
                let content =
                    std::fs::read_to_string(&state_file).context("Failed to read state file")?;
                let state: serde_json::Value =
                    serde_json::from_str(&content).context("Invalid JSON in state file")?;
                let req = CreateCheckpoint {
                    thread_id,
                    node_id,
                    parent_id,
                    state,
                    metadata: None,
                };
                let res = client
                    .create_checkpoint(&req)
                    .await
                    .context("Failed to create checkpoint")?;
                println!("Checkpoint created successfully!");
                println!("ID:           {}", res.id);
                println!("Thread:       {}", res.thread_id);
                println!("State R2 key: {}", res.state_r2_key);
            }
            CheckpointCommands::Get { id } => {
                let res = client
                    .get_checkpoint(&id)
                    .await
                    .context("Failed to fetch checkpoint")?;
                println!("{}", serde_json::to_string_pretty(&res)?);
            }
            CheckpointCommands::Delete { id } => {
                client
                    .delete_checkpoint(&id)
                    .await
                    .context("Failed to delete checkpoint")?;
                println!("Checkpoint {} deleted.", id);
            }
        },

        Commands::Policy { cmd } => match cmd {
            PolicyCommands::Check {
                action,
                actor,
                resource,
                run_id,
            } => {
                let req = PolicyCheckRequest {
                    action,
                    actor,
                    resource,
                    context: None,
                    run_id,
                };
                let res = client
                    .check_policy(&req)
                    .await
                    .context("Failed to check policy")?;
                println!("--- Policy Evaluation ---");
                println!("Decision: {}", res.decision);
                println!("Reason:   {}", res.reason);
                if let Some(level) = res.risk_level {
                    println!("Risk:     {}", level);
                }
            }
        },

        Commands::Agents { cmd } => match cmd {
            AgentCommands::List => {
                let res = client
                    .list_agents()
                    .await
                    .context("Failed to list agents")?;
                println!("{}", serde_json::to_string_pretty(&res)?);
            }
        },
    }

    Ok(())
}

fn print_runs(res: &serde_json::Value) -> Result<()> {
    use cli_table::{print_stdout, Cell, Style, Table};

    // The list endpoint returns an opaque JSON document. Look for the most
    // common shapes (`{ runs: [...] }`, `{ items: [...] }`, or a bare array).
    let runs: Vec<&serde_json::Value> = if let Some(arr) = res.as_array() {
        arr.iter().collect()
    } else if let Some(arr) = res.get("runs").and_then(|v| v.as_array()) {
        arr.iter().collect()
    } else if let Some(arr) = res.get("items").and_then(|v| v.as_array()) {
        arr.iter().collect()
    } else {
        // Unknown shape — fall back to pretty-printing the raw JSON.
        println!("{}", serde_json::to_string_pretty(res)?);
        return Ok(());
    };

    let mut rows = Vec::new();
    for run in runs {
        let id = run
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("-")
            .to_string();
        let repo = run
            .get("repo")
            .and_then(|v| v.as_str())
            .unwrap_or("-")
            .to_string();
        let status = run
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("-")
            .to_string();
        let trigger = run
            .get("trigger")
            .and_then(|v| v.as_str())
            .unwrap_or("-")
            .to_string();
        let actor = run
            .get("actor")
            .and_then(|v| v.as_str())
            .unwrap_or("-")
            .to_string();
        let created_at = run
            .get("created_at")
            .and_then(|v| v.as_str())
            .unwrap_or("-")
            .to_string();
        rows.push(vec![
            id.cell(),
            repo.cell(),
            status.cell(),
            trigger.cell(),
            actor.cell(),
            created_at.cell(),
        ]);
    }

    let table = rows.table().title(vec![
        "Run ID".cell().bold(true),
        "Repository".cell().bold(true),
        "Status".cell().bold(true),
        "Trigger".cell().bold(true),
        "Actor".cell().bold(true),
        "Created At".cell().bold(true),
    ]);
    print_stdout(table)?;

    if let Some(cursor) = res
        .get("next_cursor")
        .and_then(|v| v.as_str())
        .or_else(|| res.get("cursor").and_then(|v| v.as_str()))
    {
        println!("Next cursor: {}", cursor);
    }
    Ok(())
}
