//! Localization files (`*.lang.ron`) with string tables for UI and unit labels.

use std::collections::BTreeMap;

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

/// Largest accepted localization file in bytes.
pub const MAX_LOCALE_BYTES: usize = 512 * 1024;
/// Maximum number of translation entries in a single locale file.
pub const MAX_LOCALE_STRINGS: usize = 4096;
/// Maximum byte length of a translation key.
const MAX_KEY_BYTES: usize = 128;
/// Maximum character count of a language display name.
const MAX_NAME_CHARS: usize = 64;
/// Maximum ASCII length of a language code (e.g. `en`, `uk`, `pt-br`).
const MAX_CODE_LEN: usize = 16;

/// A parsed `*.lang.ron` translation catalog.
#[derive(Asset, TypePath, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocaleData {
    /// Short lowercase language tag, e.g. `"en"`, `"de"`, `"es"`, `"uk"`.
    pub code: String,
    /// Native display name shown in the language picker, e.g. `"English"`, `"Українська"`.
    pub name: String,
    /// Key-to-template translation table. Placeholders use `{name}` syntax.
    pub strings: BTreeMap<String, String>,
}

impl LocaleData {
    /// Parses and validates a `*.lang.ron` document.
    ///
    /// # Errors
    /// Returns an error if the file exceeds size limits, has malformed RON syntax, or fails
    /// code/name/key validation.
    pub fn parse_str(text: &str) -> Result<Self, String> {
        if text.len() > MAX_LOCALE_BYTES {
            return Err(format!(
                "locale file exceeds {MAX_LOCALE_BYTES} bytes ({} bytes)",
                text.len()
            ));
        }
        let data: Self =
            ron::from_str(text).map_err(|error| format!("invalid locale RON: {error}"))?;
        data.validate()?;
        Ok(data)
    }

    /// Parses and validates a `*.lang.ron` document for Bevy's asset loader.
    ///
    /// # Errors
    /// Returns an error if parsing or validation fails.
    pub fn from_ron(text: &str) -> Result<Self, BevyError> {
        Self::parse_str(text).map_err(Into::into)
    }

    /// Validates a language code (`2..=16` lowercase ASCII letters, digits, `-`, or `_`).
    ///
    /// # Errors
    /// Returns an error when `code` is empty, too long, or contains disallowed characters.
    pub fn validate_code(code: &str) -> Result<(), String> {
        if code.len() < 2 || code.len() > MAX_CODE_LEN {
            return Err(format!(
                "locale code `{code}` must be 2 to {MAX_CODE_LEN} ASCII characters"
            ));
        }
        let bytes = code.as_bytes();
        if !bytes[0].is_ascii_lowercase() {
            return Err(format!(
                "locale code `{code}` must start with a lowercase ASCII letter"
            ));
        }
        if !bytes.iter().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-' || *byte == b'_'
        }) {
            return Err(format!(
                "locale code `{code}` may only contain lowercase ASCII letters, digits, '-' or '_'"
            ));
        }
        Ok(())
    }

    /// Validates all fields of this locale catalog.
    ///
    /// # Errors
    /// Returns an error if any field violates the format constraints.
    pub fn validate(&self) -> Result<(), String> {
        Self::validate_code(&self.code)?;
        let trimmed_name = self.name.trim();
        if trimmed_name.is_empty()
            || trimmed_name.len() != self.name.len()
            || trimmed_name.chars().count() > MAX_NAME_CHARS
            || trimmed_name.chars().any(char::is_control)
        {
            return Err(format!("invalid locale display name `{}`", self.name));
        }
        if self.strings.len() > MAX_LOCALE_STRINGS {
            return Err(format!(
                "locale `{}` has {} strings (max {MAX_LOCALE_STRINGS})",
                self.code,
                self.strings.len()
            ));
        }
        for key in self.strings.keys() {
            if key.is_empty() || key.len() > MAX_KEY_BYTES || key.chars().any(char::is_control) {
                return Err(format!("invalid locale key `{key}` in `{}`", self.code));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    fn placeholders(template: &str) -> BTreeSet<&str> {
        let mut out = BTreeSet::new();
        let mut rest = template;
        while let Some((_, after)) = rest.split_once('{') {
            let Some((name, tail)) = after.split_once('}') else {
                break;
            };
            out.insert(name);
            rest = tail;
        }
        out
    }

    #[test]
    fn bundled_locales_are_valid_and_complete() {
        let bundled = [
            ("en", include_str!("../../../assets/locales/en.lang.ron")),
            ("de", include_str!("../../../assets/locales/de.lang.ron")),
            ("es", include_str!("../../../assets/locales/es.lang.ron")),
            ("uk", include_str!("../../../assets/locales/uk.lang.ron")),
        ];
        let mut parsed = Vec::new();
        for (expected_code, text) in bundled {
            let locale = LocaleData::parse_str(text)
                .unwrap_or_else(|error| panic!("locale {expected_code}: {error}"));
            assert_eq!(locale.code, expected_code);
            parsed.push(locale);
        }
        let english = &parsed[0];
        let expected_keys: BTreeSet<&str> = english.strings.keys().map(String::as_str).collect();
        assert!(!expected_keys.is_empty());
        for locale in &parsed[1..] {
            let keys: BTreeSet<&str> = locale.strings.keys().map(String::as_str).collect();
            let missing: Vec<_> = expected_keys.difference(&keys).copied().collect();
            let extra: Vec<_> = keys.difference(&expected_keys).copied().collect();
            assert!(
                missing.is_empty() && extra.is_empty(),
                "locale `{}` key mismatch: missing={missing:?}, extra={extra:?}",
                locale.code
            );
            for (key, en_val) in &english.strings {
                let loc_val = &locale.strings[key];
                assert_eq!(
                    placeholders(en_val),
                    placeholders(loc_val),
                    "placeholder mismatch in `{}` for key `{key}`",
                    locale.code
                );
            }
        }
    }

    #[test]
    fn rejects_invalid_locale_documents() {
        for bad in [
            "()",
            "(code: \"E\", name: \"Bad\", strings: {})",
            "(code: \"EN\", name: \"Bad\", strings: {})",
            "(code: \"en\", name: \"\", strings: {})",
            "(code: \"en\", name: \" English \", strings: {})",
            "(code: \"en\", name: \"English\", strings: {\"\": \"empty\"})",
            "(code: \"en\", name: \"English\", strings: {}, extra: 1)",
        ] {
            assert!(LocaleData::parse_str(bad).is_err(), "accepted {bad}");
        }
    }
}
