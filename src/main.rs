mod cli;
mod client;
mod commands;
mod config;
mod error;
mod models;
mod output;

use clap::Parser;
use cli::{Cli, Command};
use colored::*;
use error::RmsResult;

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    if let Err(e) = run(&cli).await {
        eprintln!();
        eprintln!("  {} {}", "[ERR]".red().bold(), e);
        if let Some(hint) = e.hint() {
            eprintln!("  {} {}", "[-->]".truecolor(255, 103, 0), hint);
        }
        eprintln!();
        std::process::exit(1);
    }
}

async fn run(cli: &Cli) -> RmsResult<()> {
    let config = config::load_config(cli.server.as_deref(), cli.token.as_deref())?;
    let api = client::ApiClient::new(&config);

    match &cli.command {
        Command::Robots { cmd } => commands::robots::run(cmd, &api, cli.json).await,
        Command::Alerts { cmd } => commands::alerts::run(cmd, &api, cli.json).await,
        Command::Status => commands::status::run(&api, cli.json).await,
        Command::Datasets { cmd } => commands::datasets::run(cmd, &api, cli.json).await,
        Command::Training { cmd } => commands::training::run(cmd, &api, cli.json).await,
        Command::Models { cmd } => commands::models::run(cmd, &api, cli.json).await,
        Command::Deployments { cmd } => commands::deployments::run(cmd, &api, cli.json).await,
        Command::Skills { cmd } => commands::skills::run(cmd, &api, cli.json).await,
        Command::Embodiments { cmd } => commands::embodiments::run(cmd, &api, cli.json).await,
        Command::Eval { cmd } => commands::eval::run(cmd, &api, cli.json).await,
    }
}
