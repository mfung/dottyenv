use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Schema {
    #[serde(default)]
    pub meta: Meta,
    /// BTreeMap so output ordering is stable and diffable.
    #[serde(default)]
    pub vars: BTreeMap<String, VarSpec>,
    /// Per-environment constraint overlays. See PRD §8.
    #[serde(default)]
    pub envs: BTreeMap<String, EnvOverlay>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvOverlay {
    #[serde(default)]
    pub vars: BTreeMap<String, VarOverride>,
}

/// An overlay may override constraints, never identity.
///
/// `description`, `source` and `secret` are absent on purpose. A variable means
/// the same thing in every environment, you obtain it from the same place, and a
/// credential is not sensitive in production and safe in development.
/// `deny_unknown_fields` turns an attempt at any of those into a load error.
#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct VarOverride {
    pub required: Option<bool>,
    pub pattern: Option<String>,
    pub one_of: Option<Vec<String>>,
    pub default: Option<String>,
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

#[derive(Debug, Deserialize, Clone)]
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

    /// Catch bad regexes and dangling overlays at load time rather than mid-check.
    fn validate(&self) -> Result<()> {
        for (name, spec) in &self.vars {
            if let Some(pattern) = &spec.pattern {
                regex::Regex::new(pattern)
                    .with_context(|| format!("invalid regex for {name}: {pattern}"))?;
            }
        }

        for (env, overlay) in &self.envs {
            for (name, over) in &overlay.vars {
                // An overlay for an undeclared variable is almost always a typo,
                // and the variable would carry no description or source anyway.
                // A production-only variable is expressible as required = false
                // in [vars] plus required = true in the overlay.
                if !self.vars.contains_key(name) {
                    bail!("[envs.{env}.vars.{name}] overrides a variable that is not declared in [vars]");
                }
                if let Some(pattern) = &over.pattern {
                    regex::Regex::new(pattern).with_context(|| {
                        format!("invalid regex for {name} in environment {env}: {pattern}")
                    })?;
                }
            }
        }
        Ok(())
    }

    pub fn environments(&self) -> Vec<&str> {
        self.envs.keys().map(String::as_str).collect()
    }

    /// Base variables with the named environment's overlay applied.
    pub fn resolve(&self, env: Option<&str>) -> BTreeMap<String, VarSpec> {
        let mut resolved = self.vars.clone();

        let Some(overlay) = env.and_then(|e| self.envs.get(e)) else {
            return resolved;
        };

        for (name, over) in &overlay.vars {
            let Some(spec) = resolved.get_mut(name) else {
                continue; // validate() already rejected this case
            };
            if let Some(required) = over.required {
                spec.required = required;
            }
            if over.pattern.is_some() {
                spec.pattern = over.pattern.clone();
            }
            if over.one_of.is_some() {
                spec.one_of = over.one_of.clone();
            }
            if over.default.is_some() {
                spec.default = over.default.clone();
            }
        }

        resolved
    }

    /// Which environment applies.
    ///
    /// An explicit `--env` must name a real one: a typo there silently downgrades
    /// a production gate to the base schema, which is the failure this feature
    /// exists to prevent. An inferred name falls back to the base schema without
    /// complaint, because `.env.local` is a common filename that is not an
    /// environment.
    pub fn select_env(&self, explicit: Option<&str>, file: &Path) -> Result<Option<String>> {
        if let Some(name) = explicit {
            if !self.envs.contains_key(name) {
                let known = self.environments();
                let known = if known.is_empty() {
                    "none are declared".to_string()
                } else {
                    known.join(", ")
                };
                bail!("unknown environment `{name}` (schema declares: {known})");
            }
            return Ok(Some(name.to_string()));
        }

        Ok(infer_env(file).filter(|n| self.envs.contains_key(n)))
    }
}

/// `.env.production` implies `production`. `.env` implies nothing.
fn infer_env(file: &Path) -> Option<String> {
    let name = file.file_name()?.to_str()?;
    let rest = name.strip_prefix(".env.")?;
    (!rest.is_empty()).then(|| rest.to_string())
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

    const SQLITE_VS_POSTGRES: &str = r#"
        [vars.DATABASE_URL]
        required = true

        [envs.development.vars.DATABASE_URL]
        pattern = '^(sqlite:|file:)'

        [envs.production.vars.DATABASE_URL]
        pattern = '^postgres(ql)?://'
    "#;

    #[test]
    fn overlay_applies_the_named_environments_constraint() {
        let schema: Schema = toml::from_str(SQLITE_VS_POSTGRES).unwrap();

        let dev = schema.resolve(Some("development"));
        assert_eq!(
            dev["DATABASE_URL"].pattern.as_deref(),
            Some("^(sqlite:|file:)")
        );

        let prod = schema.resolve(Some("production"));
        assert_eq!(
            prod["DATABASE_URL"].pattern.as_deref(),
            Some("^postgres(ql)?://")
        );

        // No environment means the base schema, which pins nothing here.
        assert_eq!(schema.resolve(None)["DATABASE_URL"].pattern, None);
    }

    #[test]
    fn overlay_inherits_everything_it_does_not_mention() {
        let schema: Schema = toml::from_str(
            r#"
            [vars.API_KEY]
            required    = true
            description = "Vendor key"
            source      = "https://example.com/keys"
            secret      = true

            [envs.development.vars.API_KEY]
            required = false
            "#,
        )
        .unwrap();

        let dev = &schema.resolve(Some("development"))["API_KEY"];
        assert!(!dev.required, "overridden");
        assert!(dev.secret, "identity must survive");
        assert_eq!(dev.description.as_deref(), Some("Vendor key"));
        assert_eq!(dev.source.as_deref(), Some("https://example.com/keys"));
    }

    /// PRD §8.4: an overlay may override constraints, never identity. A
    /// credential is not sensitive in production and safe in development.
    #[test]
    fn overlay_cannot_override_identity_fields() {
        for field in ["secret = false", r#"description = "x""#, r#"source = "x""#] {
            let toml_str = format!("[vars.FOO]\nrequired = true\n\n[envs.dev.vars.FOO]\n{field}\n");
            let result: Result<Schema, _> = toml::from_str(&toml_str);
            assert!(result.is_err(), "{field} should be rejected in an overlay");
        }
    }

    #[test]
    fn overlay_for_an_undeclared_variable_is_an_error() {
        let schema: Schema =
            toml::from_str("[vars.FOO]\nrequired = true\n\n[envs.dev.vars.TYPOD]\nrequired = true")
                .unwrap();
        let err = schema.validate().unwrap_err().to_string();
        assert!(err.contains("TYPOD"), "{err}");
    }

    /// A typo in --env would silently downgrade a production gate to the base
    /// schema, which is the failure the feature exists to prevent.
    #[test]
    fn an_explicit_unknown_environment_is_an_error() {
        let schema: Schema = toml::from_str(SQLITE_VS_POSTGRES).unwrap();
        let err = schema
            .select_env(Some("prodcution"), Path::new(".env"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("prodcution"), "{err}");
        assert!(
            err.contains("production"),
            "should list the real ones: {err}"
        );
    }

    #[test]
    fn the_environment_is_inferred_from_the_filename() {
        let schema: Schema = toml::from_str(SQLITE_VS_POSTGRES).unwrap();
        assert_eq!(
            schema
                .select_env(None, Path::new(".env.production"))
                .unwrap(),
            Some("production".to_string())
        );
        assert_eq!(schema.select_env(None, Path::new(".env")).unwrap(), None);
    }

    /// .env.local is a common filename that is not an environment, so an inferred
    /// name that matches nothing falls back to the base schema without complaint.
    #[test]
    fn an_inferred_unknown_environment_falls_back_quietly() {
        let schema: Schema = toml::from_str(SQLITE_VS_POSTGRES).unwrap();
        assert_eq!(
            schema.select_env(None, Path::new(".env.local")).unwrap(),
            None
        );
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
