use als_core::{Error, ExitCode, Output, OutputMode};
use clap::{CommandFactory, Parser};

mod cli;
mod commands;
mod output;
mod prompt;

#[tokio::main]
async fn main() {
    let parsed = cli::Cli::parse();
    let mode = if parsed.json {
        OutputMode::Json
    } else if parsed.quiet {
        OutputMode::Quiet
    } else {
        OutputMode::Human
    };
    let mut out = Output::new(mode);
    let code = match run(&mut out, parsed).await {
        Ok(()) => 0,
        Err(e) => {
            output::report_error(&mut out, &e);
            e.exit_code()
        }
    };
    std::process::exit(code);
}

async fn run(out: &mut Output, parsed: cli::Cli) -> Result<(), Error> {
    match parsed.command {
        Some(cli::Command::Config(args)) => commands::config::run(out, args).await,
        Some(cli::Command::List(args)) => {
            let client = build_client(parsed.profile.as_deref())?;
            commands::list::run(&client, out, args).await
        }
        Some(cli::Command::Site(args)) => {
            let client = build_client(parsed.profile.as_deref())?;
            commands::site::run(&client, out, args).await
        }
        Some(cli::Command::Rm(args)) => {
            let client = build_client(parsed.profile.as_deref())?;
            commands::rm::run(&client, out, args).await
        }
        Some(cli::Command::Unpin(args)) => commands::unpin::run(out, &args),
        Some(cli::Command::Auth(args)) => {
            commands::auth::run(out, parsed.profile.as_deref(), args.action).await
        }
        Some(cli::Command::Completion(args)) => commands::completion::run(out, args).await,
        Some(cli::Command::Preview(args)) => commands::preview::run(out, args).await,
        None => {
            if let Some(path) = parsed.path {
                commands::deploy::run(out, parsed.profile.as_deref(), &path, parsed.deploy).await
            } else {
                let mut cmd = cli::Cli::command();
                let _ = cmd.print_help();
                Ok(())
            }
        }
    }
}

fn build_client(profile: Option<&str>) -> Result<als_api::Client, Error> {
    let cfg = als_core::config::load(profile)?;
    als_api::Client::new(&cfg).map_err(Error::from)
}
