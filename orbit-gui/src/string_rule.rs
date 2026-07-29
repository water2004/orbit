//! Presentation parser for Core's ordered version-string set rule.

use anyhow::{Result, anyhow, bail};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StringRuleDraft {
    pub initial_all: bool,
    pub operations: Vec<StringOperationDraft>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StringOperationDraft {
    pub operator: StringSetOperator,
    pub negated: bool,
    pub predicate: StringPredicate,
    pub value: Option<String>,
    pub case_sensitive: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StringSetOperator {
    Intersect,
    Union,
    Complement,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StringPredicate {
    Empty,
    Present,
    Equals,
    Contains,
    StartsWith,
    EndsWith,
}

impl Default for StringRuleDraft {
    fn default() -> Self {
        Self {
            initial_all: true,
            operations: Vec::new(),
        }
    }
}

impl Default for StringOperationDraft {
    fn default() -> Self {
        Self {
            operator: StringSetOperator::Intersect,
            negated: false,
            predicate: StringPredicate::Contains,
            value: None,
            case_sensitive: false,
        }
    }
}

impl StringRuleDraft {
    pub fn parse(source: &str) -> Result<Self> {
        let mut parser = Parser::new(source);
        let initial_all = match parser.identifier()?.as_str() {
            "all" => true,
            "none" => false,
            value => bail!("string rule must start with all or none, found '{value}'"),
        };
        let mut operations = Vec::new();
        loop {
            parser.skip_whitespace();
            if parser.finished() {
                break;
            }
            if !parser.consume_char(';') {
                bail!("expected ';' at byte {}", parser.offset);
            }
            let operator = match parser.identifier()?.as_str() {
                "intersect" => StringSetOperator::Intersect,
                "union" => StringSetOperator::Union,
                "complement" => {
                    operations.push(StringOperationDraft {
                        operator: StringSetOperator::Complement,
                        ..StringOperationDraft::default()
                    });
                    continue;
                }
                value => bail!("unsupported string operation '{value}'"),
            };
            let negated = parser.consume_keyword("not");
            let (predicate, value, case_sensitive) = parser.predicate()?;
            operations.push(StringOperationDraft {
                operator,
                negated,
                predicate,
                value,
                case_sensitive,
            });
        }
        Ok(Self {
            initial_all,
            operations,
        })
    }

    pub fn expression(&self) -> Option<String> {
        let mut expression = if self.initial_all { "all" } else { "none" }.to_string();
        for operation in &self.operations {
            expression.push_str("; ");
            expression.push_str(&operation.expression()?);
        }
        Some(expression)
    }
}

impl StringOperationDraft {
    pub fn expression(&self) -> Option<String> {
        if self.operator == StringSetOperator::Complement {
            return Some("complement".to_string());
        }
        let mut expression = match self.operator {
            StringSetOperator::Intersect => "intersect ".to_string(),
            StringSetOperator::Union => "union ".to_string(),
            StringSetOperator::Complement => unreachable!(),
        };
        if self.negated {
            expression.push_str("not ");
        }
        expression.push_str(&self.predicate_expression()?);
        Some(expression)
    }

    fn predicate_expression(&self) -> Option<String> {
        let value = self.value.as_deref().filter(|value| !value.is_empty());
        Some(match self.predicate {
            StringPredicate::Empty => "empty".to_string(),
            StringPredicate::Present => "present".to_string(),
            StringPredicate::Equals => format!(
                "{}{}",
                if self.case_sensitive { "" } else { "i" },
                quoted(value?)
            ),
            StringPredicate::Contains => format!(
                "contains({}{})",
                if self.case_sensitive { "" } else { "i" },
                quoted(value?)
            ),
            StringPredicate::StartsWith => format!(
                "starts_with({}{})",
                if self.case_sensitive { "" } else { "i" },
                quoted(value?)
            ),
            StringPredicate::EndsWith => format!(
                "ends_with({}{})",
                if self.case_sensitive { "" } else { "i" },
                quoted(value?)
            ),
        })
    }

    pub fn needs_value(&self) -> bool {
        self.operator != StringSetOperator::Complement
            && !matches!(
                self.predicate,
                StringPredicate::Empty | StringPredicate::Present
            )
    }
}

fn quoted(value: &str) -> String {
    serde_json::to_string(value).expect("serializing a string cannot fail")
}

struct Parser<'a> {
    source: &'a str,
    offset: usize,
}

impl<'a> Parser<'a> {
    fn new(source: &'a str) -> Self {
        Self { source, offset: 0 }
    }

    fn predicate(&mut self) -> Result<(StringPredicate, Option<String>, bool)> {
        self.skip_whitespace();
        if self.peek() == Some('"') {
            return Ok((StringPredicate::Equals, Some(self.string()?), true));
        }
        let identifier = self.identifier()?;
        match identifier.as_str() {
            "empty" => Ok((StringPredicate::Empty, None, false)),
            "present" => Ok((StringPredicate::Present, None, false)),
            "i" if self.peek_after_whitespace() == Some('"') => {
                self.skip_whitespace();
                Ok((StringPredicate::Equals, Some(self.string()?), false))
            }
            "contains" | "starts_with" | "ends_with" => {
                self.skip_whitespace();
                if !self.consume_char('(') {
                    bail!("expected '(' at byte {}", self.offset);
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
                    bail!("expected ')' at byte {}", self.offset);
                }
                let predicate = match identifier.as_str() {
                    "contains" => StringPredicate::Contains,
                    "starts_with" => StringPredicate::StartsWith,
                    "ends_with" => StringPredicate::EndsWith,
                    _ => unreachable!(),
                };
                Ok((predicate, Some(value), case_sensitive))
            }
            value => bail!("unsupported string predicate '{value}'"),
        }
    }

    fn identifier(&mut self) -> Result<String> {
        self.skip_whitespace();
        let start = self.offset;
        while self
            .peek()
            .is_some_and(|character| character.is_ascii_alphanumeric() || character == '_')
        {
            self.advance();
        }
        if start == self.offset {
            bail!("expected identifier at byte {}", self.offset);
        }
        Ok(self.source[start..self.offset].to_ascii_lowercase())
    }

    fn string(&mut self) -> Result<String> {
        let start = self.offset;
        if self.peek() != Some('"') {
            bail!("expected quoted string at byte {}", self.offset);
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
                    .map_err(|error| anyhow!("invalid quoted string: {error}"));
            }
        }
        bail!("unterminated quoted string")
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

    fn consume_char(&mut self, expected: char) -> bool {
        if self.peek() == Some(expected) {
            self.advance();
            true
        } else {
            false
        }
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordered_rule_roundtrips_for_visual_editing() {
        let source = "all; intersect not contains(i\"beta\"); union \"allowed\"; complement";
        assert_eq!(
            StringRuleDraft::parse(source)
                .unwrap()
                .expression()
                .unwrap(),
            source
        );
    }
}
