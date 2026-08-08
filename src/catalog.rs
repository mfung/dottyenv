use anyhow::{Context, Result};
use serde::Deserialize;

static CATALOG_TOML: &str = include_str!("../catalog.toml");

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    #[allow(dead_code)]
    pub catalog_version: u32,
    #[serde(default)]
    pub providers: Vec<Provider>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Provider {
    pub name: String,
    /// Matched against an observed value.
    #[serde(default)]
    pub prefixes: Vec<String>,
    /// Matched against a variable name, for when only a placeholder is present.
    #[serde(default)]
    pub name_matches: Vec<String>,
    pub pattern: Option<String>,
    pub source: Option<String>,
    #[serde(default)]
    pub secret: bool,
}

impl Provider {
    /// True when we have a real value and this provider's pattern rejects it.
    fn contradicts(&self, value: &str) -> bool {
        if value.is_empty() {
            return false;
        }
        match &self.pattern {
            Some(pattern) => regex::Regex::new(pattern).is_ok_and(|re| !re.is_match(value)),
            None => false,
        }
    }
}

impl Catalog {
    pub fn load() -> Result<Self> {
        toml::from_str(CATALOG_TOML).context("built-in catalog is malformed")
    }

    /// Match on the value first — an observed value is stronger evidence than a name.
    ///
    /// A name-based match is discarded when the observed value contradicts it. Without
    /// this, `DATABASE_URL=mysql://...` matches the Postgres entry on name alone and
    /// `init` writes a schema that rejects the very file it was generated from.
    pub fn find(&self, name: &str, value: &str) -> Option<&Provider> {
        self.by_value(value)
            .or_else(|| self.by_name(name).filter(|p| !p.contradicts(value)))
    }

    fn by_value(&self, value: &str) -> Option<&Provider> {
        if value.is_empty() {
            return None;
        }
        self.providers
            .iter()
            .find(|p| p.prefixes.iter().any(|prefix| value.starts_with(prefix)))
    }

    fn by_name(&self, name: &str) -> Option<&Provider> {
        self.providers
            .iter()
            .find(|p| p.name_matches.iter().any(|pat| glob_match(pat, name)))
    }
}

/// Deliberately minimal: exact match, or a single leading/trailing `*`.
fn glob_match(pattern: &str, name: &str) -> bool {
    match (pattern.strip_prefix('*'), pattern.strip_suffix('*')) {
        (Some(suffix), None) => name.ends_with(suffix),
        (None, Some(prefix)) => name.starts_with(prefix),
        _ => pattern == name,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_catalog_parses() {
        let catalog = Catalog::load().unwrap();
        assert!(!catalog.providers.is_empty());
    }

    #[test]
    fn every_catalog_pattern_is_a_valid_regex() {
        let catalog = Catalog::load().unwrap();
        for provider in &catalog.providers {
            if let Some(pattern) = &provider.pattern {
                regex::Regex::new(pattern)
                    .unwrap_or_else(|_| panic!("bad regex for {}: {pattern}", provider.name));
            }
        }
    }

    /// PRD §7.4: a pattern that anchors the end pins the key length, which breaks
    /// when a vendor changes format. Prefix-only means no trailing `$`.
    #[test]
    fn no_catalog_pattern_anchors_the_end() {
        let catalog = Catalog::load().unwrap();
        for provider in &catalog.providers {
            if let Some(pattern) = &provider.pattern {
                assert!(
                    !pattern.ends_with('$'),
                    "{} anchors the end: {pattern}",
                    provider.name
                );
            }
        }
    }

    #[test]
    fn matches_on_value_prefix() {
        let catalog = Catalog::load().unwrap();
        let provider = catalog.find("ANYTHING", "sk_test_abc123").unwrap();
        assert_eq!(provider.name, "Stripe secret key");
    }

    /// The common .env.example shape: names declared, values left blank.
    #[test]
    fn falls_back_to_name_when_no_value_is_present() {
        let catalog = Catalog::load().unwrap();
        let provider = catalog.find("STRIPE_SECRET_KEY", "").unwrap();
        assert!(provider.pattern.as_deref().unwrap().contains("sk"));
    }

    /// A non-empty placeholder is indistinguishable from a wrong value, so the
    /// name match is dropped rather than emitting a pattern the file already fails.
    #[test]
    fn drops_name_match_when_a_placeholder_fails_the_pattern() {
        let catalog = Catalog::load().unwrap();
        assert!(catalog.find("STRIPE_SECRET_KEY", "changeme").is_none());
    }

    #[test]
    fn identifies_databases_by_value() {
        let catalog = Catalog::load().unwrap();
        for (value, expected) in [
            ("postgres://localhost/app", "PostgreSQL connection string"),
            ("mysql://localhost/app", "MySQL/MariaDB connection string"),
            ("mariadb://localhost/app", "MySQL/MariaDB connection string"),
            ("mongodb+srv://cluster/app", "MongoDB connection string"),
            ("sqlite:///var/db/app.db", "SQLite database path"),
            ("file:./dev.db", "SQLite database path"),
        ] {
            let provider = catalog
                .find("DATABASE_URL", value)
                .unwrap_or_else(|| panic!("no provider matched {value}"));
            assert_eq!(provider.name, expected, "for value {value}");
        }
    }

    /// The bug this guards: DATABASE_URL used to match Postgres on name alone.
    #[test]
    fn name_match_is_discarded_when_the_value_contradicts_it() {
        let catalog = Catalog::load().unwrap();
        let provider = catalog.find("POSTGRES_URL", "mysql://localhost/app").unwrap();
        assert_eq!(provider.name, "MySQL/MariaDB connection string");
    }

    /// A bare path is a legitimate DATABASE_URL (SQLite). Guessing any pattern
    /// here would reject a valid config.
    #[test]
    fn makes_no_guess_for_an_ambiguous_database_url() {
        let catalog = Catalog::load().unwrap();
        assert!(catalog.find("DATABASE_URL", "./data/app.db").is_none());
        assert!(catalog.find("DATABASE_URL", "changeme").is_none());
    }

    #[test]
    fn returns_none_for_unknown_vars() {
        let catalog = Catalog::load().unwrap();
        assert!(catalog.find("MY_APP_SETTING", "42").is_none());
    }

    #[test]
    fn glob_matches_leading_and_trailing_wildcards() {
        assert!(glob_match("*_DATABASE_URL", "APP_DATABASE_URL"));
        assert!(glob_match("STRIPE_*", "STRIPE_SECRET_KEY"));
        assert!(glob_match("REDIS_URL", "REDIS_URL"));
        assert!(!glob_match("REDIS_URL", "REDIS_URL_REPLICA"));
    }
}
