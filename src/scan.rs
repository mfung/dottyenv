//! Find environment variables the codebase reads.
//!
//! Two mechanisms, neither of which knows anything about specific languages.
//!
//! **Presence** is exact. The schema already holds the name, so answering "is
//! `DATABASE_URL` used anywhere?" is a literal search. It works in any language,
//! plus Dockerfiles, Helm charts, and CI config. This is the direction where a
//! false positive is most damaging, since the advice is "delete this".
//!
//! **Discovery** is heuristic. Finding names that are *not* in the schema means
//! recognising env access without being told what to look for. It leans on the
//! fact that every language spells it with the substring `env`:
//!
//! ```text
//! process.env.FOO      os.environ["FOO"]     env::var("FOO")
//! os.Getenv("FOO")     ENV["FOO"]            System.getenv("FOO")
//! $ENV{FOO}            System.get_env("FOO") getenv("FOO")
//! Environment.GetEnvironmentVariable("FOO")
//! ```
//!
//! One bounded pattern covers all of them, including languages nobody listed.
//! The cost is precision, which is why `scan` is advisory and never a gate.

use crate::schema::Schema;
use anyhow::Result;
use ignore::WalkBuilder;
use regex::Regex;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::OnceLock;

/// `env` (any case), then at most 24 characters on the same line, then a
/// delimiter, then a SCREAMING_SNAKE name.
///
/// The delimiter requirement is what stops `MY_ENV_VAR` matching itself: the
/// character after `ENV` is `_`, which is not a delimiter, and there is no later
/// delimiter before the name ends. The 24 character budget is set by the longest
/// real form, `Environment.GetEnvironmentVariable("`.
///
/// Case insensitivity is scoped to `(?i:env)` rather than applied to the whole
/// pattern. A blanket `(?i)` also loosens the name class, so `Deno.env.get("FOO")`
/// captures `get`.
fn discovery() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r#"(?i:env)[^\n]{0,24}?["'`.\[({,= ](?P<name>[A-Z][A-Z0-9_]{2,})"#)
            .expect("static regex")
    })
}

/// Env access whose argument is an expression rather than a literal, so the name
/// cannot be known statically: `process.env[key]`, `os.getenv(name)`,
/// `process.env[req.query.name]`.
///
/// The test is simply that the first character inside the bracket is not a
/// quote. Requiring a closing bracket would miss dotted expressions.
fn dynamic() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        // The `[.:]{1,2}\w+` repetition covers `.get`, `::var` and similar.
        Regex::new(r#"(?i:env)\w{0,20}(?:[.:]{1,2}\w+)*\s*[\[(]\s*[a-zA-Z_$]"#)
            .expect("static regex")
    })
}

pub fn names_in(text: &str) -> BTreeSet<String> {
    discovery()
        .captures_iter(text)
        .map(|c| c["name"].to_string())
        .collect()
}

pub fn has_dynamic_access(text: &str) -> bool {
    dynamic().is_match(text)
}

#[derive(Debug, Serialize)]
pub struct Sighting {
    pub file: String,
    pub line: usize,
}

#[derive(Debug, Serialize)]
pub struct ScanReport {
    /// Read by the code but absent from the schema. Heuristic.
    pub undeclared: BTreeMap<String, Sighting>,
    /// Declared but not mentioned in any scanned file. Exact.
    pub unmentioned: Vec<String>,
    /// Files containing env access that cannot be resolved statically.
    pub dynamic_access: Vec<String>,
    pub files_scanned: usize,
}

impl ScanReport {
    pub fn is_clean(&self) -> bool {
        self.undeclared.is_empty() && self.unmentioned.is_empty()
    }
}

pub fn scan(root: &Path, schema: &Schema, skip: &[&Path]) -> Result<ScanReport> {
    let declared: BTreeSet<&str> = schema.vars.keys().map(String::as_str).collect();

    let mut undeclared: BTreeMap<String, Sighting> = BTreeMap::new();
    let mut unmentioned: BTreeSet<&str> = declared.clone();
    let mut dynamic_access = Vec::new();
    let mut files_scanned = 0;

    // WalkBuilder honours .gitignore, so node_modules and target are skipped for
    // free. A variable referenced only inside a vendored dependency is not your
    // code reading it. require_git(false) applies those rules even outside a
    // repository, so the same directory does not scan differently depending on
    // whether it happens to be checked in.
    for entry in WalkBuilder::new(root).require_git(false).build().flatten() {
        let path = entry.path();
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        if is_excluded(path, skip, root) {
            continue;
        }
        // Non-UTF8 fails here, which skips binaries without a content sniffer.
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        files_scanned += 1;

        let display = path
            .strip_prefix(root)
            .unwrap_or(path)
            .display()
            .to_string();

        for (n, line) in text.lines().enumerate() {
            for name in names_in(line) {
                if declared.contains(name.as_str()) {
                    continue;
                }
                undeclared.entry(name).or_insert_with(|| Sighting {
                    file: display.clone(),
                    line: n + 1,
                });
            }
        }

        // Exact half: a plain substring search, so it works in any language.
        unmentioned.retain(|name| !text.contains(name));

        if has_dynamic_access(&text) {
            dynamic_access.push(display);
        }
    }

    Ok(ScanReport {
        undeclared,
        unmentioned: unmentioned.iter().map(|s| s.to_string()).collect(),
        dynamic_access,
        files_scanned,
    })
}

/// The schema and the env files name every variable by definition, so counting
/// them as usage would make `unmentioned` always empty.
fn is_excluded(path: &Path, skip: &[&Path], root: &Path) -> bool {
    // The walker yields `./dottyenv.toml` while `--schema` supplies
    // `dottyenv.toml`, so both sides are stripped of the root before comparing.
    fn rel<'a>(p: &'a Path, root: &Path) -> &'a Path {
        p.strip_prefix(root).unwrap_or(p)
    }
    if skip.iter().any(|s| rel(path, root) == rel(s, root)) {
        return true;
    }
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.starts_with(".env"))
}

pub fn render(report: &ScanReport) -> String {
    let mut out = String::new();

    if !report.undeclared.is_empty() {
        out.push_str("used but not declared\n");
        for (name, at) in &report.undeclared {
            out.push_str(&format!("  {name:<32} {}:{}\n", at.file, at.line));
        }
        out.push('\n');
    }

    if !report.unmentioned.is_empty() {
        out.push_str("declared but never mentioned\n");
        for name in &report.unmentioned {
            out.push_str(&format!("  {name}\n"));
        }
        out.push('\n');
    }

    if report.is_clean() {
        let n = report.files_scanned;
        let noun = if n == 1 { "file" } else { "files" };
        out.push_str(&format!("schema and code agree ({n} {noun} scanned)\n"));
    }

    if !report.dynamic_access.is_empty() {
        let n = report.dynamic_access.len();
        let subject = if n == 1 {
            "file accesses"
        } else {
            "files access"
        };
        out.push_str(&format!(
            "note: {n} {subject} the environment dynamically, so this list may be\n      incomplete: {}\n",
            report.dynamic_access.join(", ")
        ));
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(text: &str) -> Vec<String> {
        names_in(text).into_iter().collect()
    }

    #[test]
    fn finds_the_name_in_every_language_we_care_about() {
        for src in [
            r#"process.env.FOO"#,
            r#"process.env["FOO"]"#,
            r#"import.meta.env.FOO"#,
            r#"Deno.env.get("FOO")"#,
            r#"Bun.env.FOO"#,
            r#"os.environ["FOO"]"#,
            r#"os.environ.get("FOO")"#,
            r#"os.getenv("FOO")"#,
            r#"env::var("FOO")"#,
            r#"std::env::var_os("FOO")"#,
            r#"option_env!("FOO")"#,
            r#"os.Getenv("FOO")"#,
            r#"os.LookupEnv("FOO")"#,
            r#"ENV["FOO"]"#,
            r#"ENV.fetch("FOO")"#,
            r#"getenv("FOO")"#,
            r#"$_ENV["FOO"]"#,
            r#"env("FOO")"#,
            r#"System.getenv("FOO")"#,
            r#"Environment.GetEnvironmentVariable("FOO")"#,
            r#"System.get_env("FOO")"#,
            r#"$ENV{FOO}"#,
            r#"ProcessInfo.processInfo.environment["FOO"]"#,
        ] {
            assert!(
                names(src).contains(&"FOO".to_string()),
                "did not find FOO in: {src}"
            );
        }
    }

    #[test]
    fn handles_realistic_names_not_just_foo() {
        assert!(names(r#"process.env.STRIPE_SECRET_KEY"#).contains(&"STRIPE_SECRET_KEY".into()));
        assert!(names(r#"import.meta.env.VITE_API_URL"#).contains(&"VITE_API_URL".into()));
    }

    /// The delimiter requirement exists for this case: `ENV` appears inside the
    /// name itself, and must not cause the tail to be reported as a variable.
    #[test]
    fn does_not_match_env_inside_an_identifier() {
        assert!(names("let MY_ENV_VAR = 1;").is_empty());
        assert!(names("NODE_ENV_SETTING").is_empty());
    }

    #[test]
    fn ignores_screaming_text_with_no_env_nearby() {
        assert!(names("const MAX_RETRIES = 5;").is_empty());
        assert!(names("HTTP_OK and NOT_FOUND are constants").is_empty());
    }

    #[test]
    fn detects_dynamic_access() {
        assert!(has_dynamic_access("process.env[key]"));
        assert!(has_dynamic_access("os.getenv(name)"));
        assert!(has_dynamic_access("process.env.get(varName)"));
        assert!(has_dynamic_access("os.environ.get(k)"));
        assert!(has_dynamic_access("env::var(name)"));
        // A dotted expression, which an earlier version missed.
        assert!(has_dynamic_access("process.env[req.query.name]"));
        // A constant holding the name is still not statically resolvable.
        assert!(has_dynamic_access("process.env[KEY]"));
    }

    #[test]
    fn literal_access_is_not_dynamic() {
        assert!(!has_dynamic_access(r#"process.env["FOO"]"#));
        assert!(!has_dynamic_access(r#"os.getenv("FOO")"#));
        assert!(!has_dynamic_access(r#"os.environ.get("FOO")"#));
        assert!(!has_dynamic_access(r#"env::var("FOO")"#));
        assert!(!has_dynamic_access(r#"env("FOO")"#));
    }
}
