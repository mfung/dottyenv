use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::path::Path;

/// Parse a .env file into key/value pairs.
///
/// Deliberately lenient: unparseable lines are skipped rather than fatal, because
/// refusing to run on a file with one odd line would be worse than ignoring it.
pub fn parse(text: &str) -> BTreeMap<String, String> {
    let mut vars = BTreeMap::new();

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let line = line.strip_prefix("export ").unwrap_or(line);
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };

        let key = key.trim();
        if key.is_empty() {
            continue;
        }

        vars.insert(key.to_string(), unquote(value.trim()).to_string());
    }

    vars
}

pub fn load(path: &Path) -> Result<BTreeMap<String, String>> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("could not read env file at {}", path.display()))?;
    Ok(parse(&text))
}

fn unquote(value: &str) -> &str {
    let bytes = value.as_bytes();
    if bytes.len() >= 2
        && (bytes[0] == b'"' || bytes[0] == b'\'')
        && bytes[0] == bytes[bytes.len() - 1]
    {
        &value[1..value.len() - 1]
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_basic_pairs() {
        let vars = parse("FOO=bar\nBAZ=qux");
        assert_eq!(vars["FOO"], "bar");
        assert_eq!(vars["BAZ"], "qux");
    }

    #[test]
    fn skips_comments_and_blanks() {
        let vars = parse("# a comment\n\nFOO=bar\n");
        assert_eq!(vars.len(), 1);
    }

    #[test]
    fn strips_export_prefix() {
        let vars = parse("export FOO=bar");
        assert_eq!(vars["FOO"], "bar");
    }

    #[test]
    fn strips_matching_quotes() {
        let vars = parse("A=\"quoted\"\nB='single'\nC=\"mismatched'");
        assert_eq!(vars["A"], "quoted");
        assert_eq!(vars["B"], "single");
        assert_eq!(vars["C"], "\"mismatched'");
    }

    #[test]
    fn keeps_equals_signs_in_values() {
        let vars = parse("TOKEN=abc==");
        assert_eq!(vars["TOKEN"], "abc==");
    }

    #[test]
    fn allows_empty_values() {
        let vars = parse("EMPTY=");
        assert_eq!(vars["EMPTY"], "");
    }
}
