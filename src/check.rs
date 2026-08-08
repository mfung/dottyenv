use crate::schema::{Schema, VarSpec};
use anstyle::{AnsiColor, Style};
use regex::Regex;
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Problem {
    Missing,
    Invalid { reason: String, got: String },
}

#[derive(Debug, Serialize)]
pub struct Finding {
    pub name: String,
    #[serde(flatten)]
    pub problem: Problem,
    pub description: Option<String>,
    pub source: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct Report {
    pub findings: Vec<Finding>,
    pub required_total: usize,
    pub required_ok: usize,
}

impl Report {
    pub fn is_ok(&self) -> bool {
        self.findings.is_empty()
    }
}

pub fn check(schema: &Schema, vars: &BTreeMap<String, String>) -> Report {
    let mut findings = Vec::new();
    let mut required_total = 0;
    let mut required_ok = 0;

    for (name, spec) in &schema.vars {
        if spec.required {
            required_total += 1;
        }

        // An empty value is treated as absent: `FOO=` in a .env file almost always
        // means "not filled in yet", not "intentionally empty".
        let value = vars.get(name).filter(|v| !v.is_empty());

        let Some(value) = value else {
            if spec.required {
                findings.push(Finding {
                    name: name.clone(),
                    problem: Problem::Missing,
                    description: spec.description.clone(),
                    source: spec.source.clone(),
                });
            }
            continue;
        };

        match validate_value(spec, value) {
            Some(reason) => findings.push(Finding {
                name: name.clone(),
                problem: Problem::Invalid {
                    reason,
                    got: redact(spec, value),
                },
                description: spec.description.clone(),
                source: spec.source.clone(),
            }),
            None => {
                if spec.required {
                    required_ok += 1;
                }
            }
        }
    }

    Report {
        findings,
        required_total,
        required_ok,
    }
}

/// Returns a human-readable reason when the value fails, or None when it passes.
fn validate_value(spec: &VarSpec, value: &str) -> Option<String> {
    if let Some(pattern) = &spec.pattern {
        // Already validated at schema load time, so a failure here is unreachable
        // in practice; treat an unparseable regex as "no opinion" rather than panic.
        if let Ok(re) = Regex::new(pattern) {
            if !re.is_match(value) {
                return Some(format!("expected {pattern}"));
            }
        }
    }

    if let Some(allowed) = &spec.one_of {
        if !allowed.iter().any(|a| a == value) {
            // Bracket the list so the trailing ", got ..." cannot be misread as
            // another member of it.
            return Some(format!("expected one of [{}]", allowed.join(", ")));
        }
    }

    None
}

/// Never echo a secret value. Length alone is enough to debug a paste error.
fn redact(spec: &VarSpec, value: &str) -> String {
    if spec.secret {
        format!("<redacted, {} chars>", value.chars().count())
    } else {
        value.to_string()
    }
}

pub fn render(report: &Report, env_path: &str) -> String {
    let red = Style::new().fg_color(Some(AnsiColor::Red.into()));
    let green = Style::new().fg_color(Some(AnsiColor::Green.into()));
    let dim = Style::new().dimmed();
    let bold = Style::new().bold();

    if report.is_ok() {
        return format!(
            "{green}✓{green:#} {} of {} required variables OK in {env_path}\n",
            report.required_ok, report.required_total
        );
    }

    let mut out = String::new();
    let n = report.findings.len();
    let plural = if n == 1 { "problem" } else { "problems" };
    out.push_str(&format!(
        "{red}✗{red:#} {n} {plural} in {env_path}\n\n"
    ));

    for finding in &report.findings {
        let label = match &finding.problem {
            Problem::Missing => "MISSING",
            Problem::Invalid { .. } => "INVALID",
        };
        out.push_str(&format!(
            "  {red}{label}{red:#}   {bold}{}{bold:#}\n",
            finding.name
        ));

        if let Problem::Invalid { reason, got } = &finding.problem {
            out.push_str(&format!("            {dim}{reason}, got {got:?}{dim:#}\n"));
        }
        if let Some(description) = &finding.description {
            out.push_str(&format!("            {dim}{description}{dim:#}\n"));
        }
        if let Some(source) = &finding.source {
            out.push_str(&format!("            → {source}\n"));
        }
        out.push('\n');
    }

    out.push_str(&format!(
        "  {} of {} required variables OK\n",
        report.required_ok, report.required_total
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schema_from(toml_str: &str) -> Schema {
        toml::from_str(toml_str).unwrap()
    }

    fn env_from(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn passes_when_required_var_is_present() {
        let schema = schema_from("[vars.FOO]\nrequired = true");
        let report = check(&schema, &env_from(&[("FOO", "bar")]));
        assert!(report.is_ok());
        assert_eq!(report.required_ok, 1);
    }

    #[test]
    fn reports_missing_required_var() {
        let schema = schema_from("[vars.FOO]\nrequired = true");
        let report = check(&schema, &env_from(&[]));
        assert_eq!(report.findings.len(), 1);
        assert!(matches!(report.findings[0].problem, Problem::Missing));
    }

    #[test]
    fn ignores_missing_optional_var() {
        let schema = schema_from("[vars.FOO]\nrequired = false");
        let report = check(&schema, &env_from(&[]));
        assert!(report.is_ok());
    }

    #[test]
    fn treats_empty_value_as_missing() {
        let schema = schema_from("[vars.FOO]\nrequired = true");
        let report = check(&schema, &env_from(&[("FOO", "")]));
        assert_eq!(report.findings.len(), 1);
    }

    #[test]
    fn reports_pattern_mismatch() {
        let schema = schema_from("[vars.DB]\nrequired = true\npattern = \"^postgres://\"");
        let report = check(&schema, &env_from(&[("DB", "mysql://localhost")]));
        assert!(matches!(
            report.findings[0].problem,
            Problem::Invalid { .. }
        ));
    }

    #[test]
    fn reports_one_of_mismatch() {
        let schema = schema_from("[vars.LOG]\none_of = [\"debug\", \"info\"]");
        let report = check(&schema, &env_from(&[("LOG", "verbose")]));
        assert_eq!(report.findings.len(), 1);
    }

    /// The rendered line appends ", got ...", so the allowed set must be
    /// delimited or `info` and `got` read as one comma-separated list.
    #[test]
    fn one_of_message_delimits_the_allowed_set() {
        let schema = schema_from("[vars.LOG]\none_of = [\"debug\", \"info\"]");
        let report = check(&schema, &env_from(&[("LOG", "verbose")]));
        let rendered = render(&report, ".env");
        assert!(
            rendered.contains("expected one of [debug, info], got \"verbose\""),
            "{rendered}"
        );
    }

    #[test]
    fn never_echoes_a_secret_value() {
        let schema = schema_from("[vars.KEY]\nrequired = true\npattern = \"^sk_\"\nsecret = true");
        let report = check(&schema, &env_from(&[("KEY", "totally-wrong-value")]));

        let Problem::Invalid { got, .. } = &report.findings[0].problem else {
            panic!("expected an Invalid finding");
        };
        assert!(!got.contains("totally-wrong-value"));
        assert_eq!(got, "<redacted, 19 chars>");
    }
}
