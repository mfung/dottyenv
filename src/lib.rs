pub mod catalog;
pub mod check;
pub mod cli;
pub mod envfile;
pub mod init;
pub mod scan;
pub mod schema;

use anyhow::{bail, Context, Result};
use cli::{Cli, Command};
use std::path::{Path, PathBuf};

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
        Command::Init { force } => cmd_init(&cli, force),
        Command::Scan => cmd_scan(&cli),
    }
}

fn cmd_init(cli: &Cli, force: bool) -> Result<Outcome> {
    let source = resolve_source(&cli.file)?;

    if cli.schema.exists() && !force {
        bail!(
            "{} already exists; pass --force to overwrite",
            cli.schema.display()
        );
    }

    let vars = envfile::load(&source)?;
    if vars.is_empty() {
        bail!("no variables found in {}", source.display());
    }

    let catalog = catalog::Catalog::load()?;
    let generated = init::generate(&vars, &catalog);

    std::fs::write(&cli.schema, &generated)
        .with_context(|| format!("could not write {}", cli.schema.display()))?;

    if !cli.quiet {
        let count = vars.len();
        let noun = if count == 1 { "variable" } else { "variables" };
        anstream::eprintln!(
            "wrote {} ({count} {noun} from {})\nreview it, then run `dottyenv check`",
            cli.schema.display(),
            source.display()
        );
    }

    Ok(Outcome::Success)
}

/// Fall back to .env.example only when the user did not name a file explicitly.
/// A repo often has the example committed but no .env yet.
fn resolve_source(file: &Path) -> Result<PathBuf> {
    if file.exists() {
        return Ok(file.to_path_buf());
    }

    let example = Path::new(".env.example");
    if file == Path::new(".env") && example.exists() {
        return Ok(example.to_path_buf());
    }

    bail!("no env file found at {}", file.display())
}

fn cmd_check(cli: &Cli) -> Result<Outcome> {
    let schema = schema::Schema::load(&cli.schema)?;
    let vars = envfile::load(&cli.file)?;
    let report = check::check(&schema, &vars);

    if cli.json {
        // Data to stdout, so the tool composes in pipes.
        anstream::println!("{}", serde_json::to_string_pretty(&report)?);
    } else if !cli.quiet {
        anstream::print!(
            "{}",
            check::render(&report, &cli.file.display().to_string())
        );
    }

    Ok(if report.is_ok() {
        Outcome::Success
    } else {
        Outcome::Failure
    })
}

fn cmd_scan(cli: &Cli) -> Result<Outcome> {
    let schema = schema::Schema::load(&cli.schema)?;
    let report = scan::scan(Path::new("."), &schema, &[&cli.schema])?;

    if cli.json {
        anstream::println!("{}", serde_json::to_string_pretty(&report)?);
    } else if !cli.quiet {
        anstream::print!("{}", scan::render(&report));
    }

    // Always succeeds. Discovery is heuristic, so `scan` is advisory and must
    // never become a CI gate. See PRD §9.
    Ok(Outcome::Success)
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
        let required = if spec.required {
            "required"
        } else {
            "optional"
        };
        anstream::println!("{name:<32} {status:<8} {required}");
    }

    Ok(Outcome::Success)
}
