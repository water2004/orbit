//! Presentation parser for Core's ordered suffix-set rule.

use anyhow::{Result, anyhow, bail};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SuffixRuleDraft {
    pub initial_all: bool,
    pub operations: Vec<SuffixOperationDraft>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SuffixOperationDraft {
    pub operator: SuffixSetOperator,
    pub negated: bool,
    pub predicate: SuffixPredicate,
    pub value: Option<String>,
    pub case_sensitive: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SuffixSetOperator {
    Intersect,
    Union,
    Complement,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SuffixPredicate {
    Empty,
    Present,
    Equals,
    Contains,
    StartsWith,
    EndsWith,
}

impl Default for SuffixRuleDraft {
    fn default() -> Self {
        Self {
            initial_all: true,
            operations: Vec::new(),
        }
    }
}

impl Default for SuffixOperationDraft {
    fn default() -> Self {
        Self {
            operator: SuffixSetOperator::Intersect,
            negated: false,
            predicate: SuffixPredicate::Contains,
            value: None,
            case_sensitive: false,
        }
    }
}

impl SuffixRuleDraft {
    pub fn parse(source: &str) -> Result<Self> {
        let mut parser = Parser::new(source);
        let initial_all = match parser.identifier()?.as_str() {
            "all" => true,
            "none" => false,
            value => bail!("suffix rule must start with all or none, found '{value}'"),
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
                "intersect" => SuffixSetOperator::Intersect,
                "union" => SuffixSetOperator::Union,
                "complement" => {
                    operations.push(SuffixOperationDraft {
                        operator: SuffixSetOperator::Complement,
                        ..SuffixOperationDraft::default()
                    });
                    continue;
                }
                value => bail!("unsupported suffix operation '{value}'"),
            };
            let negated = parser.consume_keyword("not");
            let (predicate, value, case_sensitive) = parser.predicate()?;
            operations.push(SuffixOperationDraft {
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

impl SuffixOperationDraft {
    pub fn expression(&self) -> Option<String> {
        if self.operator == SuffixSetOperator::Complement {
            return Some("complement".to_string());
        }
        let mut expression = match self.operator {
            SuffixSetOperator::Intersect => "intersect ".to_string(),
            SuffixSetOperator::Union => "union ".to_string(),
            SuffixSetOperator::Complement => unreachable!(),
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
            SuffixPredicate::Empty => "empty".to_string(),
            SuffixPredicate::Present => "present".to_string(),
            SuffixPredicate::Equals => format!(
                "{}{}",
                if self.case_sensitive { "" } else { "i" },
                quoted(value?)
            ),
            SuffixPredicate::Contains => format!(
                "contains({}{})",
                if self.case_sensitive { "" } else { "i" },
                quoted(value?)
            ),
            SuffixPredicate::StartsWith => format!(
                "starts_with({}{})",
                if self.case_sensitive { "" } else { "i" },
                quoted(value?)
            ),
            SuffixPredicate::EndsWith => format!(
                "ends_with({}{})",
                if self.case_sensitive { "" } else { "i" },
                quoted(value?)
            ),
        })
    }

    pub fn needs_value(&self) -> bool {
        self.operator != SuffixSetOperator::Complement
            && !matches!(
                self.predicate,
                SuffixPredicate::Empty | SuffixPredicate::Present
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

    fn predicate(&mut self) -> Result<(SuffixPredicate, Option<String>, bool)> {
        self.skip_whitespace();
        if self.peek() == Some('"') {
            return Ok((SuffixPredicate::Equals, Some(self.string()?), true));
        }
        let identifier = self.identifier()?;
        match identifier.as_str() {
            "empty" => Ok((SuffixPredicate::Empty, None, false)),
            "present" => Ok((SuffixPredicate::Present, None, false)),
            "i" if self.peek_after_whitespace() == Some('"') => {
                self.skip_whitespace();
                Ok((SuffixPredicate::Equals, Some(self.string()?), false))
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
                    "contains" => SuffixPredicate::Contains,
                    "starts_with" => SuffixPredicate::StartsWith,
                    "ends_with" => SuffixPredicate::EndsWith,
                    _ => unreachable!(),
                };
                Ok((predicate, Some(value), case_sensitive))
            }
            value => bail!("unsupported suffix predicate '{value}'"),
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
            SuffixRuleDraft::parse(source)
                .unwrap()
                .expression()
                .unwrap(),
            source
        );
    }
}
