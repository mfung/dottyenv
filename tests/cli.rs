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
fn scan_finds_a_variable_the_code_reads_but_the_schema_omits() {
    let dir = project("PORT=8080\n");
    write(
        dir.path(),
        "app.js",
        "const k = process.env.SENDGRID_API_KEY;\n",
    );

    dottyenv(dir.path())
        .arg("scan")
        .assert()
        .code(0) // advisory, never a gate
        .stdout(predicates::str::contains("used but not declared"))
        .stdout(predicates::str::contains("SENDGRID_API_KEY"))
        .stdout(predicates::str::contains("app.js:1"));
}

#[test]
fn scan_finds_a_declared_variable_no_code_mentions() {
    let dir = project("PORT=8080\n");
    write(dir.path(), "app.py", "import os\nos.getenv('PORT')\n");

    let assert = dottyenv(dir.path()).arg("scan").assert().code(0);
    let out = String::from_utf8(assert.get_output().stdout.clone()).unwrap();

    assert!(out.contains("declared but never mentioned"), "{out}");
    assert!(out.contains("DATABASE_URL"), "{out}");
    // PORT is read by app.py, so it must not be listed as unused.
    let unused = out.split("declared but never mentioned").nth(1).unwrap();
    assert!(!unused.contains("PORT"), "{out}");
}

/// The schema names every variable by definition. Counting it as usage would
/// make the unmentioned list permanently empty.
#[test]
fn scan_does_not_treat_the_schema_itself_as_usage() {
    let dir = project("PORT=8080\n");
    dottyenv(dir.path())
        .arg("scan")
        .assert()
        .stdout(predicates::str::contains("DATABASE_URL"));
}

#[test]
fn scan_respects_gitignore() {
    let dir = project("PORT=8080\n");
    write(dir.path(), ".gitignore", "vendor/\n");
    std::fs::create_dir(dir.path().join("vendor")).unwrap();
    write(
        &dir.path().join("vendor"),
        "dep.js",
        "process.env.VENDORED_ONLY_KEY\n",
    );

    let assert = dottyenv(dir.path()).arg("scan").assert().code(0);
    let out = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    assert!(
        !out.contains("VENDORED_ONLY_KEY"),
        "scanned an ignored dir: {out}"
    );
}

#[test]
fn scan_warns_about_dynamic_access() {
    let dir = project("PORT=8080\n");
    write(dir.path(), "app.js", "const v = process.env[userKey];\n");

    dottyenv(dir.path())
        .arg("scan")
        .assert()
        .code(0)
        .stdout(predicates::str::contains("dynamically"));
}

#[test]
fn scan_json_is_parseable() {
    let dir = project("PORT=8080\n");
    write(dir.path(), "app.js", "process.env.EXTRA_KEY\n");

    let assert = dottyenv(dir.path())
        .args(["scan", "--json"])
        .assert()
        .code(0);
    let parsed: serde_json::Value =
        serde_json::from_slice(&assert.get_output().stdout).expect("valid JSON");
    assert!(parsed["undeclared"]["EXTRA_KEY"].is_object());
}

// ------------------------------------------------------- environments

const MULTI_ENV: &str = r#"
[vars.DATABASE_URL]
required = true

[envs.development.vars.DATABASE_URL]
pattern = '^(sqlite:|file:)'

[envs.production.vars.DATABASE_URL]
pattern = '^postgres(ql)?://'
"#;

fn multi_env_project() -> TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    write(dir.path(), "dottyenv.toml", MULTI_ENV);
    write(
        dir.path(),
        ".env.development",
        "DATABASE_URL=sqlite:///dev.db\n",
    );
    write(
        dir.path(),
        ".env.production",
        "DATABASE_URL=postgres://db/app\n",
    );
    dir
}

/// The case this feature exists for: SQLite locally, Postgres in production.
/// One schema, and each file passes under its own environment.
#[test]
fn each_environment_accepts_its_own_database() {
    let dir = multi_env_project();
    for file in [".env.development", ".env.production"] {
        dottyenv(dir.path())
            .args(["check", "--file", file])
            .assert()
            .code(0);
    }
}

#[test]
fn sqlite_in_production_is_rejected() {
    let dir = multi_env_project();
    write(
        dir.path(),
        ".env.production",
        "DATABASE_URL=sqlite:///oops.db\n",
    );

    dottyenv(dir.path())
        .args(["check", "--file", ".env.production"])
        .assert()
        .code(1)
        .stdout(predicates::str::contains("^postgres(ql)?://"));
}

#[test]
fn the_applied_environment_is_named_in_the_output() {
    let dir = multi_env_project();
    dottyenv(dir.path())
        .args(["check", "--file", ".env.production"])
        .assert()
        .code(0)
        .stdout(predicates::str::contains("[production]"));
}

#[test]
fn env_flag_overrides_the_inferred_name() {
    let dir = multi_env_project();
    // A production file checked against development rules passes, since
    // postgres:// is only rejected by nothing in the development overlay.
    dottyenv(dir.path())
        .args(["check", "--file", ".env.production", "--env", "development"])
        .assert()
        .code(1)
        .stdout(predicates::str::contains("sqlite"));
}

#[test]
fn a_misspelled_environment_is_a_config_error() {
    let dir = multi_env_project();
    dottyenv(dir.path())
        .args(["check", "--env", "prodcution"])
        .assert()
        .code(3)
        .stderr(predicates::str::contains("prodcution"))
        .stderr(predicates::str::contains("production"));
}

/// .env.local is a common filename that names no environment, so it must fall
/// back to the base schema rather than erroring.
#[test]
fn an_unrecognised_filename_falls_back_to_the_base_schema() {
    let dir = multi_env_project();
    write(dir.path(), ".env.local", "DATABASE_URL=anything-at-all\n");

    dottyenv(dir.path())
        .args(["check", "--file", ".env.local"])
        .assert()
        .code(0);
}

#[test]
fn an_overlay_naming_an_undeclared_variable_exits_3() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "dottyenv.toml",
        "[vars.FOO]\nrequired = true\n\n[envs.dev.vars.TYPOD]\nrequired = true\n",
    );
    write(dir.path(), ".env", "FOO=1\n");

    dottyenv(dir.path())
        .arg("check")
        .assert()
        .code(3)
        .stderr(predicates::str::contains("TYPOD"));
}

// ------------------------------------------------------------ completions

#[test]
fn completions_generate_for_every_supported_shell() {
    let dir = project("PORT=8080\n");
    for shell in ["bash", "zsh", "fish", "powershell", "elvish"] {
        let assert = dottyenv(dir.path())
            .args(["completions", shell])
            .assert()
            .code(0);
        let out = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
        assert!(out.contains("dottyenv"), "{shell} script looks empty");
    }
}

/// Completions must not need a schema, since they are generated during install
/// with no project in scope.
#[test]
fn completions_work_without_a_schema() {
    let dir = tempfile::tempdir().unwrap();
    dottyenv(dir.path())
        .args(["completions", "bash"])
        .assert()
        .code(0);
}

#[test]
fn an_unknown_shell_is_a_usage_error() {
    let dir = project("PORT=8080\n");
    dottyenv(dir.path())
        .args(["completions", "klingon"])
        .assert()
        .code(2);
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
