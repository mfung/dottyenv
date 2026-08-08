pub mod check;
pub mod cli;
pub mod envfile;
pub mod schema;

use anyhow::{bail, Result};
use cli::{Cli, Command};

/// Distinguishes "the tool ran and found problems" (exit 1) from "the tool could
/// not run" (exit 3). See PRD §6.
pub enum Outcome {
    Success,
    Failure,
}

pub fn run(cli: Cli) -> Result<Outcome> {
    match cli.command {
        Command::Check => cmd_check(&cli),
        Command::List => cmd_list(&cli),
        Command::Init => bail!("`init` is not implemented yet — see PRD §9, v0.1"),
        Command::Scan => bail!("`scan` is not implemented yet — see PRD §9, v0.2"),
    }
}

fn cmd_check(cli: &Cli) -> Result<Outcome> {
    let schema = schema::Schema::load(&cli.schema)?;
    let vars = envfile::load(&cli.file)?;
    let report = check::check(&schema, &vars);

    if cli.json {
        // Data to stdout, so the tool composes in pipes.
        anstream::println!("{}", serde_json::to_string_pretty(&report)?);
    } else if !cli.quiet {
        anstream::print!("{}", check::render(&report, &cli.file.display().to_string()));
    }

    Ok(if report.is_ok() {
        Outcome::Success
    } else {
        Outcome::Failure
    })
}

fn cmd_list(cli: &Cli) -> Result<Outcome> {
    let schema = schema::Schema::load(&cli.schema)?;
    let vars = envfile::load(&cli.file)?;

    for (name, spec) in &schema.vars {
        let status = if vars.get(name).is_some_and(|v| !v.is_empty()) {
            "set"
        } else if spec.required {
            "MISSING"
        } else {
            "unset"
        };
        let required = if spec.required { "required" } else { "optional" };
        anstream::println!("{name:<32} {status:<8} {required}");
    }

    Ok(Outcome::Success)
}
