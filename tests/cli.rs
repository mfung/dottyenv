//! End-to-end tests: spawn the real binary and assert on the contract users see.
//!
//! The exit codes and the stdout/stderr split are a public interface. Changing
//! them breaks scripts and CI gates, so they are pinned here rather than checked
//! by hand.

use assert_cmd::Command;
use std::path::Path;
use tempfile::TempDir;

const SCHEMA: &str = r#"
[vars.DATABASE_URL]
required    = true
pattern     = '^postgres(ql)?://'
description = 'PostgreSQL connection string'
secret      = true

[vars.LOG_LEVEL]
required = false
one_of   = ["debug", "info", "warn", "error"]

[vars.PORT]
required = true
"#;

fn project(env: &str) -> TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    write(dir.path(), "dottyenv.toml", SCHEMA);
    write(dir.path(), ".env", env);
    dir
}

fn write(dir: &Path, name: &str, contents: &str) {
    std::fs::write(dir.join(name), contents).expect("write fixture");
}

fn dottyenv(dir: &Path) -> Command {
    let mut cmd = Command::cargo_bin("dottyenv").expect("binary built");
    cmd.current_dir(dir);
    cmd
}

// ---------------------------------------------------------------- exit codes

#[test]
fn clean_env_exits_0() {
    let dir = project("DATABASE_URL=postgres://localhost/app\nPORT=8080\n");
    dottyenv(dir.path()).arg("check").assert().code(0);
}

#[test]
fn validation_failure_exits_1() {
    let dir = project("DATABASE_URL=mysql://localhost/app\nPORT=8080\n");
    dottyenv(dir.path()).arg("check").assert().code(1);
}

#[test]
fn usage_error_exits_2() {
    let dir = project("PORT=8080\n");
    dottyenv(dir.path()).arg("--bogus").assert().code(2);
    dottyenv(dir.path()).arg("nosuchcommand").assert().code(2);
}

#[test]
fn missing_schema_exits_3() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), ".env", "FOO=bar\n");
    dottyenv(dir.path()).arg("check").assert().code(3);
}

#[test]
fn unparseable_schema_exits_3() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "dottyenv.toml", "this is not valid toml {{{");
    write(dir.path(), ".env", "FOO=bar\n");
    dottyenv(dir.path()).arg("check").assert().code(3);
}

// ------------------------------------------------------------ stream splitting

#[test]
fn json_goes_to_stdout_and_parses() {
    let dir = project("DATABASE_URL=mysql://localhost/app\nPORT=8080\n");
    let out = dottyenv(dir.path())
        .args(["check", "--json"])
        .assert()
        .code(1)
        .get_output()
        .stdout
        .clone();

    let parsed: serde_json::Value = serde_json::from_slice(&out).expect("stdout is valid JSON");
    assert_eq!(parsed["required_total"], 2);
    assert_eq!(parsed["findings"][0]["name"], "DATABASE_URL");
    assert_eq!(parsed["findings"][0]["kind"], "invalid");
}

#[test]
fn errors_go_to_stderr_leaving_stdout_clean() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), ".env", "FOO=bar\n");
    let assert = dottyenv(dir.path()).arg("check").assert().code(3);
    let output = assert.get_output();

    assert!(output.stdout.is_empty(), "stdout must stay pipeable");
    assert!(String::from_utf8_lossy(&output.stderr).contains("error:"));
}

#[test]
fn quiet_suppresses_output_but_keeps_the_exit_code() {
    let dir = project("DATABASE_URL=mysql://localhost/app\nPORT=8080\n");
    let assert = dottyenv(dir.path())
        .args(["check", "--quiet"])
        .assert()
        .code(1);
    assert!(assert.get_output().stdout.is_empty());
}

// ----------------------------------------------------------------- secrecy

#[test]
fn a_secret_value_never_reaches_any_stream() {
    let dir = project("DATABASE_URL=mysql://user:hunter2@db.internal/prod\nPORT=8080\n");
    let assert = dottyenv(dir.path()).arg("check").assert().code(1);
    let output = assert.get_output();

    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!combined.contains("hunter2"), "secret leaked: {combined}");
    assert!(combined.contains("<redacted,"));
}

#[test]
fn init_never_writes_a_secret_into_the_schema() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), ".env", "SESSION_SECRET=changeme\n");
    dottyenv(dir.path()).arg("init").assert().code(0);

    let schema = std::fs::read_to_string(dir.path().join("dottyenv.toml")).unwrap();
    assert!(
        !schema.contains("changeme"),
        "secret leaked into schema:\n{schema}"
    );
}

// -------------------------------------------------------------------- init

/// The invariant that a generated schema can never reject its own source file.
#[test]
fn init_output_always_validates_its_input() {
    for value in [
        "postgres://localhost/app",
        "mysql://localhost/app",
        "mongodb+srv://cluster0.example/app",
        "sqlite:///var/db/app.db",
        "file:./dev.db",
        "./data/app.db",
    ] {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), ".env", &format!("DATABASE_URL={value}\n"));

        dottyenv(dir.path()).arg("init").assert().code(0);
        dottyenv(dir.path())
            .arg("check")
            .assert()
            .code(0)
            .stdout(predicates::str::contains("1 of 1"));
    }
}

#[test]
fn init_refuses_to_overwrite_without_force() {
    let dir = project("PORT=8080\n");
    dottyenv(dir.path()).arg("init").assert().code(3);

    // The original schema must survive the refusal.
    let schema = std::fs::read_to_string(dir.path().join("dottyenv.toml")).unwrap();
    assert!(schema.contains("DATABASE_URL"));
}

#[test]
fn init_force_overwrites() {
    let dir = project("PORT=8080\n");
    dottyenv(dir.path())
        .args(["init", "--force"])
        .assert()
        .code(0);

    let schema = std::fs::read_to_string(dir.path().join("dottyenv.toml")).unwrap();
    assert!(
        !schema.contains("DATABASE_URL"),
        "should reflect the new .env"
    );
    assert!(schema.contains("PORT"));
}

#[test]
fn init_falls_back_to_env_example() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), ".env.example", "STRIPE_SECRET_KEY=\n");

    dottyenv(dir.path())
        .arg("init")
        .assert()
        .code(0)
        .stderr(predicates::str::contains(".env.example"));

    let schema = std::fs::read_to_string(dir.path().join("dottyenv.toml")).unwrap();
    assert!(
        schema.contains("dashboard.stripe.com"),
        "catalog source missing"
    );
}

#[test]
fn init_pluralises_its_summary() {
    let one = tempfile::tempdir().unwrap();
    write(one.path(), ".env", "ONLY=1\n");
    dottyenv(one.path())
        .arg("init")
        .assert()
        .stderr(predicates::str::contains("(1 variable from"));

    let many = tempfile::tempdir().unwrap();
    write(many.path(), ".env", "A=1\nB=2\n");
    dottyenv(many.path())
        .arg("init")
        .assert()
        .stderr(predicates::str::contains("(2 variables from"));
}

// -------------------------------------------------------------------- list

#[test]
fn list_reports_status_per_variable() {
    let dir = project("DATABASE_URL=postgres://localhost/app\n");
    dottyenv(dir.path())
        .arg("list")
        .assert()
        .code(0)
        .stdout(predicates::str::contains("DATABASE_URL"))
        .stdout(predicates::str::contains("MISSING"));
}

// ---------------------------------------------------------------- stubs

#[test]
fn scan_is_still_a_stub() {
    let dir = project("PORT=8080\n");
    dottyenv(dir.path())
        .arg("scan")
        .assert()
        .code(3)
        .stderr(predicates::str::contains("not implemented"));
}

// --------------------------------------------------------------- rendering

/// Pins the human-readable report. anstream strips styling when stdout is not a
/// TTY, so the snapshot is plain text.
#[test]
fn check_report_format() {
    let dir = project("DATABASE_URL=mysql://user:hunter2@db.internal/prod\nLOG_LEVEL=verbose\n");
    let assert = dottyenv(dir.path()).arg("check").assert().code(1);
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();

    insta::assert_snapshot!(stdout);
}
