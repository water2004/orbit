//! JSON parsing for loader-owned resources embedded in mod JARs.
//!
//! Fabric Loader's metadata reader accepts raw U+0000–U+001F control
//! characters inside quoted strings. Standard JSON requires those characters
//! to be escaped, so `serde_json` rejects them. This crate performs only that
//! one compatibility transformation and leaves every other JSON rule strict.

use std::fmt::Write as _;

use serde::de::DeserializeOwned;

/// Deserialize loader-owned JSON while accepting unescaped control characters
/// inside double-quoted strings.
pub fn from_str<T: DeserializeOwned>(input: &str) -> Result<T, serde_json::Error> {
    match serde_json::from_str(input) {
        Ok(value) => Ok(value),
        Err(strict_error) => {
            let Some(escaped) = escape_string_control_characters(input) else {
                return Err(strict_error);
            };
            serde_json::from_str(&escaped)
        }
    }
}

/// Byte-slice variant of [`from_str`]. Invalid UTF-8 remains a strict JSON
/// error; loader metadata is UTF-8 and is never decoded lossily.
pub fn from_slice<T: DeserializeOwned>(input: &[u8]) -> Result<T, serde_json::Error> {
    match serde_json::from_slice(input) {
        Ok(value) => Ok(value),
        Err(strict_error) => {
            let Ok(input) = std::str::from_utf8(input) else {
                return Err(strict_error);
            };
            let Some(escaped) = escape_string_control_characters(input) else {
                return Err(strict_error);
            };
            serde_json::from_str(&escaped)
        }
    }
}

fn escape_string_control_characters(input: &str) -> Option<String> {
    let mut output = String::new();
    let mut in_string = false;
    let mut escaped = false;
    let mut changed = false;

    for character in input.chars() {
        if !in_string {
            output.push(character);
            if character == '"' {
                in_string = true;
            }
            continue;
        }

        if escaped {
            output.push(character);
            escaped = false;
            continue;
        }

        match character {
            '\\' => {
                output.push(character);
                escaped = true;
            }
            '"' => {
                output.push(character);
                in_string = false;
            }
            '\u{0008}' => {
                output.push_str("\\b");
                changed = true;
            }
            '\t' => {
                output.push_str("\\t");
                changed = true;
            }
            '\n' => {
                output.push_str("\\n");
                changed = true;
            }
            '\u{000c}' => {
                output.push_str("\\f");
                changed = true;
            }
            '\r' => {
                output.push_str("\\r");
                changed = true;
            }
            control if control <= '\u{001f}' => {
                write!(output, "\\u{:04x}", control as u32)
                    .expect("writing to a String cannot fail");
                changed = true;
            }
            other => output.push(other),
        }
    }

    changed.then_some(output)
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    use super::*;

    #[derive(Debug, Deserialize, PartialEq, Eq)]
    struct Document {
        name: String,
    }

    #[test]
    fn accepts_unescaped_control_characters_inside_strings() {
        let parsed: Document = from_str("{\"name\":\"line one\nline two\t\u{0001}\"}").unwrap();

        assert_eq!(
            parsed,
            Document {
                name: "line one\nline two\t\u{0001}".to_string(),
            }
        );
    }

    #[test]
    fn accepts_controls_in_property_names_without_changing_string_boundaries() {
        let parsed: serde_json::Value = from_str("{\"na\nme\":\"escaped \\\" quote\"}").unwrap();

        assert_eq!(parsed["na\nme"], "escaped \" quote");
    }

    #[test]
    fn leaves_other_nonstandard_json_syntax_invalid() {
        for input in [
            "{\"name\":\"value\",}",
            "{name:\"value\"}",
            "{\"name\":'value'}",
            "{\"name\":\"value\" // comment\n}",
        ] {
            assert!(from_str::<Document>(input).is_err(), "{input}");
        }
    }

    #[test]
    fn controls_outside_strings_remain_invalid() {
        assert!(from_str::<Document>("{\u{0001}\"name\":\"value\"}").is_err());
    }

    #[test]
    fn byte_parser_does_not_decode_invalid_utf8_lossily() {
        assert!(from_slice::<Document>(b"{\"name\":\"\xff\"}").is_err());
    }
}
