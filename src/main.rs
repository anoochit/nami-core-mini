mod agent;
mod api_client;
mod cli;
mod compaction;
mod config;
mod goal;
mod grill;
mod mcp;
mod media;
mod memory;
mod plan;
mod server;
mod tools;
mod workspace;

use agent::build_nami_agent;
use api_client::NamiApiClient;
use config::NamiConfig;
use memory::NamiMemory;
use server::NamiServer;
use workspace::WorkspaceGuard;

#[derive(Debug, PartialEq)]
pub enum Command {
    Chat,
    Serve,
    Ask(String),
    Help,
    Version,
}

#[derive(Debug)]
pub struct CliArgs {
    pub command: Command,
    pub port: Option<u16>,
    pub url: Option<String>,
}

pub fn parse_cli_args<I, T>(args: I) -> anyhow::Result<CliArgs>
where
    I: IntoIterator<Item = T>,
    T: AsRef<str>,
{
    let args_vec: Vec<String> = args.into_iter().map(|s| s.as_ref().to_string()).collect();
    let mut port = None;
    let mut url = None;
    let mut subcommand = None;
    let mut ask_text = None;

    let mut i = 1;
    while i < args_vec.len() {
        let arg = &args_vec[i];
        if arg == "--port" {
            i += 1;
            if i >= args_vec.len() {
                anyhow::bail!("Missing value for --port");
            }
            let p: u16 = args_vec[i]
                .parse()
                .map_err(|_| anyhow::anyhow!("Invalid port number: {}", args_vec[i]))?;
            port = Some(p);
        } else if let Some(p_str) = arg.strip_prefix("--port=") {
            let p: u16 = p_str
                .parse()
                .map_err(|_| anyhow::anyhow!("Invalid port number: {}", p_str))?;
            port = Some(p);
        } else if arg == "--url" {
            i += 1;
            if i >= args_vec.len() {
                anyhow::bail!("Missing value for --url");
            }
            url = Some(args_vec[i].clone());
        } else if let Some(u_str) = arg.strip_prefix("--url=") {
            url = Some(u_str.to_string());
        } else if arg == "--help" || arg == "-h" || arg == "help" {
            subcommand = Some(Command::Help);
        } else if arg == "--version" || arg == "-V" || arg == "version" {
            subcommand = Some(Command::Version);
        } else if subcommand.is_none() {
            match arg.as_str() {
                "chat" => subcommand = Some(Command::Chat),
                "serve" => subcommand = Some(Command::Serve),
                "ask" => {
                    subcommand = Some(Command::Ask(String::new()));
                    if i + 1 < args_vec.len() && !args_vec[i + 1].starts_with('-') {
                        i += 1;
                        ask_text = Some(args_vec[i].clone());
                    }
                }
                _ => {
                    anyhow::bail!("Unknown argument or subcommand: {}", arg);
                }
            }
        } else if matches!(subcommand, Some(Command::Ask(_)))
            && ask_text.is_none()
            && !arg.starts_with('-')
        {
            ask_text = Some(arg.clone());
        } else {
            anyhow::bail!("Unexpected argument: {}", arg);
        }
        i += 1;
    }

    let command = match subcommand {
        Some(Command::Ask(_)) => {
            let text = ask_text.unwrap_or_default();
            if text.trim().is_empty() {
                anyhow::bail!("Command 'ask' requires a prompt text argument");
            }
            Command::Ask(text)
        }
        Some(cmd) => cmd,
        None => Command::Chat,
    };

    Ok(CliArgs { command, port, url })
}

fn print_help() {
    println!("Nami CLI — AI Assistant");
    println!();
    println!("Usage: nami [COMMAND] [OPTIONS]");
    println!();
    println!("Commands:");
    println!("  chat             Start server in background and run interactive REPL (default)");
    println!("  serve            Start the HTTP API server only");
    println!("  ask \"<text>\"     Start server, send one query, print streamed answer, and exit");
    println!();
    println!("Options:");
    println!("  --port <n>       Override HTTP server port");
    println!("  --url <base>     Attach to an already-running server (e.g. http://127.0.0.1:8080)");
    println!("  -h, --help       Print help");
    println!("  -V, --version    Print version");
}

fn print_version(config: &NamiConfig) {
    println!("nami {}", config.version);
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt::init();

    let args: Vec<String> = std::env::args().collect();
    let cli_args = match parse_cli_args(&args) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    };

    let config = NamiConfig::load_default()?;

    match cli_args.command {
        Command::Help => {
            print_help();
            return Ok(());
        }
        Command::Version => {
            print_version(&config);
            return Ok(());
        }
        _ => {}
    }

    let guard = WorkspaceGuard::new(
        config.workspace.allowed_paths.clone(),
        config.workspace.read_only,
    )?;

    let memory = NamiMemory::init(&config.storage.db_path, &config.storage.wiki_dir).await?;
    let nami_agent = build_nami_agent(&config, guard).await?;
    let agent = nami_agent.agent;
    let mcp_manager = nami_agent.mcp_manager;

    let effective_port = cli_args
        .port
        .or_else(|| std::env::var("PORT").ok().and_then(|p| p.parse().ok()))
        .unwrap_or(config.server.port);
    let effective_host = &config.server.host;

    match cli_args.command {
        Command::Serve => {
            let app =
                server::build_app(agent, memory.session_service.clone(), &config)?;
            let addr = format!("{}:{}", effective_host, effective_port);
            tracing::info!("🚀 Starting Nami Axum HTTP server on http://{}", addr);

            let listener = tokio::net::TcpListener::bind(&addr).await?;
            tracing::info!("✅ Nami API & Web UI ready at http://{}/ui/", addr);
            tracing::info!("🔄 Durable background runs (/api/runs) & Cron (/api/cron) enabled");
            axum::serve(listener, app).await?;
            if let Some(mcp) = mcp_manager {
                let _ = mcp.shutdown().await;
            }
        }
        Command::Chat | Command::Ask(_) => {
            let (mut client, server_handle) = if let Some(ref url_base) = cli_args.url {
                tracing::info!("Connecting to existing Nami server at {}", url_base);
                let client =
                    NamiApiClient::connect(url_base, &config.name, &config.cli.user_id).await?;
                (client, None)
            } else {
                let app =
                    server::build_app(agent, memory.session_service.clone(), &config)?;
                let server = NamiServer::start(app, effective_host, effective_port).await?;
                let client =
                    NamiApiClient::connect(&server.base_url, &config.name, &config.cli.user_id)
                        .await?;
                (client, Some(server))
            };

            match cli_args.command {
                Command::Chat => {
                    cli::run_repl(&mut client, &config).await?;
                }
                Command::Ask(text) => {
                    let response = client.run_stream(&text).await?;
                    let _ = cli::render_stream_response(
                        response,
                        config::DisplayOptions::all_disabled(),
                    )
                    .await;
                    println!();
                }
                _ => unreachable!(),
            }

            if let Some(server) = server_handle {
                server.shutdown().await;
            }
            if let Some(mcp) = mcp_manager {
                let _ = mcp.shutdown().await;
            }
        }
        Command::Help | Command::Version => unreachable!(),
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_args_default() {
        let args = vec!["nami".to_string()];
        let parsed = parse_cli_args(&args).unwrap();
        assert_eq!(parsed.command, Command::Chat);
        assert_eq!(parsed.port, None);
        assert_eq!(parsed.url, None);
    }

    #[test]
    fn test_parse_args_chat_with_port() {
        let args = vec![
            "nami".to_string(),
            "chat".to_string(),
            "--port".to_string(),
            "9000".to_string(),
        ];
        let parsed = parse_cli_args(&args).unwrap();
        assert_eq!(parsed.command, Command::Chat);
        assert_eq!(parsed.port, Some(9000));
    }

    #[test]
    fn test_parse_args_serve_with_url_and_port() {
        let args = vec![
            "nami".to_string(),
            "--port=3000".to_string(),
            "serve".to_string(),
        ];
        let parsed = parse_cli_args(&args).unwrap();
        assert_eq!(parsed.command, Command::Serve);
        assert_eq!(parsed.port, Some(3000));
    }

    #[test]
    fn test_parse_args_ask() {
        let args = vec![
            "nami".to_string(),
            "ask".to_string(),
            "hello world".to_string(),
        ];
        let parsed = parse_cli_args(&args).unwrap();
        assert_eq!(parsed.command, Command::Ask("hello world".to_string()));
    }

    #[test]
    fn test_parse_args_help() {
        let args = vec!["nami".to_string(), "--help".to_string()];
        let parsed = parse_cli_args(&args).unwrap();
        assert_eq!(parsed.command, Command::Help);
    }
}
