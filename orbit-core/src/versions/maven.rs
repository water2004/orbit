//! Maven `ComparableVersion` ordering and version ranges used by Forge-family loaders.

use std::cmp::Ordering;
use std::hash::{Hash, Hasher};

use pubgrub::Ranges;

use super::{CorePosition, Version};

#[derive(Debug, Clone)]
pub struct MavenVersion {
    raw: String,
    items: Vec<Item>,
    position: CorePosition,
}

#[derive(Debug, Clone, Hash)]
enum Item {
    Number(String),
    Qualifier(String),
    List(Vec<Item>),
    Combination { qualifier: String, number: String },
}

impl MavenVersion {
    pub fn parse(raw: &str) -> Self {
        Self {
            raw: raw.to_string(),
            items: parse_items(raw),
            position: CorePosition::Concrete,
        }
    }

    fn before_core(&self) -> Option<Self> {
        self.boundary(CorePosition::Before)
    }

    fn after_core(&self) -> Option<Self> {
        self.boundary(CorePosition::After)
    }

    fn boundary(&self, position: CorePosition) -> Option<Self> {
        let core = super::numeric_core(&self.raw)?;
        let raw = core.join(".");
        Some(Self {
            items: parse_items(&raw),
            raw,
            position,
        })
    }

    pub(super) fn cmp_precedence(&self, other: &Self) -> Ordering {
        match (
            super::numeric_core(&self.raw),
            super::numeric_core(&other.raw),
        ) {
            (Some(left), Some(right)) => super::cmp_numeric_core(&left, &right),
            _ => compare_lists(&self.items, &other.items),
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
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for MavenVersion {}

impl Hash for MavenVersion {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.position.hash(state);
        if self.position == CorePosition::Concrete {
            self.items.hash(state);
        } else {
            super::numeric_core(&self.raw).hash(state);
        }
    }
}

impl PartialOrd for MavenVersion {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for MavenVersion {
    fn cmp(&self, other: &Self) -> Ordering {
        let core = self.cmp_precedence(other);
        if core != Ordering::Equal {
            return core;
        }
        match (self.position, other.position) {
            (CorePosition::Before, CorePosition::Before)
            | (CorePosition::After, CorePosition::After) => Ordering::Equal,
            (CorePosition::Before, _) | (_, CorePosition::After) => Ordering::Less,
            (CorePosition::After, _) | (_, CorePosition::Before) => Ordering::Greater,
            (CorePosition::Concrete, CorePosition::Concrete) => {
                compare_lists(&self.items, &other.items)
            }
        }
    }
}

#[derive(Clone)]
enum BuilderItem {
    Value(Item),
    List(usize),
}

#[derive(Default)]
struct ListBuilder {
    items: Vec<BuilderItem>,
}

fn parse_items(raw: &str) -> Vec<Item> {
    let characters: Vec<_> = raw.to_lowercase().chars().collect();
    let mut lists = vec![ListBuilder::default()];
    let mut current = 0;
    let mut is_digit = false;
    let mut is_combination = false;
    let mut start = 0;

    for index in 0..characters.len() {
        let character = characters[index];
        match character {
            '.' => {
                if index == start {
                    push_value(&mut lists, current, Item::Number("0".to_string()));
                } else {
                    push_value(
                        &mut lists,
                        current,
                        parse_item(
                            is_combination,
                            is_digit,
                            substring(&characters, start, index),
                        ),
                    );
                }
                is_combination = false;
                start = index + 1;
            }
            '-' => {
                if index == start {
                    push_value(&mut lists, current, Item::Number("0".to_string()));
                } else {
                    if !is_digit
                        && characters
                            .get(index + 1)
                            .is_some_and(|next| next.is_ascii_digit())
                    {
                        is_combination = true;
                        continue;
                    }
                    push_value(
                        &mut lists,
                        current,
                        parse_item(
                            is_combination,
                            is_digit,
                            substring(&characters, start, index),
                        ),
                    );
                }
                start = index + 1;
                if !lists[current].items.is_empty() {
                    current = push_list(&mut lists, current);
                }
                is_combination = false;
            }
            character if character.is_ascii_digit() => {
                if !is_digit && index > start {
                    is_combination = true;
                    if !lists[current].items.is_empty() {
                        current = push_list(&mut lists, current);
                    }
                }
                is_digit = true;
            }
            _ => {
                if is_digit && index > start {
                    push_value(
                        &mut lists,
                        current,
                        parse_item(is_combination, true, substring(&characters, start, index)),
                    );
                    start = index;
                    current = push_list(&mut lists, current);
                    is_combination = false;
                }
                is_digit = false;
            }
        }
    }

    if characters.len() > start {
        if !is_digit && !lists[current].items.is_empty() {
            current = push_list(&mut lists, current);
        }
        push_value(
            &mut lists,
            current,
            parse_item(
                is_combination,
                is_digit,
                substring(&characters, start, characters.len()),
            ),
        );
    }

    let mut items = materialize_list(0, &lists);
    normalize_list(&mut items);
    items
}

fn substring(characters: &[char], start: usize, end: usize) -> String {
    characters[start..end].iter().collect()
}

fn push_value(lists: &mut [ListBuilder], current: usize, item: Item) {
    lists[current].items.push(BuilderItem::Value(item));
}

fn push_list(lists: &mut Vec<ListBuilder>, current: usize) -> usize {
    let child = lists.len();
    lists.push(ListBuilder::default());
    lists[current].items.push(BuilderItem::List(child));
    child
}

fn materialize_list(index: usize, lists: &[ListBuilder]) -> Vec<Item> {
    lists[index]
        .items
        .iter()
        .map(|item| match item {
            BuilderItem::Value(item) => item.clone(),
            BuilderItem::List(child) => Item::List(materialize_list(*child, lists)),
        })
        .collect()
}

fn parse_item(combination: bool, digit: bool, value: String) -> Item {
    if combination {
        let value = value.replace('-', "");
        let digit_index = value
            .char_indices()
            .find_map(|(index, character)| character.is_ascii_digit().then_some(index))
            .unwrap_or(value.len());
        let qualifier = normalize_combination_qualifier(&value[..digit_index]);
        let number = normalize_number(&value[digit_index..]);
        Item::Combination { qualifier, number }
    } else if digit {
        Item::Number(normalize_number(&value))
    } else {
        Item::Qualifier(normalize_qualifier_alias(&value))
    }
}

fn normalize_number(number: &str) -> String {
    let number = number.trim_start_matches('0');
    if number.is_empty() {
        "0".to_string()
    } else {
        number.to_string()
    }
}

fn normalize_combination_qualifier(qualifier: &str) -> String {
    normalize_qualifier_alias(match qualifier {
        "a" => "alpha",
        "b" => "beta",
        "m" => "milestone",
        other => other,
    })
}

fn normalize_qualifier_alias(qualifier: &str) -> String {
    match qualifier {
        "cr" => "rc",
        "ga" | "final" | "release" => "",
        other => other,
    }
    .to_string()
}

fn normalize_list(items: &mut Vec<Item>) {
    for item in items.iter_mut() {
        if let Item::List(children) = item {
            normalize_list(children);
        }
    }

    let mut index = items.len();
    while index > 0 {
        index -= 1;
        if !item_is_null(&items[index]) {
            continue;
        }
        let remove = if index + 1 == items.len() {
            true
        } else {
            match &items[index + 1] {
                Item::Qualifier(_) => true,
                Item::List(children) => matches!(
                    children.first(),
                    Some(Item::Qualifier(_) | Item::Combination { .. })
                ),
                _ => false,
            }
        };
        if remove {
            items.remove(index);
        }
    }
}

fn item_is_null(item: &Item) -> bool {
    match item {
        Item::Number(number) => number == "0",
        Item::Qualifier(qualifier) => qualifier.is_empty(),
        Item::List(items) => items.is_empty(),
        Item::Combination { .. } => false,
    }
}

fn compare_lists(left: &[Item], right: &[Item]) -> Ordering {
    let length = left.len().max(right.len());
    for index in 0..length {
        let ordering = match (left.get(index), right.get(index)) {
            (Some(left), right) => compare_item(left, right),
            (None, Some(right)) => compare_item(right, None).reverse(),
            (None, None) => Ordering::Equal,
        };
        if ordering != Ordering::Equal {
            return ordering;
        }
    }
    Ordering::Equal
}

fn compare_item(left: &Item, right: Option<&Item>) -> Ordering {
    let Some(right) = right else {
        return match left {
            Item::Number(number) => compare_numbers(number, "0"),
            Item::Qualifier(qualifier) => compare_qualifiers(qualifier, ""),
            Item::Combination { qualifier, .. } => compare_qualifiers(qualifier, ""),
            Item::List(items) => {
                for item in items {
                    let ordering = compare_item(item, None);
                    if ordering != Ordering::Equal {
                        return ordering;
                    }
                }
                Ordering::Equal
            }
        };
    };

    match (left, right) {
        (Item::Number(left), Item::Number(right)) => compare_numbers(left, right),
        (Item::Number(_), _) => Ordering::Greater,
        (Item::Qualifier(_), Item::Number(_)) => Ordering::Less,
        (Item::Qualifier(left), Item::Qualifier(right)) => compare_qualifiers(left, right),
        (
            Item::Qualifier(left),
            Item::Combination {
                qualifier: right, ..
            },
        ) => {
            let ordering = compare_qualifiers(left, right);
            if ordering == Ordering::Equal {
                Ordering::Less
            } else {
                ordering
            }
        }
        (Item::Qualifier(_), Item::List(_)) => Ordering::Less,
        (Item::List(_), Item::Number(_)) => Ordering::Less,
        (Item::List(_), Item::Qualifier(_) | Item::Combination { .. }) => Ordering::Greater,
        (Item::List(left), Item::List(right)) => compare_lists(left, right),
        (Item::Combination { .. }, Item::Number(_)) => Ordering::Less,
        (
            Item::Combination {
                qualifier: left, ..
            },
            Item::Qualifier(right),
        ) => {
            let ordering = compare_qualifiers(left, right);
            if ordering == Ordering::Equal {
                Ordering::Greater
            } else {
                ordering
            }
        }
        (Item::Combination { .. }, Item::List(_)) => Ordering::Less,
        (
            Item::Combination {
                qualifier: left_qualifier,
                number: left_number,
            },
            Item::Combination {
                qualifier: right_qualifier,
                number: right_number,
            },
        ) => compare_qualifiers(left_qualifier, right_qualifier)
            .then_with(|| compare_numbers(left_number, right_number)),
    }
}

fn compare_numbers(left: &str, right: &str) -> Ordering {
    left.len().cmp(&right.len()).then_with(|| left.cmp(right))
}

fn compare_qualifiers(left: &str, right: &str) -> Ordering {
    qualifier_key(left).cmp(&qualifier_key(right))
}

fn qualifier_key(qualifier: &str) -> String {
    let qualifier = match qualifier {
        "ga" | "final" | "release" => "",
        other => other,
    };
    let known = ["alpha", "beta", "milestone", "rc", "snapshot", "", "sp"];
    known
        .iter()
        .position(|known| *known == qualifier)
        .map(|index| index.to_string())
        .unwrap_or_else(|| format!("{}-{qualifier}", known.len()))
}

pub fn parse_constraint(raw: &str) -> Ranges<Version> {
    let raw = raw.trim();
    if raw.is_empty() || raw == "*" {
        return Ranges::full();
    }
    if let Some(exact) = raw.strip_prefix("!=") {
        let exact = exact.trim();
        return exact_range(&MavenVersion::parse(exact), exact).complement();
    }
    if let Some(exact) = raw.strip_prefix('=') {
        let exact = exact.trim();
        return exact_range(&MavenVersion::parse(exact), exact);
    }
    for operator in [">=", "<=", ">", "<"] {
        if let Some(bound) = raw.strip_prefix(operator) {
            let version = MavenVersion::parse(bound.trim());
            return match operator {
                ">=" => lower_bound(&version, true),
                ">" => lower_bound(&version, false),
                "<=" => upper_bound(&version, true),
                "<" => upper_bound(&version, false),
                _ => unreachable!(),
            };
        }
    }
    if !matches!(raw.as_bytes().first(), Some(b'[' | b'(')) {
        // Maven treats a bare version as a recommendation, not a hard
        // restriction. PubGrub has no preference-only range, so accept all.
        return Ranges::full();
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
    } else {
        lower_bound(&MavenVersion::parse(lower), lower_inclusive)
    };
    let upper_range = if upper.is_empty() {
        Ranges::full()
    } else {
        upper_bound(&MavenVersion::parse(upper), upper_inclusive)
    };
    lower_range.intersection(&upper_range)
}

fn exact_range(version: &MavenVersion, raw: &str) -> Ranges<Version> {
    if super::has_explicit_suffix(raw) {
        Ranges::singleton(Version::Maven(version.clone()))
    } else {
        precedence_class(version)
    }
}

fn lower_bound(version: &MavenVersion, inclusive: bool) -> Ranges<Version> {
    match (inclusive, version.before_core(), version.after_core()) {
        (true, Some(before), _) => Ranges::higher_than(Version::Maven(before)),
        (false, _, Some(after)) => Ranges::strictly_higher_than(Version::Maven(after)),
        (true, None, _) => Ranges::higher_than(Version::Maven(version.clone())),
        (false, _, None) => Ranges::strictly_higher_than(Version::Maven(version.clone())),
    }
}

fn upper_bound(version: &MavenVersion, inclusive: bool) -> Ranges<Version> {
    match (inclusive, version.before_core(), version.after_core()) {
        (true, _, Some(after)) => Ranges::lower_than(Version::Maven(after)),
        (false, Some(before), _) => Ranges::strictly_lower_than(Version::Maven(before)),
        (true, _, None) => Ranges::lower_than(Version::Maven(version.clone())),
        (false, None, _) => Ranges::strictly_lower_than(Version::Maven(version.clone())),
    }
}

pub(super) fn precedence_class(version: &MavenVersion) -> Ranges<Version> {
    match (version.before_core(), version.after_core()) {
        (Some(before), Some(after)) => {
            Ranges::between(Version::Maven(before), Version::Maven(after))
        }
        _ => Ranges::singleton(Version::Maven(version.clone())),
    }
}

pub(super) fn strictly_higher_precedence(version: &MavenVersion) -> Ranges<Version> {
    match version.after_core() {
        Some(after) => Ranges::strictly_higher_than(Version::Maven(after)),
        None => Ranges::strictly_higher_than(Version::Maven(version.clone())),
    }
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
    fn follows_comparable_version_hyphen_and_qualifier_ordering() {
        let ordered = [
            "1-alpha2snapshot",
            "1-alpha2",
            "1-beta-2",
            "1-m2",
            "1-rc",
            "1-SNAPSHOT",
            "1",
            "1-sp",
            "1-abc",
            "1-1",
            "1-2",
        ];
        for pair in ordered.windows(2) {
            assert!(
                version(pair[0]) < version(pair[1]),
                "expected {} < {}",
                pair[0],
                pair[1]
            );
        }
        assert_eq!(version("1ga"), version("1"));
        assert_eq!(version("1final"), version("1"));
        assert_eq!(version("1cr"), version("1rc"));
        assert_eq!(version("1a1"), version("1-alpha-1"));
        assert!(version("1.0.RC2") < version("1.0-RC3"));
        assert!(version("1.0-RC3") < version("1.0.1"));
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
    fn parses_unions_exact_ranges_and_recommendations() {
        let range = parse_constraint("(,20],[21,)");
        assert!(range.contains(&version("19")));
        assert!(!range.contains(&version("20.5")));
        assert!(range.contains(&version("21")));

        let exact = parse_constraint("[47.2.0]");
        assert!(exact.contains(&version("47.2")));
        assert!(!exact.contains(&version("47.3")));

        let recommendation = parse_constraint("47.2.0");
        assert!(recommendation.contains(&version("1")));
        assert!(recommendation.contains(&version("999")));

        let orbit_exact = parse_constraint("=47.2.0");
        assert!(orbit_exact.contains(&version("47.2")));
        assert!(!orbit_exact.contains(&version("47.3")));
    }

    #[test]
    fn orbit_exact_operator_distinguishes_explicit_suffixes() {
        let core = parse_constraint("=1.2.3");
        assert!(core.contains(&version("1.2.3-alpha")));
        assert!(core.contains(&version("1.2.3-beta")));
        assert!(core.contains(&version("1.2.3")));
        assert!(!core.contains(&version("1.2.4-alpha")));

        let suffixed = parse_constraint("=1.2.3-alpha");
        assert!(suffixed.contains(&version("1.2.3-alpha")));
        assert!(!suffixed.contains(&version("1.2.3-beta")));
        assert!(!suffixed.contains(&version("1.2.3")));
    }

    #[test]
    fn orbit_ordered_operators_ignore_suffix_precedence() {
        let inclusive = parse_constraint(">=1.2.3-alpha");
        assert!(inclusive.contains(&version("1.2.3-beta")));
        assert!(inclusive.contains(&version("1.2.3")));

        let strict = parse_constraint(">1.2.3-alpha");
        assert!(!strict.contains(&version("1.2.3-beta")));
        assert!(!strict.contains(&version("1.2.3")));
        assert!(strict.contains(&version("1.2.4-alpha")));
    }
}
