use clap::Parser;
use std::process::ExitCode;

fn main() -> ExitCode {
    // Usage errors (exit 2) are handled by clap before we get here.
    let cli = dottyenv::cli::Cli::parse();

    match dottyenv::run(cli) {
        Ok(dottyenv::Outcome::Success) => ExitCode::SUCCESS,
        Ok(dottyenv::Outcome::Failure) => ExitCode::from(1),
        Err(err) => {
            // Diagnostics to stderr so stdout stays pipeable.
            eprintln!("error: {err:#}");
            ExitCode::from(3)
        }
    }
}
