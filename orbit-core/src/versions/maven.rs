//! Maven-style version ordering and range parsing used by Forge and NeoForge.

use std::cmp::Ordering;
use std::hash::{Hash, Hasher};

use pubgrub::Ranges;

use super::Version;

#[derive(Debug, Clone)]
pub struct MavenVersion {
    raw: String,
    items: Vec<Item>,
}

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
enum Item {
    Number(String),
    Qualifier(String),
}

impl MavenVersion {
    pub fn parse(raw: &str) -> Self {
        Self {
            raw: raw.to_string(),
            items: tokenize(raw),
        }
    }
}

impl std::fmt::Display for MavenVersion {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.raw)
    }
}

impl PartialEq for MavenVersion {
    fn eq(&self, other: &Self) -> bool {
        self.items == other.items
    }
}

impl Eq for MavenVersion {}

impl Hash for MavenVersion {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.items.hash(state);
    }
}

impl PartialOrd for MavenVersion {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for MavenVersion {
    fn cmp(&self, other: &Self) -> Ordering {
        let length = self.items.len().max(other.items.len());
        for index in 0..length {
            let ordering = compare_optional(self.items.get(index), other.items.get(index));
            if ordering != Ordering::Equal {
                return ordering;
            }
        }
        Ordering::Equal
    }
}

fn tokenize(raw: &str) -> Vec<Item> {
    let mut items = Vec::new();
    let mut token = String::new();
    let mut token_is_numeric = None;
    for character in raw.trim().chars() {
        if matches!(character, '.' | '-' | '_' | '+') {
            push_token(&mut items, &mut token);
            token_is_numeric = None;
            continue;
        }
        let numeric = character.is_ascii_digit();
        if token_is_numeric.is_some_and(|current| current != numeric) {
            push_token(&mut items, &mut token);
        }
        token_is_numeric = Some(numeric);
        token.push(character.to_ascii_lowercase());
    }
    push_token(&mut items, &mut token);
    while matches!(
        items.last(),
        Some(Item::Number(number)) if number == "0"
    ) || matches!(
        items.last(),
        Some(Item::Qualifier(qualifier)) if qualifier.is_empty()
    ) {
        items.pop();
    }
    items
}

fn push_token(items: &mut Vec<Item>, token: &mut String) {
    if token.is_empty() {
        return;
    }
    if token.bytes().all(|character| character.is_ascii_digit()) {
        let normalized = token.trim_start_matches('0');
        items.push(Item::Number(if normalized.is_empty() {
            "0".to_string()
        } else {
            normalized.to_string()
        }));
    } else {
        items.push(Item::Qualifier(normalize_qualifier(token)));
    }
    token.clear();
}

fn normalize_qualifier(qualifier: &str) -> String {
    match qualifier {
        "a" => "alpha",
        "b" => "beta",
        "m" => "milestone",
        "cr" => "rc",
        "ga" | "final" | "release" => "",
        other => other,
    }
    .to_string()
}

fn compare_optional(left: Option<&Item>, right: Option<&Item>) -> Ordering {
    match (left, right) {
        (Some(Item::Number(left)), Some(Item::Number(right))) => compare_numbers(left, right),
        (Some(Item::Qualifier(left)), Some(Item::Qualifier(right))) => {
            compare_qualifiers(left, right)
        }
        (Some(Item::Number(_)), Some(Item::Qualifier(_))) => Ordering::Greater,
        (Some(Item::Qualifier(_)), Some(Item::Number(_))) => Ordering::Less,
        (Some(Item::Number(left)), None) => compare_numbers(left, "0"),
        (None, Some(Item::Number(right))) => compare_numbers("0", right),
        (Some(Item::Qualifier(left)), None) => compare_qualifiers(left, ""),
        (None, Some(Item::Qualifier(right))) => compare_qualifiers("", right),
        (None, None) => Ordering::Equal,
    }
}

fn compare_numbers(left: &str, right: &str) -> Ordering {
    left.len().cmp(&right.len()).then_with(|| left.cmp(right))
}

fn compare_qualifiers(left: &str, right: &str) -> Ordering {
    qualifier_key(left).cmp(&qualifier_key(right))
}

fn qualifier_key(qualifier: &str) -> (u8, &str) {
    let rank = match qualifier {
        "alpha" => 0,
        "beta" => 1,
        "milestone" => 2,
        "rc" => 3,
        "snapshot" => 4,
        "" => 5,
        "sp" => 6,
        _ => 7,
    };
    (rank, qualifier)
}

pub fn parse_constraint(raw: &str) -> Ranges<Version> {
    let raw = raw.trim();
    if raw.is_empty() || raw == "*" {
        return Ranges::full();
    }
    if !matches!(raw.as_bytes().first(), Some(b'[' | b'(')) {
        return Ranges::singleton(maven_version(raw));
    }

    let mut result = Ranges::empty();
    let mut remaining = raw;
    while !remaining.trim_start().is_empty() {
        remaining = remaining.trim_start();
        let Some(opening) = remaining.chars().next() else {
            break;
        };
        if !matches!(opening, '[' | '(') {
            return Ranges::singleton(maven_version(raw));
        }
        let Some(end) = remaining.find([']', ')']) else {
            return Ranges::singleton(maven_version(raw));
        };
        let segment = &remaining[..=end];
        result = result.union(&parse_segment(segment));
        remaining = remaining[end + 1..].trim_start_matches(',');
    }
    result
}

fn parse_segment(segment: &str) -> Ranges<Version> {
    let lower_inclusive = segment.starts_with('[');
    let upper_inclusive = segment.ends_with(']');
    let body = &segment[1..segment.len() - 1];
    let Some((lower, upper)) = body.split_once(',') else {
        return Ranges::singleton(maven_version(body.trim()));
    };
    let lower = lower.trim();
    let upper = upper.trim();
    let lower_range = if lower.is_empty() {
        Ranges::full()
    } else if lower_inclusive {
        Ranges::higher_than(maven_version(lower))
    } else {
        Ranges::strictly_higher_than(maven_version(lower))
    };
    let upper_range = if upper.is_empty() {
        Ranges::full()
    } else if upper_inclusive {
        Ranges::lower_than(maven_version(upper))
    } else {
        Ranges::strictly_lower_than(maven_version(upper))
    };
    lower_range.intersection(&upper_range)
}

fn maven_version(raw: &str) -> Version {
    Version::Maven(MavenVersion::parse(raw))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version(raw: &str) -> Version {
        maven_version(raw)
    }

    #[test]
    fn compares_numeric_versions_numerically() {
        assert!(version("47.10") > version("47.2"));
        assert_eq!(version("47"), version("47.0.0"));
        assert!(version("21.1.0-beta") < version("21.1.0"));
        assert!(version("21.1.0") < version("21.1.0-sp"));
    }

    #[test]
    fn parses_open_and_closed_ranges() {
        let range = parse_constraint("[47,48)");
        assert!(range.contains(&version("47")));
        assert!(range.contains(&version("47.2.1")));
        assert!(!range.contains(&version("48")));

        let open = parse_constraint("[21,)");
        assert!(open.contains(&version("21.1")));
        assert!(!open.contains(&version("20.9")));
    }

    #[test]
    fn parses_unions_and_exact_ranges() {
        let range = parse_constraint("(,20],[21,)");
        assert!(range.contains(&version("19")));
        assert!(!range.contains(&version("20.5")));
        assert!(range.contains(&version("21")));

        let exact = parse_constraint("[47.2.0]");
        assert!(exact.contains(&version("47.2")));
        assert!(!exact.contains(&version("47.3")));
    }
}
