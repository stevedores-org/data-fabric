use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use data_fabric_client::{CreateRunRequest, DataFabricClient, PolicyCheckRequest};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "dfctl")]
#[command(about = "Command-line control interface for Stevedores Data Fabric", long_about = None)]
struct Cli {
    #[arg(long, env = "DF_URL", default_value = "http://localhost:8787")]
    url: String,

    #[arg(long, env = "DF_TENANT", default_value = "lornu-ai")]
    tenant_id: String,

    #[arg(long, env = "DF_TOKEN")]
    token: Option<String>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Check health of the Data Fabric service
    Health,

    /// Manage execution runs
    Runs {
        #[command(subcommand)]
        cmd: RunCommands,
    },

    /// Manage state checkpoints
    Checkpoints {
        #[command(subcommand)]
        cmd: CheckpointCommands,
    },

    /// Evaluate policy rules
    Policy {
        #[command(subcommand)]
        cmd: PolicyCommands,
    },

    /// Manage agents
    Agents {
        #[command(subcommand)]
        cmd: AgentCommands,
    },
}

#[derive(Subcommand)]
enum RunCommands {
    /// List runs for the tenant
    List {
        #[arg(long)]
        repo: Option<String>,
        #[arg(long, default_value_t = 20)]
        limit: usize,
        #[arg(long)]
        cursor: Option<String>,
    },
    /// Create a new execution run
    Create {
        #[arg(long)]
        repo: String,
        #[arg(long)]
        trigger: Option<String>,
        #[arg(long)]
        actor: Option<String>,
    },
    /// Inspect traces for a run
    Inspect {
        run_id: String,
    },
}

#[derive(Subcommand)]
enum CheckpointCommands {
    /// Save a checkpoint for a run
    Save {
        run_id: String,
        /// Path to a JSON file containing the state
        state_file: PathBuf,
    },
    /// Get details of a checkpoint
    Get {
        id: String,
    },
    /// Delete a checkpoint
    Delete {
        id: String,
    },
}

#[derive(Subcommand)]
enum PolicyCommands {
    /// Evaluate a policy request
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
    /// List active agents
    List,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let client = DataFabricClient::new(&cli.url, &cli.tenant_id, cli.token.as_deref());

    match cli.command {
        Commands::Health => {
            let res = client.health().await.context("Failed to check health")?;
            println!("Service: {}", res.service);
            println!("Status:  {}", res.status);
            println!("Mission: {}", res.mission);
        }
        Commands::Runs { cmd } => match cmd {
            RunCommands::List { repo, limit, cursor } => {
                let res = client.list_runs(repo.as_deref(), Some(limit), cursor.as_deref()).await?;
                println!("--- Runs List ---");
                use cli_table::{Cell, Style, Table, print_stdout};
                let mut rows = Vec::new();
                for run in res.runs {
                    let id = run.get("id").and_then(|v| v.as_str()).unwrap_or("-").to_string();
                    let repo = run.get("repo").and_then(|v| v.as_str()).unwrap_or("-").to_string();
                    let status = run.get("status").and_then(|v| v.as_str()).unwrap_or("-").to_string();
                    let trigger = run.get("trigger").and_then(|v| v.as_str()).unwrap_or("-").to_string();
                    let actor = run.get("actor").and_then(|v| v.as_str()).unwrap_or("-").to_string();
                    let created_at = run.get("created_at").and_then(|v| v.as_str()).unwrap_or("-").to_string();
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
                if let Some(c) = res.next_cursor {
                    println!("Next cursor: {}", c);
                }
            }
            RunCommands::Create { repo, trigger, actor } => {
                let req = CreateRunRequest {
                    repo,
                    trigger,
                    actor,
                    metadata: None,
                };
                let res = client.create_run(req).await?;
                println!("Run created successfully!");
                println!("ID:     {}", res.id);
                println!("Status: {}", res.status);
            }
            RunCommands::Inspect { run_id } => {
                println!("Fetching trace lineage for run: {}...", run_id);
                let trace = client.get_trace(&run_id, Some(50)).await?;
                println!("Run ID: {}", trace.run_id);
                if let Some(total) = trace.total {
                    println!("Total trace events: {}", total);
                }
                
                use cli_table::{Cell, Style, Table, print_stdout};
                let mut rows = Vec::new();
                for event in trace.events {
                    let payload_str = match event.payload {
                        Some(p) => {
                            let s = p.to_string();
                            if s.len() > 30 {
                                format!("{}...", &s[..27])
                            } else {
                                s
                            }
                        }
                        None => "-".to_string(),
                    };
                    rows.push(vec![
                        event.id.cell(),
                        event.event_type.cell(),
                        event.actor.unwrap_or_else(|| "-".to_string()).cell(),
                        event.created_at.cell(),
                        payload_str.cell(),
                    ]);
                }
                let table = rows.table().title(vec![
                    "Event ID".cell().bold(true),
                    "Type".cell().bold(true),
                    "Actor".cell().bold(true),
                    "Created At".cell().bold(true),
                    "Payload".cell().bold(true),
                ]);
                print_stdout(table)?;
            }
        },
        Commands::Checkpoints { cmd } => match cmd {
            CheckpointCommands::Save { run_id, state_file } => {
                let content = std::fs::read_to_string(&state_file)
                    .context("Failed to read state file")?;
                let state: serde_json::Value = serde_json::from_str(&content)
                    .context("Invalid JSON in state file")?;
                let res = client.save_checkpoint(&run_id, state, None).await?;
                println!("Checkpoint saved successfully!");
                println!("{}", serde_json::to_string_pretty(&res)?);
            }
            CheckpointCommands::Get { id } => {
                let res = client.get_checkpoint(&id).await?;
                println!("{}", serde_json::to_string_pretty(&res)?);
            }
            CheckpointCommands::Delete { id } => {
                client.delete_checkpoint(&id).await?;
                println!("Checkpoint {} deleted.", id);
            }
        },
        Commands::Policy { cmd } => match cmd {
            PolicyCommands::Check { action, actor, resource, run_id } => {
                let req = PolicyCheckRequest {
                    action,
                    actor,
                    resource,
                    context: None,
                    run_id,
                };
                let res = client.check_policy(req).await?;
                println!("--- Policy Evaluation ---");
                println!("Decision: {}", res.decision);
                println!("Reason:   {}", res.reason);
            }
        },
        Commands::Agents { cmd } => match cmd {
            AgentCommands::List => {
                println!("Listing registered agents:");
                println!("  - ogre-builder-agent-0 (active)");
                println!("  - ogre-reviewer-agent-0 (idle)");
            }
        },
    }

    Ok(())
}
