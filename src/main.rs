mod cli;
mod client;
mod commands;
mod config;
mod ctx;
mod error;
mod models;
mod output;

use clap::Parser;
use cli::{Cli, Command};
use colored::*;
use ctx::Ctx;
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
    output::set_requested_page(cli.page);
    let ctx = Ctx {
        json: cli.json,
        yes: cli.yes,
        page: cli.page,
        limit: cli.limit,
    };

    match &cli.command {
        Command::Robots { cmd } => commands::robots::run(cmd, &api, &ctx).await,
        Command::Alerts { cmd } => commands::alerts::run(cmd, &api, &ctx).await,
        Command::Status => commands::status::run(&api, &ctx).await,
        Command::Safety { cmd } => commands::safety::run(cmd, &api, &ctx).await,
        Command::Incidents { cmd } => commands::incidents::run(cmd, &api, &ctx).await,
        Command::Oversight { cmd } => commands::oversight::run(cmd, &api, &ctx).await,
        Command::Approvals { cmd } => commands::approvals::run(cmd, &api, &ctx).await,
        Command::Teleop { cmd } => commands::teleop::run(cmd, &api, &ctx).await,
        Command::Patrol { cmd } => commands::patrol::run(cmd, &api, &ctx).await,
        Command::Updates { cmd } => commands::updates::run(cmd, &api, &ctx).await,
        Command::Processes { cmd } => commands::processes::run(cmd, &api, &ctx).await,
        Command::Compliance { cmd } => commands::compliance::run(cmd, &api, &ctx).await,
        Command::Datasets { cmd } => commands::datasets::run(cmd, &api, &ctx).await,
        Command::Training { cmd } => commands::training::run(cmd, &api, &ctx).await,
        Command::Models { cmd } => commands::models::run(cmd, &api, &ctx).await,
        Command::Deployments { cmd } => commands::deployments::run(cmd, &api, &ctx).await,
        Command::Skills { cmd } => commands::skills::run(cmd, &api, &ctx).await,
        Command::Embodiments { cmd } => commands::embodiments::run(cmd, &api, &ctx).await,
        Command::Eval { cmd } => commands::eval::run(cmd, &api, &ctx).await,
    }
}
