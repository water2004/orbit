//! Ordered set operations over a complete JAR-declared version string.
//!
//! A rule starts from `all` or `none`, then evaluates each operation from left
//! to right. Orbit assigns no release-stage meaning to author-chosen text.

use serde::{Deserialize, Serialize};

use crate::error::OrbitError;

const MAX_OPERATIONS: usize = 128;
const MAX_VALUE_BYTES: usize = 512;

/// Applied only when `orbit add` creates a brand-new package declaration.
pub const DEFAULT_NEW_PACKAGE_STRING: &str =
    "all; intersect not contains(i\"beta\"); intersect not contains(i\"snapshot\")";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VersionStringInitialSet {
    All,
    None,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionStringRule {
    pub initial: VersionStringInitialSet,
    pub operations: Vec<VersionStringOperation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum VersionStringOperation {
    Intersect {
        negated: bool,
        predicate: VersionStringPredicate,
    },
    Union {
        negated: bool,
        predicate: VersionStringPredicate,
    },
    Complement,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum VersionStringPredicate {
    Empty,
    Present,
    Equals { value: String, case_sensitive: bool },
    Contains { value: String, case_sensitive: bool },
    StartsWith { value: String, case_sensitive: bool },
    EndsWith { value: String, case_sensitive: bool },
}

impl Default for VersionStringRule {
    fn default() -> Self {
        Self {
            initial: VersionStringInitialSet::All,
            operations: Vec::new(),
        }
    }
}

impl VersionStringRule {
    pub fn parse(source: &str) -> Result<Self, OrbitError> {
        let mut parser = Parser::new(source);
        let initial = match parser.identifier()?.as_str() {
            "all" => VersionStringInitialSet::All,
            "none" => VersionStringInitialSet::None,
            other => {
                return Err(parser.error(&format!(
                    "string rule must start with 'all' or 'none', found '{other}'"
                )));
            }
        };
        let mut operations = Vec::new();
        loop {
            parser.skip_whitespace();
            if parser.finished() {
                break;
            }
            if !parser.consume_char(';') {
                return Err(parser.error("expected ';' between string operations"));
            }
            parser.skip_whitespace();
            if parser.finished() {
                return Err(parser.error("string rule cannot end with an empty operation"));
            }
            let operation = match parser.identifier()?.as_str() {
                "intersect" => VersionStringOperation::Intersect {
                    negated: parser.consume_keyword("not"),
                    predicate: parser.predicate()?,
                },
                "union" => VersionStringOperation::Union {
                    negated: parser.consume_keyword("not"),
                    predicate: parser.predicate()?,
                },
                "complement" => VersionStringOperation::Complement,
                other => {
                    return Err(parser.error(&format!("unknown string set operation '{other}'")));
                }
            };
            operations.push(operation);
            if operations.len() > MAX_OPERATIONS {
                return Err(parser.error("string rule contains too many operations"));
            }
        }
        let rule = Self {
            initial,
            operations,
        };
        rule.validate()?;
        Ok(rule)
    }

    pub fn matches(&self, value: &str) -> bool {
        self.operations.iter().fold(
            self.initial == VersionStringInitialSet::All,
            |selected, operation| match operation {
                VersionStringOperation::Intersect { negated, predicate } => {
                    selected && (predicate.matches(value) != *negated)
                }
                VersionStringOperation::Union { negated, predicate } => {
                    selected || (predicate.matches(value) != *negated)
                }
                VersionStringOperation::Complement => !selected,
            },
        )
    }

    pub fn validate(&self) -> Result<(), OrbitError> {
        if self.operations.len() > MAX_OPERATIONS {
            return Err(invalid("string rule contains too many operations"));
        }
        for operation in &self.operations {
            if let Some(predicate) = operation.predicate() {
                predicate.validate()?;
            }
        }
        Ok(())
    }

    pub fn canonical(&self) -> String {
        self.to_string()
    }
}

impl VersionStringOperation {
    fn predicate(&self) -> Option<&VersionStringPredicate> {
        match self {
            Self::Intersect { predicate, .. } | Self::Union { predicate, .. } => Some(predicate),
            Self::Complement => None,
        }
    }
}

impl std::fmt::Display for VersionStringRule {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self.initial {
            VersionStringInitialSet::All => "all",
            VersionStringInitialSet::None => "none",
        })?;
        for operation in &self.operations {
            write!(formatter, "; {operation}")?;
        }
        Ok(())
    }
}

impl std::fmt::Display for VersionStringOperation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Intersect { negated, predicate } => write!(
                formatter,
                "intersect {}{predicate}",
                if *negated { "not " } else { "" }
            ),
            Self::Union { negated, predicate } => write!(
                formatter,
                "union {}{predicate}",
                if *negated { "not " } else { "" }
            ),
            Self::Complement => formatter.write_str("complement"),
        }
    }
}

impl VersionStringPredicate {
    fn matches(&self, text: &str) -> bool {
        match self {
            Self::Empty => text.is_empty(),
            Self::Present => !text.is_empty(),
            Self::Equals {
                value,
                case_sensitive,
            } => compare(text, value, *case_sensitive, str::eq),
            Self::Contains {
                value,
                case_sensitive,
            } => compare(text, value, *case_sensitive, |left, right| {
                left.contains(right)
            }),
            Self::StartsWith {
                value,
                case_sensitive,
            } => compare(text, value, *case_sensitive, |left, right| {
                left.starts_with(right)
            }),
            Self::EndsWith {
                value,
                case_sensitive,
            } => compare(text, value, *case_sensitive, |left, right| {
                left.ends_with(right)
            }),
        }
    }

    fn validate(&self) -> Result<(), OrbitError> {
        let value = match self {
            Self::Equals { value, .. }
            | Self::Contains { value, .. }
            | Self::StartsWith { value, .. }
            | Self::EndsWith { value, .. } => Some(value),
            Self::Empty | Self::Present => None,
        };
        if let Some(value) = value {
            if value.is_empty() {
                return Err(invalid(
                    "version string predicates require a non-empty value",
                ));
            }
            if value.len() > MAX_VALUE_BYTES {
                return Err(invalid("string predicate value is too long"));
            }
            if value.chars().any(char::is_control) {
                return Err(invalid(
                    "string predicate values cannot contain control characters",
                ));
            }
        }
        Ok(())
    }
}

impl std::fmt::Display for VersionStringPredicate {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => formatter.write_str("empty"),
            Self::Present => formatter.write_str("present"),
            Self::Equals {
                value,
                case_sensitive,
            } => write!(formatter, "{}{}", insensitive(*case_sensitive), json(value)),
            Self::Contains {
                value,
                case_sensitive,
            } => write!(
                formatter,
                "contains({}{})",
                insensitive(*case_sensitive),
                json(value)
            ),
            Self::StartsWith {
                value,
                case_sensitive,
            } => write!(
                formatter,
                "starts_with({}{})",
                insensitive(*case_sensitive),
                json(value)
            ),
            Self::EndsWith {
                value,
                case_sensitive,
            } => write!(
                formatter,
                "ends_with({}{})",
                insensitive(*case_sensitive),
                json(value)
            ),
        }
    }
}

fn insensitive(case_sensitive: bool) -> &'static str {
    if case_sensitive { "" } else { "i" }
}

fn json(value: &str) -> String {
    serde_json::to_string(value).expect("serializing a string cannot fail")
}

fn compare(
    left: &str,
    right: &str,
    case_sensitive: bool,
    operation: impl Fn(&str, &str) -> bool,
) -> bool {
    if case_sensitive {
        operation(left, right)
    } else {
        operation(&left.to_lowercase(), &right.to_lowercase())
    }
}

struct Parser<'a> {
    source: &'a str,
    offset: usize,
}

impl<'a> Parser<'a> {
    fn new(source: &'a str) -> Self {
        Self { source, offset: 0 }
    }

    fn predicate(&mut self) -> Result<VersionStringPredicate, OrbitError> {
        self.skip_whitespace();
        if self.peek() == Some('"') {
            return Ok(VersionStringPredicate::Equals {
                value: self.string()?,
                case_sensitive: true,
            });
        }
        let identifier = self.identifier()?;
        match identifier.as_str() {
            "empty" => Ok(VersionStringPredicate::Empty),
            "present" => Ok(VersionStringPredicate::Present),
            "i" if self.peek_after_whitespace() == Some('"') => {
                self.skip_whitespace();
                Ok(VersionStringPredicate::Equals {
                    value: self.string()?,
                    case_sensitive: false,
                })
            }
            "contains" | "starts_with" | "ends_with" => {
                self.skip_whitespace();
                if !self.consume_char('(') {
                    return Err(self.error("expected '(' after string predicate"));
                }
                self.skip_whitespace();
                let case_sensitive = if self.peek() == Some('i') {
                    self.advance();
                    false
                } else {
                    true
                };
                let value = self.string()?;
                self.skip_whitespace();
                if !self.consume_char(')') {
                    return Err(self.error("expected ')' after string predicate value"));
                }
                Ok(match identifier.as_str() {
                    "contains" => VersionStringPredicate::Contains {
                        value,
                        case_sensitive,
                    },
                    "starts_with" => VersionStringPredicate::StartsWith {
                        value,
                        case_sensitive,
                    },
                    "ends_with" => VersionStringPredicate::EndsWith {
                        value,
                        case_sensitive,
                    },
                    _ => unreachable!(),
                })
            }
            _ => Err(self.error(&format!("unknown string predicate '{identifier}'"))),
        }
    }

    fn identifier(&mut self) -> Result<String, OrbitError> {
        self.skip_whitespace();
        let start = self.offset;
        while self
            .peek()
            .is_some_and(|character| character.is_ascii_alphanumeric() || character == '_')
        {
            self.advance();
        }
        if start == self.offset {
            return Err(self.error("expected an identifier"));
        }
        Ok(self.source[start..self.offset].to_ascii_lowercase())
    }

    fn string(&mut self) -> Result<String, OrbitError> {
        let start = self.offset;
        if self.peek() != Some('"') {
            return Err(self.error("string predicate values must be JSON-quoted strings"));
        }
        self.advance();
        let mut escaped = false;
        while let Some(character) = self.peek() {
            self.advance();
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                return serde_json::from_str(&self.source[start..self.offset])
                    .map_err(|error| self.error(&format!("invalid quoted string: {error}")));
            }
        }
        Err(self.error("unterminated quoted string"))
    }

    fn consume_char(&mut self, expected: char) -> bool {
        if self.peek() == Some(expected) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn consume_keyword(&mut self, keyword: &str) -> bool {
        self.skip_whitespace();
        let remaining = &self.source[self.offset..];
        let Some(candidate) = remaining.get(..keyword.len()) else {
            return false;
        };
        if !candidate.eq_ignore_ascii_case(keyword)
            || remaining[keyword.len()..]
                .chars()
                .next()
                .is_some_and(|character| character.is_ascii_alphanumeric() || character == '_')
        {
            return false;
        }
        self.offset += keyword.len();
        true
    }

    fn skip_whitespace(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.advance();
        }
    }

    fn finished(&self) -> bool {
        self.offset == self.source.len()
    }

    fn peek(&self) -> Option<char> {
        self.source[self.offset..].chars().next()
    }

    fn peek_after_whitespace(&self) -> Option<char> {
        self.source[self.offset..]
            .chars()
            .find(|character| !character.is_whitespace())
    }

    fn advance(&mut self) {
        if let Some(character) = self.peek() {
            self.offset += character.len_utf8();
        }
    }

    fn error(&self, message: &str) -> OrbitError {
        invalid(&format!(
            "invalid string rule at byte {}: {message}",
            self.offset
        ))
    }
}

fn invalid(message: &str) -> OrbitError {
    OrbitError::Other(anyhow::anyhow!(message.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordered_set_operations_are_evaluated_left_to_right() {
        let rule = VersionStringRule::parse(
            "all; intersect not contains(i\"beta\"); union \"beta-allowed\"",
        )
        .unwrap();
        assert!(!rule.matches("BETA"));
        assert!(rule.matches("beta-allowed"));
        assert!(rule.matches("release"));
    }

    #[test]
    fn quoted_literals_are_exact_and_i_literals_ignore_case() {
        let exact = VersionStringRule::parse("none; union \"Beta\"").unwrap();
        assert!(exact.matches("Beta"));
        assert!(!exact.matches("beta"));
        let insensitive = VersionStringRule::parse("none; union i\"Beta\"").unwrap();
        assert!(insensitive.matches("beta"));
    }

    #[test]
    fn canonical_form_preserves_operation_order() {
        let source = "all; intersect present; intersect not ends_with(i\"fabric\"); complement";
        assert_eq!(
            VersionStringRule::parse(source).unwrap().canonical(),
            source
        );
    }
}
