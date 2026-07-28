use std::collections::{BTreeMap, BTreeSet};

use clap::ValueEnum;
use comfy_table::{
    Attribute, Cell, Color, ContentArrangement, Table, presets::UTF8_HORIZONTAL_ONLY,
};
use orbit_core::{
    CheckResult, InstanceEntry, ListedPackage, OutdatedMod, PackageChange, PackageChangeKind,
    RemovedPackage, ResolutionReport, SyncReport,
    providers::{ModInfo, SearchResultItem, SideSupport},
    resolver::types::{CandidateDiagnostic, CandidateDiagnosticKind},
};

pub mod view;

mod audit;
mod progress_ndjson;

pub use audit::audit_report;
pub use progress_ndjson::{ndjson_audit_reporter, ndjson_progress_reporter};
pub use view::{
    CacheOutput, CheckOutput, CheckSummary, ConfigEntryOutput, ConfigEntryView, ConfigListOutput,
    ConfigPathOutput, ConfigValueView, DependencyEnvironmentOutput, DiagnosticView, ErrorJson,
    ExportOutput, ImportOutput, InitOutput, InstanceDefaultOutput, InstanceRemoveOutput,
    InstancesOutput, JsonEnvelope, OutdatedOutput, OutdatedSummary, PurgeOutput, RemoveOutput,
    RemovedPackageView, SearchFilters, SearchOutput, SearchResultView,
};
pub use view::{
    check_result_view, diagnostic_view, info_view, instance_view, list_view, outdated_mod_view,
    remote_view, restore_view, search_result_view, sync_view, transaction_view,
};

/// Render format for command results.
///
/// `Text` keeps the adaptive-table/interactive output. `Json` writes a single
/// JSON document (the [`JsonEnvelope`]) to stdout and silences progress unless
/// `--progress-format ndjson` is explicitly requested.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Default)]
pub enum OutputFormat {
    #[default]
    Text,
    Json,
}

/// Progress protocol for long-running operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Default)]
pub enum ProgressFormat {
    #[default]
    None,
    /// One JSON object per line on stderr.
    Ndjson,
}

/// Effective output configuration resolved from CLI flags and config.
///
/// Commands receive this and call [`render`] / [`print_json`] instead of
/// `println!`, so the format switch lives in one place.
#[derive(Debug, Clone, Copy, Default)]
pub struct OutputCfg {
    pub format: OutputFormat,
    pub progress: ProgressFormat,
}

impl OutputCfg {
    /// Whether structured NDJSON progress should be emitted on stderr.
    ///
    /// `--format json` disables progress unless `--progress-format ndjson` is
    /// explicit; `--format text` follows the configured `ui.progress_bar`
    /// style (the caller decides whether to construct a reporter at all).
    pub fn ndjson_progress(self) -> bool {
        self.progress == ProgressFormat::Ndjson
    }
}

/// Render a view-model in the configured format.
///
/// `text` calls the supplied table renderer; `json` prints the envelope to
/// stdout as a single pretty-printed document.
pub fn render<T: serde::Serialize>(
    cfg: OutputCfg,
    command: &'static str,
    view: &T,
    text: impl FnOnce(&T) -> String,
) {
    match cfg.format {
        OutputFormat::Text => print!("{}", text(view)),
        OutputFormat::Json => println!(
            "{}",
            serde_json::to_string_pretty(&JsonEnvelope::new(command, view))
                .expect("command view-models are serializable")
        ),
    }
}

/// Print a JSON envelope directly to stdout.
pub fn print_json<T: serde::Serialize>(command: &'static str, view: &T) {
    println!(
        "{}",
        serde_json::to_string_pretty(&JsonEnvelope::new(command, view))
            .expect("command view-models are serializable")
    );
}

const ABSENT: &str = "—";

const COMPATIBLE_MARK: &str = "\u{2713}";
const INCOMPATIBLE_MARK: &str = "\u{2717}";

pub fn config_entries_table(entries: &[ConfigEntryView]) -> String {
    let mut table = output_table(["Key", "Type", "File/default value"]);
    for entry in entries {
        let value = match &entry.value {
            Some(ConfigValueView::Text(value)) => value.clone(),
            Some(ConfigValueView::Integer(value)) => value.to_string(),
            None => ABSENT.to_string(),
        };
        table.add_row([
            Cell::new(&entry.key),
            Cell::new(&entry.value_type),
            Cell::new(value),
        ]);
    }
    table.to_string()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LogicalChange {
    current: String,
    selected: String,
    candidate: String,
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
                change.selected_description.as_deref().unwrap_or(ABSENT),
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
                    change.candidate.as_str(),
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
                        change.candidate.as_str(),
                        change_label(change.kind),
                    )
                }));
            } else {
                let current = current_version_for(package, &logical);
                rows.push(("◆", package.as_str(), current, current, ABSENT, "keep"));
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
                candidate: change
                    .selected_description
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
                .then_with(|| left.candidate.cmp(&right.candidate))
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
    rows: impl IntoIterator<Item = (&'a str, &'a str, &'a str, &'a str, &'a str, &'a str)>,
    highlight: bool,
) -> String {
    let mut table = error_table(["", "Package", "Current", "Selected", "Candidate", "Action"]);
    for (marker, package, current, selected, candidate, action) in rows {
        let cells = [marker, package, current, selected, candidate, action].map(|value| {
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

/// Format a download count the way search/info tables render it.
fn format_downloads(downloads: u64) -> String {
    if downloads >= 1_000_000 {
        format!("{:.1}M", downloads as f64 / 1_000_000.0)
    } else if downloads >= 1_000 {
        format!("{:.1}K", downloads as f64 / 1_000.0)
    } else {
        downloads.to_string()
    }
}

/// Truncate a description to `max` chars, appending `…` when it was truncated.
fn truncate(text: &str, max: usize) -> String {
    let mut chars = text.chars();
    let mut out: String = chars.by_ref().take(max).collect();
    if chars.next().is_some() {
        out.push('\u{2026}');
    }
    out
}

/// Render search results as an adaptive table.
///
/// `ref_mc` enables a `✓` compatibility column when the caller resolved a
/// reference Minecraft version. Rows are grouped by provider but rendered in a
/// single table so the output stays readable when redirected.
pub fn search_results_table(results: &[(&str, &SearchResultItem)], ref_mc: Option<&str>) -> String {
    let mut table = if ref_mc.is_some() {
        output_table(["", "Package", "Platform", "Downloads", "MC versions"])
    } else {
        output_table(["Package", "Platform", "Downloads", "MC versions"])
    };
    for (provider, item) in results {
        let mc_list = item
            .mc_versions
            .iter()
            .rev()
            .take(3)
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join(", ");
        let name_part = if item.name.to_lowercase() != item.slug.to_lowercase().replace('-', " ") {
            format!("{} — {}", item.slug, item.name)
        } else {
            item.slug.clone()
        };
        let row = if ref_mc.is_some() {
            let compatible = ref_mc
                .map(|rmc| item.mc_versions.iter().any(|v| v == rmc))
                .unwrap_or(false);
            let mark = if compatible { COMPATIBLE_MARK } else { " " };
            vec![
                Cell::new(mark),
                Cell::new(format!("{name_part}\n{}", truncate(&item.description, 80))),
                Cell::new(*provider),
                Cell::new(format_downloads(item.downloads)),
                Cell::new(mc_list),
            ]
        } else {
            vec![
                Cell::new(format!("{name_part}\n{}", truncate(&item.description, 80))),
                Cell::new(*provider),
                Cell::new(format_downloads(item.downloads)),
                Cell::new(mc_list),
            ]
        };
        table.add_row(row);
    }
    table.to_string()
}

/// Render `orbit check` compatibility results as an adaptive table.
pub fn check_results_table(results: &[CheckResult]) -> String {
    let mut table = output_table(["Package", "Current", "Status", "Available", "Provider"]);
    for result in results {
        let (mark, available) = if let Some(version) = &result.available_version {
            (COMPATIBLE_MARK, version.clone())
        } else {
            (INCOMPATIBLE_MARK, ABSENT.to_string())
        };
        table.add_row([
            Cell::new(&result.mod_name),
            Cell::new(&result.current_version),
            Cell::new(mark),
            Cell::new(available),
            Cell::new(if result.available_version.is_some() {
                &result.provider
            } else {
                ABSENT
            }),
        ]);
    }
    table.to_string()
}

/// Render the registered instance list as an adaptive table.
///
/// `current_path` marks the row matching the current working directory with
/// `*`; the default instance is annotated in a dedicated column.
pub fn instances_table(instances: &[InstanceEntry], current_path: Option<&str>) -> String {
    let mut table = output_table(["", "Default", "Name", "Path", "MC", "Loader"]);
    for instance in instances {
        let is_current = current_path.is_some_and(|current| {
            std::path::Path::new(&instance.path)
                .canonicalize()
                .ok()
                .as_deref()
                == Some(std::path::Path::new(current))
        });
        let marker = if is_current { "*" } else { " " };
        let default_marker = if instance.is_default { "(default)" } else { "" };
        table.add_row([
            Cell::new(marker),
            Cell::new(default_marker),
            Cell::new(&instance.name),
            Cell::new(&instance.path),
            Cell::new(&instance.mc_version),
            Cell::new(&instance.modloader),
        ]);
    }
    table.to_string()
}

/// Render the remotes of a single package as an adaptive table.
///
/// `header` overrides the default summary line so callers can distinguish a
/// dry-run preview from the live state.
pub fn remote_list_table(report: &orbit_core::RemoteReport, header: Option<&str>) -> String {
    let mut table = output_table(["#", "Provider", "Locator"]);
    for (index, remote) in report.remotes.iter().enumerate() {
        table.add_row([
            Cell::new(index + 1),
            Cell::new(remote.provider()),
            Cell::new(remote.display_locator()),
        ]);
    }
    let default_header = format!(
        "Package has {} remote(s) for {}:",
        report.remotes.len(),
        report.package
    );
    let header = header.unwrap_or(&default_header);
    format!("{header}\n{table}")
}

/// Render the flat `orbit list` output as an adaptive table.
pub fn installed_packages_table(packages: &[ListedPackage]) -> String {
    let mut table = output_table(["Package", "Version", "Remotes", "Env", "Notes"]);
    for package in packages {
        let mut notes = Vec::new();
        if package.optional {
            notes.push("optional".to_string());
        }
        for (name, ver) in &package.bundled {
            notes.push(format!("+ bundled: {name} v{ver}"));
        }
        table.add_row([
            Cell::new(&package.mod_id),
            Cell::new(&package.version),
            Cell::new(package.remotes.join(", ")),
            Cell::new(&package.environment),
            Cell::new(notes.join("\n")),
        ]);
    }
    table.to_string()
}

/// Render `orbit sync` platform and package deltas as an adaptive table.
pub fn sync_report_table(report: &SyncReport) -> String {
    let mut table = output_table(["", "Field", "Previous", "Current"]);
    for change in &report.platform_changes {
        table.add_row([
            Cell::new("~").fg(Color::Cyan),
            Cell::new(format!("platform:{}", change.field)),
            Cell::new(&change.previous),
            Cell::new(&change.current),
        ]);
    }
    for package in &report.added {
        table.add_row([
            Cell::new("+").fg(Color::Green),
            Cell::new("added"),
            Cell::new(ABSENT),
            Cell::new(package),
        ]);
    }
    for package in &report.changed {
        table.add_row([
            Cell::new("~").fg(Color::Yellow),
            Cell::new("changed"),
            Cell::new(ABSENT),
            Cell::new(package),
        ]);
    }
    for package in &report.missing {
        table.add_row([
            Cell::new("-").fg(Color::Red),
            Cell::new("missing"),
            Cell::new(package),
            Cell::new(ABSENT),
        ]);
    }
    for package in &report.unlocked {
        table.add_row([
            Cell::new("?").fg(Color::Magenta),
            Cell::new("unlocked"),
            Cell::new(package),
            Cell::new(ABSENT),
        ]);
    }
    if table.row_count() == 0 {
        return "No local changes.".to_string();
    }
    table.to_string()
}

/// Render `orbit info` details as an adaptive table.
pub fn mod_info_table(provider: &str, info: &ModInfo) -> String {
    let mut table = output_table(["Field", "Value"]);
    table.add_row([
        Cell::new("name"),
        Cell::new(format!("{} ({provider})", info.name)),
    ]);
    table.add_row([Cell::new("id"), Cell::new(&info.project_id)]);
    table.add_row([Cell::new("slug"), Cell::new(&info.slug)]);
    table.add_row([Cell::new("description"), Cell::new(&info.description)]);
    if !info.authors.is_empty() {
        table.add_row([Cell::new("authors"), Cell::new(info.authors.join(", "))]);
    }
    table.add_row([Cell::new("latest version"), Cell::new(&info.latest_version)]);
    table.add_row([
        Cell::new("client side"),
        Cell::new(side_label(info.client_side.as_ref())),
    ]);
    table.add_row([
        Cell::new("server side"),
        Cell::new(side_label(info.server_side.as_ref())),
    ]);
    table.add_row([
        Cell::new("license"),
        Cell::new(info.license.as_deref().unwrap_or("unknown")),
    ]);
    table.add_row([Cell::new("downloads"), Cell::new(info.downloads)]);
    if !info.categories.is_empty() {
        table.add_row([
            Cell::new("categories"),
            Cell::new(info.categories.join(", ")),
        ]);
    }
    if !info.recent_versions.is_empty() {
        let mut versions = Table::new();
        versions
            .load_preset(UTF8_HORIZONTAL_ONLY)
            .set_content_arrangement(ContentArrangement::Dynamic)
            .set_header(["Version", "MC", "Loader", "Released"]);
        for version in &info.recent_versions {
            versions.add_row([
                Cell::new(&version.version),
                Cell::new(version.mc_versions.join(", ")),
                Cell::new(&version.loader),
                Cell::new(&version.released_at),
            ]);
        }
        table.add_row([
            Cell::new("recent versions"),
            Cell::new(versions.to_string()),
        ]);
    }
    let deps = if info.dependencies.is_empty() {
        "(none)".to_string()
    } else {
        info.dependencies
            .iter()
            .map(|dependency| {
                let name = dependency
                    .slug
                    .as_deref()
                    .or(dependency.project_id.as_deref())
                    .unwrap_or("unknown");
                let kind = if dependency.required {
                    "required"
                } else {
                    "optional"
                };
                format!("{name} ({kind})")
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    table.add_row([Cell::new("dependencies"), Cell::new(deps)]);
    table.to_string()
}

fn side_label(side: Option<&SideSupport>) -> &'static str {
    match side {
        Some(SideSupport::Required) => "required",
        Some(SideSupport::Optional) => "optional",
        Some(SideSupport::Unsupported) => "unsupported",
        None => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use orbit_core::{PackageRemote, PlatformChange};

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
            selected_description: None,
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
    fn choices_distinguish_same_version_candidates_without_rendering_hashes() {
        let mut first_change = change(
            "voxy",
            Some("1"),
            Some("2"),
            PackageChangeKind::Upgrade,
            "voxy-a.jar",
        );
        first_change.selected_description =
            Some("Modrinth project abc, release one; requires sodium =0.8.9".to_string());
        let mut second_change = first_change.clone();
        second_change.selected_description =
            Some("CurseForge project 123, file 456; requires sodium >=0.9".to_string());
        let first = ResolutionReport {
            selected_candidates: BTreeMap::from([(
                "voxy".to_string(),
                "sha512:must-not-be-rendered".to_string(),
            )]),
            changes: vec![first_change],
            ..ResolutionReport::default()
        };
        let second = ResolutionReport {
            selected_candidates: BTreeMap::from([(
                "voxy".to_string(),
                "sha512:also-secret".to_string(),
            )]),
            changes: vec![second_change],
            ..ResolutionReport::default()
        };

        let output = resolution_choices(&[first, second]);

        assert_eq!(output.matches("Option ").count(), 2);
        assert!(output.contains("Modrinth project abc"));
        assert!(output.contains("CurseForge project 123"));
        assert!(!output.contains("sha512"));
        assert!(!output.contains(".jar"));
    }

    #[test]
    fn blocked_candidates_are_never_described_as_up_to_date() {
        let message = no_upgrade_message(Some("voxy"), true);

        assert!(message.contains("No feasible upgrade"));
        assert!(!message.contains("up to date"));
    }

    fn search_item(
        slug: &str,
        name: &str,
        downloads: u64,
        mc_versions: &[&str],
    ) -> SearchResultItem {
        SearchResultItem {
            project_id: format!("id-{slug}"),
            slug: slug.to_string(),
            name: name.to_string(),
            description: format!("description for {slug}"),
            latest_version: "1.0".to_string(),
            downloads,
            mc_versions: mc_versions.iter().map(|s| s.to_string()).collect(),
            client_side: None,
            server_side: None,
            categories: Vec::new(),
            icon_url: None,
            accent_color: None,
        }
    }

    #[test]
    fn search_table_marks_compatible_results_with_a_check() {
        let item = search_item("sodium", "Sodium", 1_500_000, &["1.20", "1.21"]);
        let output = search_results_table(&[("modrinth", &item)], Some("1.21"));

        assert!(output.contains('\u{2713}'));
        assert!(output.contains("sodium"));
        assert!(output.contains("1.5M"));
        assert!(output.contains("modrinth"));
    }

    #[test]
    fn search_table_omits_compatibility_column_without_reference_mc() {
        let item = search_item("sodium", "Sodium", 999, &["1.20"]);
        let output = search_results_table(&[("modrinth", &item)], None);

        assert!(!output.contains('\u{2713}'));
        assert!(output.contains("sodium"));
    }

    #[test]
    fn check_table_distinguishes_compatible_from_blocking_results() {
        let results = vec![
            CheckResult {
                mod_name: "sodium".to_string(),
                current_version: "0.8".to_string(),
                provider: "modrinth".to_string(),
                compatible: true,
                available_version: Some("0.9".to_string()),
            },
            CheckResult {
                mod_name: "voxy".to_string(),
                current_version: "1.0".to_string(),
                provider: "modrinth".to_string(),
                compatible: false,
                available_version: None,
            },
        ];
        let output = check_results_table(&results);

        assert!(output.contains('\u{2713}'));
        assert!(output.contains('\u{2717}'));
        assert!(output.contains("0.9"));
        assert!(output.contains("voxy"));
        // The blocking row shows the absent marker, not a "no compatible" string.
        assert!(output.contains(ABSENT));
    }

    #[test]
    fn instances_table_marks_current_and_default_entries() {
        let instances = vec![
            InstanceEntry {
                name: "alpha".to_string(),
                path: "/tmp/alpha".to_string(),
                mc_version: "1.21".to_string(),
                modloader: "fabric".to_string(),
                is_default: true,
            },
            InstanceEntry {
                name: "beta".to_string(),
                path: "/tmp/beta".to_string(),
                mc_version: "1.20".to_string(),
                modloader: "forge".to_string(),
                is_default: false,
            },
        ];
        let output = instances_table(&instances, None);

        assert!(output.contains("(default)"));
        assert!(output.contains("alpha"));
        assert!(output.contains("beta"));
    }

    #[test]
    fn remote_table_does_not_render_internal_hashes() {
        let report = orbit_core::RemoteReport {
            package: "sodium".to_string(),
            remotes: vec![
                PackageRemote::Modrinth {
                    project_id: "abc".to_string(),
                },
                PackageRemote::File {
                    path: ".orbit/sources/sha512-deadbeef.jar".to_string(),
                },
            ],
            changed: true,
        };
        let output = remote_list_table(&report, None);

        assert!(output.contains("modrinth:abc"));
        assert!(output.contains("file:managed local source"));
        assert!(!output.contains("sha512"));
        assert!(!output.contains(".jar"));
    }

    #[test]
    fn installed_table_lists_bundled_mods_without_jar_filenames() {
        let package = ListedPackage {
            mod_id: "fabric-api".to_string(),
            version: "0.100".to_string(),
            remotes: vec!["modrinth:project".to_string()],
            environment: "both".to_string(),
            optional: false,
            dependencies: Vec::new(),
            bundled: vec![("fabric-api-base".to_string(), "0.100".to_string())],
        };
        let output = installed_packages_table(&[package]);

        assert!(output.contains("fabric-api"));
        assert!(output.contains("bundled: fabric-api-base"));
        assert!(!output.contains(".jar"));
    }

    #[test]
    fn sync_table_reports_platform_and_package_changes() {
        let report = SyncReport {
            platform_changes: vec![PlatformChange {
                field: "mc_version",
                previous: "1.20".to_string(),
                current: "1.21".to_string(),
            }],
            added: vec!["sodium".to_string()],
            changed: vec!["lithium".to_string()],
            missing: vec!["fabric-api".to_string()],
            unlocked: vec!["voxy".to_string()],
            removed: Vec::new(),
            diagnostics: Vec::new(),
            warnings: Vec::new(),
        };
        let output = sync_report_table(&report);

        assert!(output.contains("platform:mc_version"));
        assert!(output.contains("added"));
        assert!(output.contains("missing"));
        assert!(output.contains("unlocked"));
    }

    #[test]
    fn empty_sync_report_is_summarised_without_a_table() {
        let report = SyncReport::default();
        let output = sync_report_table(&report);

        assert!(output.contains("No local changes"));
    }

    #[test]
    fn mod_info_table_renders_required_side_and_dependencies() {
        let info = ModInfo {
            project_id: "abc".to_string(),
            slug: "sodium".to_string(),
            name: "Sodium".to_string(),
            description: "rendering engine".to_string(),
            authors: vec!["author".to_string()],
            latest_version: "0.9".to_string(),
            downloads: 1_000,
            license: Some("MIT".to_string()),
            client_side: Some(SideSupport::Required),
            server_side: Some(SideSupport::Unsupported),
            categories: vec!["optimization".to_string()],
            icon_url: None,
            accent_color: None,
            website_url: None,
            source_url: None,
            issues_url: None,
            wiki_url: None,
            gallery: Vec::new(),
            recent_versions: Vec::new(),
            dependencies: Vec::new(),
        };
        let output = mod_info_table("modrinth", &info);

        assert!(output.contains("Sodium (modrinth)"));
        assert!(output.contains("required"));
        assert!(output.contains("unsupported"));
    }
}
