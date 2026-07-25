use std::collections::{BTreeMap, BTreeSet};

use comfy_table::{
    Attribute, Cell, Color, ContentArrangement, Table, presets::UTF8_HORIZONTAL_ONLY,
};
use orbit_core::{
    OutdatedMod, PackageChange, PackageChangeKind, RemovedPackage, ResolutionReport,
    resolver::types::{CandidateDiagnostic, CandidateDiagnosticKind},
};

mod audit;

pub use audit::audit_report;

const ABSENT: &str = "—";

#[derive(Debug, Clone, PartialEq, Eq)]
struct LogicalChange {
    current: String,
    selected: String,
    kind: PackageChangeKind,
}

pub fn outdated_table(updates: &[OutdatedMod]) -> String {
    let mut table = output_table(["Package", "Current", "Available"]);
    for update in updates {
        table.add_row([
            Cell::new(&update.mod_id),
            Cell::new(&update.current_version),
            Cell::new(&update.new_version),
        ]);
    }
    table.to_string()
}

pub fn diagnostics_table(diagnostics: &[CandidateDiagnostic]) -> String {
    let mut table = error_table([
        "Package",
        "Selected",
        "Candidate checked",
        "Why not upgraded",
    ]);
    for diagnostic in diagnostics {
        let summary = match diagnostic.kind {
            CandidateDiagnosticKind::NoCompatibleCandidate => {
                "no compatible remote candidate was discovered"
            }
            CandidateDiagnosticKind::ExcludedByPropagation => "excluded by dependency propagation",
            CandidateDiagnosticKind::Backtracked => "backtracked after a dependency conflict",
            CandidateDiagnosticKind::Unexplained => "the solver recorded no excluding derivation",
        };
        let mut reason = summary.to_string();
        for fact in &diagnostic.facts {
            reason.push_str("\n• ");
            reason.push_str(fact);
        }
        table.add_row([
            Cell::new(&diagnostic.package),
            Cell::new(&diagnostic.selected_version),
            Cell::new(&diagnostic.candidate_version),
            Cell::new(reason),
        ]);
    }
    table.to_string()
}

pub fn package_changes_table(changes: &[PackageChange]) -> String {
    changes_table(
        changes.iter().map(|change| {
            (
                "",
                change.package.as_str(),
                change.current_version.as_deref().unwrap_or(ABSENT),
                change.selected_version.as_deref().unwrap_or(ABSENT),
                change_label(change.kind),
            )
        }),
        false,
    )
}

pub fn removed_packages_table(removals: &[RemovedPackage]) -> String {
    let mut table = output_table(["Package", "Version", "Action"]);
    for package in removals {
        table.add_row([
            Cell::new(&package.mod_id),
            Cell::new(&package.version),
            Cell::new("remove"),
        ]);
    }
    table.to_string()
}

pub fn no_upgrade_message(package: Option<&str>, has_diagnostics: bool) -> String {
    match (package, has_diagnostics) {
        (Some(package), true) => format!("No feasible upgrade is available for {package}."),
        (Some(package), false) => format!("{package} is up to date."),
        (None, true) => "No feasible package upgrades are available.".to_string(),
        (None, false) => "All packages are up to date.".to_string(),
    }
}

pub fn resolution_choices(alternatives: &[ResolutionReport]) -> String {
    let logical: Vec<_> = alternatives.iter().map(logical_changes).collect();
    let packages: BTreeSet<_> = logical
        .iter()
        .flat_map(|changes| changes.keys().cloned())
        .collect();
    let common: BTreeSet<_> = packages
        .iter()
        .filter(|package| {
            logical
                .windows(2)
                .all(|pair| pair[0].get(*package) == pair[1].get(*package))
        })
        .cloned()
        .collect();
    let differing: Vec<_> = packages.difference(&common).cloned().collect();

    let mut output = String::new();
    let common_rows: Vec<_> = common
        .iter()
        .flat_map(|package| {
            logical[0][package].iter().map(|change| {
                (
                    "",
                    package.as_str(),
                    change.current.as_str(),
                    change.selected.as_str(),
                    change_label(change.kind),
                )
            })
        })
        .collect();
    if !common_rows.is_empty() {
        output.push_str("Common actions:\n");
        output.push_str(&changes_table(common_rows, false));
        output.push('\n');
    }

    for (index, alternative) in alternatives.iter().enumerate() {
        if index > 0 || !output.is_empty() {
            output.push('\n');
        }
        output.push_str(&format!("Option {} — differing actions:\n", index + 1));
        let mut rows = Vec::new();
        for package in &differing {
            if let Some(changes) = logical[index].get(package) {
                rows.extend(changes.iter().map(|change| {
                    (
                        "◆",
                        package.as_str(),
                        change.current.as_str(),
                        change.selected.as_str(),
                        change_label(change.kind),
                    )
                }));
            } else {
                let current = current_version_for(package, &logical);
                rows.push(("◆", package.as_str(), current, current, "keep"));
            }
        }
        if rows.is_empty() {
            output.push_str("  No logical package action differs.\n");
        } else {
            output.push_str(&changes_table(rows, true));
            output.push('\n');
        }
        if !alternative.warnings.is_empty() {
            output.push_str(&format!(
                "  {} dependency ordering warning(s)\n",
                alternative.warnings.len()
            ));
        }
    }
    output.trim_end().to_string()
}

pub fn change_label(kind: PackageChangeKind) -> &'static str {
    match kind {
        PackageChangeKind::Install => "install",
        PackageChangeKind::Upgrade => "upgrade",
        PackageChangeKind::Downgrade => "downgrade",
        PackageChangeKind::Replace => "replace",
        PackageChangeKind::Remove => "remove",
    }
}

fn logical_changes(report: &ResolutionReport) -> BTreeMap<String, Vec<LogicalChange>> {
    let mut changes = BTreeMap::<String, Vec<LogicalChange>>::new();
    for change in &report.changes {
        changes
            .entry(change.package.clone())
            .or_default()
            .push(LogicalChange {
                current: change
                    .current_version
                    .clone()
                    .unwrap_or_else(|| ABSENT.into()),
                selected: change
                    .selected_version
                    .clone()
                    .unwrap_or_else(|| ABSENT.into()),
                kind: change.kind,
            });
    }
    for changes in changes.values_mut() {
        changes.sort_by(|left, right| {
            left.current
                .cmp(&right.current)
                .then_with(|| left.selected.cmp(&right.selected))
                .then_with(|| change_label(left.kind).cmp(change_label(right.kind)))
        });
    }
    changes
}

fn current_version_for<'a>(
    package: &str,
    alternatives: &'a [BTreeMap<String, Vec<LogicalChange>>],
) -> &'a str {
    alternatives
        .iter()
        .filter_map(|changes| changes.get(package))
        .flatten()
        .map(|change| change.current.as_str())
        .find(|current| *current != ABSENT)
        .unwrap_or(ABSENT)
}

fn changes_table<'a>(
    rows: impl IntoIterator<Item = (&'a str, &'a str, &'a str, &'a str, &'a str)>,
    highlight: bool,
) -> String {
    let mut table = error_table(["", "Package", "Current", "Selected", "Action"]);
    for (marker, package, current, selected, action) in rows {
        let cells = [marker, package, current, selected, action].map(|value| {
            let cell = Cell::new(value);
            if highlight {
                cell.fg(Color::Yellow).add_attribute(Attribute::Bold)
            } else {
                cell
            }
        });
        table.add_row(cells);
    }
    table.to_string()
}

fn output_table<const N: usize>(headers: [&str; N]) -> Table {
    configured_table(headers, false)
}

fn error_table<const N: usize>(headers: [&str; N]) -> Table {
    configured_table(headers, true)
}

fn configured_table<const N: usize>(headers: [&str; N], stderr: bool) -> Table {
    let mut table = Table::new();
    table
        .load_preset(UTF8_HORIZONTAL_ONLY)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_header(headers);
    if stderr {
        table.use_stderr();
    }
    if table.width().is_none() {
        table.set_width(120);
    }
    table
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(
        package: &str,
        current: Option<&str>,
        selected: Option<&str>,
        kind: PackageChangeKind,
        filename: &str,
    ) -> PackageChange {
        PackageChange {
            package: package.to_string(),
            current_version: current.map(str::to_string),
            selected_version: selected.map(str::to_string),
            filename: Some(filename.to_string()),
            selected_filename: Some(format!("selected-{filename}")),
            kind,
        }
    }

    #[test]
    fn package_tables_never_render_physical_jar_names() {
        let output = package_changes_table(&[change(
            "sodium",
            Some("1"),
            Some("2"),
            PackageChangeKind::Upgrade,
            "sodium-fabric-mc26.1.2-very-long-name.jar",
        )]);

        assert!(output.contains("sodium"));
        assert!(output.contains("upgrade"));
        assert!(!output.contains(".jar"));
    }

    #[test]
    fn diagnostics_are_structured_as_a_wrapping_table() {
        let output = diagnostics_table(&[CandidateDiagnostic {
            package: "voxy".to_string(),
            selected_version: "1".to_string(),
            candidate_version: "2".to_string(),
            kind: CandidateDiagnosticKind::ExcludedByPropagation,
            facts: vec!["voxy 2 requires sodium =0.8.9".to_string()],
        }]);

        assert!(output.contains("Why not upgraded"));
        assert!(output.contains("excluded by dependency propagation"));
        assert!(output.contains("requires sodium"));
    }

    #[test]
    fn choices_show_common_actions_once_and_mark_every_difference() {
        let common = change(
            "fabric-api",
            Some("1"),
            Some("2"),
            PackageChangeKind::Upgrade,
            "fabric-api.jar",
        );
        let first = ResolutionReport {
            changes: vec![
                common.clone(),
                change(
                    "sodium",
                    Some("1"),
                    Some("2"),
                    PackageChangeKind::Upgrade,
                    "sodium.jar",
                ),
            ],
            ..ResolutionReport::default()
        };
        let second = ResolutionReport {
            changes: vec![
                common,
                change(
                    "lithium",
                    Some("1"),
                    Some("2"),
                    PackageChangeKind::Upgrade,
                    "lithium.jar",
                ),
            ],
            ..ResolutionReport::default()
        };

        let output = resolution_choices(&[first, second]);

        assert_eq!(output.matches("fabric-api").count(), 1);
        assert_eq!(output.matches("Option ").count(), 2);
        assert!(output.matches('◆').count() >= 4, "{output}");
        assert!(output.contains("keep"));
        assert!(!output.contains(".jar"));
    }

    #[test]
    fn blocked_candidates_are_never_described_as_up_to_date() {
        let message = no_upgrade_message(Some("voxy"), true);

        assert!(message.contains("No feasible upgrade"));
        assert!(!message.contains("up to date"));
    }
}
