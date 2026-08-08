use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Deserialize)]
pub struct Schema {
    #[serde(default)]
    pub meta: Meta,
    /// BTreeMap so output ordering is stable and diffable.
    #[serde(default)]
    pub vars: BTreeMap<String, VarSpec>,
}

#[derive(Debug, Deserialize)]
pub struct Meta {
    pub version: u32,
}

impl Default for Meta {
    fn default() -> Self {
        Self { version: 1 }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VarSpec {
    #[serde(default)]
    pub required: bool,
    pub pattern: Option<String>,
    pub one_of: Option<Vec<String>>,
    pub default: Option<String>,
    /// URL a human visits to obtain the value.
    pub source: Option<String>,
    pub description: Option<String>,
    /// Never echo this value in output.
    #[serde(default)]
    pub secret: bool,
}

impl Schema {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("could not read schema at {}", path.display()))?;
        let schema: Schema = toml::from_str(&text)
            .with_context(|| format!("could not parse schema at {}", path.display()))?;
        schema.validate()?;
        Ok(schema)
    }

    /// Catch bad regexes at load time rather than mid-check.
    fn validate(&self) -> Result<()> {
        for (name, spec) in &self.vars {
            if let Some(pattern) = &spec.pattern {
                regex::Regex::new(pattern)
                    .with_context(|| format!("invalid regex for {name}: {pattern}"))?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_minimal_schema() {
        let schema: Schema = toml::from_str(
            r#"
            [vars.DATABASE_URL]
            required = true
            pattern = "^postgres://"
            "#,
        )
        .unwrap();

        assert_eq!(schema.meta.version, 1);
        assert!(schema.vars["DATABASE_URL"].required);
        assert!(!schema.vars["DATABASE_URL"].secret);
    }

    #[test]
    fn rejects_unknown_fields() {
        // A typo like `requred` must be an error, not silence.
        let result: Result<Schema, _> = toml::from_str(
            r#"
            [vars.FOO]
            requred = true
            "#,
        );
        assert!(result.is_err());
    }

    #[test]
    fn rejects_invalid_regex() {
        let schema: Schema = toml::from_str(
            r#"
            [vars.FOO]
            pattern = "^(unclosed"
            "#,
        )
        .unwrap();
        assert!(schema.validate().is_err());
    }
}
