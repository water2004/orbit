//! View-models for command output.
//!
//! Each command builds a view-model from the core domain report. View-models
//! are the single source of truth for both `text` (tables) and `json` output.
//! They never contain content hashes, physical JAR filenames, or provider
//! secrets (CLAUDE.md #41/#50): those stay in core domain types and never
//! cross the output boundary.
//!
//! The JSON envelope wraps every command result with a stable
//! `schema_version` + `command` + `ok` header so automation can branch on
//! command name without parsing the body.

use serde::Serialize;

pub use orbit_machine_protocol::{ErrorEnvelope as ErrorJson, SuccessEnvelope as JsonEnvelope};

// ---------------------------------------------------------------------------
// search
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct SearchOutput {
    pub query: String,
    pub platforms: Vec<String>,
    pub filters: SearchFilters,
    pub ref_mc_version: Option<String>,
    pub results: Vec<SearchResultView>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchFilters {
    pub mc_version: Option<String>,
    pub modloader: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchResultView {
    pub slug: String,
    pub name: String,
    pub project_id: String,
    pub platform: String,
    pub description: String,
    pub latest_version: String,
    pub downloads: u64,
    pub mc_versions: Vec<String>,
    pub client_side: String,
    pub server_side: String,
    pub categories: Vec<String>,
    pub icon_url: Option<String>,
    pub accent_color: Option<u32>,
    /// `None` when no reference MC version was supplied; `Some(bool)` otherwise.
    pub compatible: Option<bool>,
}

// ---------------------------------------------------------------------------
// info
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct InfoOutput {
    pub provider: String,
    pub project_id: String,
    pub slug: String,
    pub name: String,
    pub description: String,
    pub authors: Vec<String>,
    pub latest_version: String,
    pub downloads: u64,
    pub license: Option<String>,
    pub client_side: String,
    pub server_side: String,
    pub categories: Vec<String>,
    pub icon_url: Option<String>,
    pub accent_color: Option<u32>,
    pub website_url: Option<String>,
    pub source_url: Option<String>,
    pub issues_url: Option<String>,
    pub wiki_url: Option<String>,
    pub gallery: Vec<ProjectImageView>,
    pub recent_versions: Vec<ModVersionView>,
    pub dependencies: Vec<DependencyView>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProjectImageView {
    pub url: String,
    pub thumbnail_url: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModVersionView {
    pub version: String,
    pub mc_versions: Vec<String>,
    pub loader: String,
    pub released_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DependencyView {
    pub slug: Option<String>,
    pub project_id: Option<String>,
    pub required: bool,
}

// ---------------------------------------------------------------------------
// dependency environment
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct PackageEnvironmentOutput {
    pub package: String,
    /// `None` is the persisted `auto` state.
    pub configured: Option<String>,
    /// Missing only when auto has no selected lock entry yet.
    pub effective: Option<String>,
    pub dry_run: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct PackageActivationOutput {
    pub package: String,
    pub previous_enabled: bool,
    pub enabled: bool,
    pub changed: bool,
    pub dry_run: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct PackageConstraintOutput {
    pub package: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous: Option<String>,
    pub current: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_string: Option<String>,
    pub string: String,
    pub policy: PackageVersionPolicyOutput,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_selected_version: Option<String>,
    pub selected_version: Option<String>,
    pub selected_satisfies: Option<bool>,
    pub changed: bool,
    pub applied: bool,
    pub dry_run: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transaction: Option<TransactionOutput>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PackageVersionPolicyOutput {
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operator: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lower: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upper: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_lower: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_upper: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requirement: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PackageVersionsOutput {
    pub package: String,
    pub constraint: String,
    pub string: String,
    pub policy: PackageVersionPolicyOutput,
    pub selected_version: Option<String>,
    pub candidates: Vec<PackageVersionCandidateView>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PackageVersionCandidateView {
    pub version: String,
    pub numeric_core: Option<String>,
    pub string_tokens: Vec<String>,
    pub numeric_filterable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub numeric_error: Option<String>,
    pub sources: Vec<String>,
    pub details: String,
    pub selected: bool,
    pub matches_constraint: bool,
}

// ---------------------------------------------------------------------------
// list
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct ListOutput {
    pub target: Option<String>,
    pub tree: bool,
    pub packages: Vec<ListedPackageView>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ListedPackageView {
    pub mod_id: String,
    pub version: String,
    pub version_constraint: String,
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_path: Option<String>,
    pub remotes: Vec<String>,
    pub configured_environment: Option<String>,
    pub environment: String,
    pub optional: bool,
    pub dependencies: Vec<String>,
    pub bundled_count: usize,
}

// ---------------------------------------------------------------------------
// migrate
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct MigrationOutput {
    pub subcommand: String,
    pub dry_run: bool,
    pub target_directory: String,
    pub source_mc_version: String,
    pub target_mc_version: String,
    pub target_loader: String,
    pub target_loader_version: String,
    pub summary: MigrationSummary,
    pub changes: Vec<PackageChangeView>,
    pub diagnostics: Vec<DiagnosticView>,
    pub warnings: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub export: Option<MigrationExportView>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MigrationSummary {
    pub selected_packages: usize,
    pub installs: usize,
    pub upgrades: usize,
    pub downgrades: usize,
    pub replacements: usize,
    pub removals: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct MigrationExportView {
    pub applied: bool,
    pub config_files: usize,
    pub config_bytes: u64,
}

// ---------------------------------------------------------------------------
// outdated
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct OutdatedOutput {
    pub package: Option<String>,
    pub summary: OutdatedSummary,
    pub updates: Vec<OutdatedModView>,
    pub diagnostics: Vec<DiagnosticView>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OutdatedSummary {
    pub upgrades: usize,
    pub up_to_date: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct OutdatedModView {
    pub mod_id: String,
    pub current_version: String,
    pub new_version: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiagnosticView {
    pub package: String,
    pub selected_version: String,
    pub candidate_version: String,
    pub kind: String,
    pub facts: Vec<String>,
}

// ---------------------------------------------------------------------------
// instances
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct InstancesOutput {
    pub subcommand: String,
    pub instances: Vec<InstanceView>,
}

#[derive(Debug, Clone, Serialize)]
pub struct InstanceView {
    pub name: String,
    pub path: String,
    pub mc_version: String,
    pub modloader: String,
    pub is_default: bool,
    pub is_current: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct InstanceDefaultOutput {
    pub subcommand: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct InstanceRegisterOutput {
    pub subcommand: String,
    pub instance: InstanceView,
}

#[derive(Debug, Clone, Serialize)]
pub struct InstanceRemoveOutput {
    pub subcommand: String,
    pub name: String,
}

// ---------------------------------------------------------------------------
// remote
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct RemoteOutput {
    pub subcommand: String,
    pub package: String,
    pub changed: bool,
    pub remotes: Vec<RemoteEntryView>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RemoteEntryView {
    /// One-based index shown to users.
    pub index: usize,
    pub provider: String,
    pub locator: String,
}

// ---------------------------------------------------------------------------
// sync
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct SyncOutput {
    pub dry_run: bool,
    pub summary: SyncSummary,
    pub platform_changes: Vec<PlatformChangeView>,
    pub added: Vec<String>,
    pub changed: Vec<String>,
    /// Packages removed from TOML, lock, and groups because no local JAR exists.
    pub removed: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SyncSummary {
    pub platform_changes: usize,
    pub added: usize,
    pub changed: usize,
    pub removed: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlatformChangeView {
    pub field: String,
    pub previous: String,
    pub current: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RemovedPackageView {
    pub mod_id: String,
    pub version: String,
}

// ---------------------------------------------------------------------------
// install / add / upgrade (transaction report)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct InstanceInstallOutput {
    pub dry_run: bool,
    pub summary: InstanceInstallSummary,
    pub installed: Vec<String>,
    pub already_present: Vec<String>,
    pub skipped: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct InstanceInstallSummary {
    pub installed: usize,
    pub already_present: usize,
    pub skipped: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct TransactionOutput {
    pub dry_run: bool,
    pub summary: TransactionSummary,
    pub changes: Vec<PackageChangeView>,
    pub installed: Vec<InstalledView>,
    pub removed: Vec<RemovedPackageView>,
    pub already_satisfied: Vec<String>,
    pub diagnostics: Vec<DiagnosticView>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TransactionSummary {
    pub installed: usize,
    pub removed: usize,
    pub already_satisfied: usize,
    pub skipped_optional: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct PackageChangeView {
    pub package: String,
    pub kind: String,
    pub current_version: Option<String>,
    pub selected_version: Option<String>,
    pub selected_description: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct InstalledView {
    pub mod_id: String,
    pub version: String,
}

// ---------------------------------------------------------------------------
// remove / purge
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct RemoveOutput {
    pub mod_id: String,
    pub jar_deleted: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct PurgeOutput {
    pub mod_id: String,
    pub jar_deleted: bool,
    pub configs_removed: Vec<String>,
}

// ---------------------------------------------------------------------------
// import / export
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct ImportOutput {
    pub dry_run: bool,
    pub added: Vec<String>,
    pub merged: Vec<String>,
    pub replaced: Vec<String>,
    pub kept: Vec<String>,
    pub extracted: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExportOutput {
    pub dry_run: bool,
    pub path: String,
    pub packages: usize,
    pub bytes: u64,
}

// ---------------------------------------------------------------------------
// init
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct InitOutput {
    pub dry_run: bool,
    pub name: String,
    pub mc_version: String,
    pub modloader: String,
    pub modloader_version: String,
    pub locked_packages: usize,
    pub scanned_mods: usize,
    pub identified: usize,
    pub unknown: usize,
    pub lock_created: bool,
    pub dependency_error: Option<String>,
}

// ---------------------------------------------------------------------------
// cache
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct CacheOutput {
    pub subcommand: String,
    pub dry_run: bool,
    pub cache_path: String,
    pub files_before: usize,
    pub bytes_before: u64,
    pub files_removed: usize,
    pub bytes_freed: u64,
}

// ---------------------------------------------------------------------------
// config
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct ConfigPathOutput {
    pub subcommand: String,
    pub config_path: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConfigListOutput {
    pub subcommand: String,
    pub config_path: String,
    pub entries: Vec<ConfigEntryView>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConfigEntryOutput {
    pub subcommand: String,
    pub config_path: String,
    pub dry_run: bool,
    pub entry: ConfigEntryView,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConfigEntryView {
    pub key: String,
    pub value_type: String,
    pub sensitive: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<ConfigValueView>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum ConfigValueView {
    Text(String),
    Integer(u64),
}

// ---------------------------------------------------------------------------
// Conversions from core domain types to view-models.
//
// These keep the (hash/filename/secret)-free boundary explicit: a field that
// must not be exposed is simply not copied. Adding a new core field does not
// leak into JSON unless a converter is updated.
// ---------------------------------------------------------------------------

use orbit_core::providers::{ModInfo, SearchResultItem, SideSupport};
use orbit_core::resolver::types::{CandidateDiagnostic, CandidateDiagnosticKind};
use orbit_core::{
    InstanceEntry, ListedPackage, OutdatedMod, PackageChange, PackageChangeKind, RemoteReport,
    RemovedPackage, SyncReport,
};

/// Stable string codes for `CandidateDiagnosticKind` / `PackageChangeKind`.
pub fn diagnostic_kind_code(kind: &CandidateDiagnosticKind) -> &'static str {
    match kind {
        CandidateDiagnosticKind::NoCompatibleCandidate => "no_compatible_candidate",
        CandidateDiagnosticKind::ExcludedByPropagation => "excluded_by_propagation",
        CandidateDiagnosticKind::Backtracked => "backtracked",
        CandidateDiagnosticKind::Unexplained => "unexplained",
    }
}

pub fn change_kind_code(kind: PackageChangeKind) -> &'static str {
    match kind {
        PackageChangeKind::Install => "install",
        PackageChangeKind::Upgrade => "upgrade",
        PackageChangeKind::Downgrade => "downgrade",
        PackageChangeKind::Replace => "replace",
        PackageChangeKind::Remove => "remove",
    }
}

fn side_label(side: Option<&SideSupport>) -> String {
    match side {
        Some(SideSupport::Required) => "required",
        Some(SideSupport::Optional) => "optional",
        Some(SideSupport::Unsupported) => "unsupported",
        None => "unknown",
    }
    .to_string()
}

pub fn search_result_view(
    platform: &str,
    item: &SearchResultItem,
    ref_mc: Option<&str>,
) -> SearchResultView {
    SearchResultView {
        slug: item.slug.clone(),
        name: item.name.clone(),
        project_id: item.project_id.clone(),
        platform: platform.to_string(),
        description: item.description.clone(),
        latest_version: item.latest_version.clone(),
        downloads: item.downloads,
        mc_versions: item.mc_versions.clone(),
        client_side: side_label(item.client_side.as_ref()),
        server_side: side_label(item.server_side.as_ref()),
        categories: item.categories.clone(),
        icon_url: item.icon_url.clone(),
        accent_color: item.accent_color,
        compatible: ref_mc.map(|rmc| item.mc_versions.iter().any(|v| v == rmc)),
    }
}

pub fn info_view(provider: &str, info: &ModInfo) -> InfoOutput {
    InfoOutput {
        provider: provider.to_string(),
        project_id: info.project_id.clone(),
        slug: info.slug.clone(),
        name: info.name.clone(),
        description: info.description.clone(),
        authors: info.authors.clone(),
        latest_version: info.latest_version.clone(),
        downloads: info.downloads,
        license: info.license.clone(),
        client_side: side_label(info.client_side.as_ref()),
        server_side: side_label(info.server_side.as_ref()),
        categories: info.categories.clone(),
        icon_url: info.icon_url.clone(),
        accent_color: info.accent_color,
        website_url: info.website_url.clone(),
        source_url: info.source_url.clone(),
        issues_url: info.issues_url.clone(),
        wiki_url: info.wiki_url.clone(),
        gallery: info
            .gallery
            .iter()
            .map(|image| ProjectImageView {
                url: image.url.clone(),
                thumbnail_url: image.thumbnail_url.clone(),
                title: image.title.clone(),
                description: image.description.clone(),
            })
            .collect(),
        recent_versions: info
            .recent_versions
            .iter()
            .map(|v| ModVersionView {
                version: v.version.clone(),
                mc_versions: v.mc_versions.clone(),
                loader: v.loader.clone(),
                released_at: v.released_at.clone(),
            })
            .collect(),
        dependencies: info
            .dependencies
            .iter()
            .map(|d| DependencyView {
                slug: d.slug.clone(),
                project_id: d.project_id.clone(),
                required: d.required,
            })
            .collect(),
    }
}

pub fn listed_package_view(
    pkg: &ListedPackage,
    presentation_cache: Option<&std::path::Path>,
) -> ListedPackageView {
    ListedPackageView {
        mod_id: pkg.mod_id.clone(),
        version: pkg.version.clone(),
        version_constraint: pkg.version_constraint.clone(),
        enabled: pkg.enabled,
        icon_path: presentation_cache
            .and_then(|cache| orbit_core::materialize_listed_package_icon(pkg, cache).ok())
            .flatten()
            .map(|path| path.to_string_lossy().into_owned()),
        remotes: pkg.remotes.clone(),
        configured_environment: pkg.configured_environment.clone(),
        environment: pkg.environment.clone(),
        optional: pkg.optional,
        dependencies: pkg.dependencies.clone(),
        bundled_count: pkg.bundled.len(),
    }
}

pub fn outdated_mod_view(m: &OutdatedMod) -> OutdatedModView {
    OutdatedModView {
        mod_id: m.mod_id.clone(),
        current_version: m.current_version.clone(),
        new_version: m.new_version.clone(),
    }
}

pub fn diagnostic_view(d: &CandidateDiagnostic) -> DiagnosticView {
    DiagnosticView {
        package: d.package.clone(),
        selected_version: d.selected_version.clone(),
        candidate_version: d.candidate_version.clone(),
        kind: diagnostic_kind_code(&d.kind).to_string(),
        facts: d.facts.clone(),
    }
}

pub fn package_change_view(c: &PackageChange) -> PackageChangeView {
    PackageChangeView {
        package: c.package.clone(),
        kind: change_kind_code(c.kind).to_string(),
        current_version: c.current_version.clone(),
        selected_version: c.selected_version.clone(),
        selected_description: c.selected_description.clone(),
    }
}

pub fn removed_package_view(r: &RemovedPackage) -> RemovedPackageView {
    RemovedPackageView {
        mod_id: r.mod_id.clone(),
        version: r.version.clone(),
    }
}

pub fn instance_view(entry: &InstanceEntry, is_current: bool) -> InstanceView {
    InstanceView {
        name: entry.name.clone(),
        path: entry.path.clone(),
        mc_version: entry.mc_version.clone(),
        modloader: entry.modloader.clone(),
        is_default: entry.is_default,
        is_current,
    }
}

pub fn remote_view(report: &RemoteReport, subcommand: &str) -> RemoteOutput {
    RemoteOutput {
        subcommand: subcommand.to_string(),
        package: report.package.clone(),
        changed: report.changed,
        remotes: report
            .remotes
            .iter()
            .enumerate()
            .map(|(i, r)| RemoteEntryView {
                index: i + 1,
                provider: r.provider().to_string(),
                locator: r.display_locator(),
            })
            .collect(),
    }
}

pub fn platform_change_view(c: &orbit_core::PlatformChange) -> PlatformChangeView {
    PlatformChangeView {
        field: c.field.to_string(),
        previous: c.previous.clone(),
        current: c.current.clone(),
    }
}

pub fn sync_view(report: &SyncReport, dry_run: bool) -> SyncOutput {
    SyncOutput {
        dry_run,
        summary: SyncSummary {
            platform_changes: report.platform_changes.len(),
            added: report.added.len(),
            changed: report.changed.len(),
            removed: report.removed.len(),
        },
        platform_changes: report
            .platform_changes
            .iter()
            .map(platform_change_view)
            .collect(),
        added: report.added.clone(),
        changed: report.changed.clone(),
        removed: report.removed.clone(),
        warnings: report.warnings.clone(),
    }
}

/// Tree view used by `orbit list --tree`.
pub fn list_view(
    packages: &[ListedPackage],
    target: Option<&str>,
    tree: bool,
    presentation_cache: Option<&std::path::Path>,
) -> ListOutput {
    ListOutput {
        target: target.map(str::to_string),
        tree,
        packages: packages
            .iter()
            .map(|package| listed_package_view(package, presentation_cache))
            .collect(),
    }
}

// ---------------------------------------------------------------------------
// Transaction report (install / add / upgrade)
// ---------------------------------------------------------------------------

use orbit_core::{InstallReport, InstanceInstallReport};

pub fn package_version_policy_view(
    policy: &orbit_core::PackageVersionPolicy,
) -> PackageVersionPolicyOutput {
    use orbit_core::PackageVersionPolicy;

    match policy {
        PackageVersionPolicy::Any => PackageVersionPolicyOutput {
            kind: "any".to_string(),
            operator: None,
            version: None,
            lower: None,
            upper: None,
            include_lower: None,
            include_upper: None,
            requirement: None,
        },
        PackageVersionPolicy::Comparison { operator, version } => PackageVersionPolicyOutput {
            kind: "comparison".to_string(),
            operator: Some(operator.operator().to_string()),
            version: Some(version.clone()),
            lower: None,
            upper: None,
            include_lower: None,
            include_upper: None,
            requirement: None,
        },
        PackageVersionPolicy::Range {
            lower,
            upper,
            include_lower,
            include_upper,
        } => PackageVersionPolicyOutput {
            kind: "range".to_string(),
            operator: None,
            version: None,
            lower: Some(lower.clone()),
            upper: Some(upper.clone()),
            include_lower: Some(*include_lower),
            include_upper: Some(*include_upper),
            requirement: None,
        },
        PackageVersionPolicy::Custom(requirement) => PackageVersionPolicyOutput {
            kind: "custom".to_string(),
            operator: None,
            version: None,
            lower: None,
            upper: None,
            include_lower: None,
            include_upper: None,
            requirement: Some(requirement.clone()),
        },
    }
}

pub fn install_instance_view(
    report: &InstanceInstallReport,
    dry_run: bool,
) -> InstanceInstallOutput {
    InstanceInstallOutput {
        dry_run,
        summary: InstanceInstallSummary {
            installed: report.installed.len(),
            already_present: report.already_present.len(),
            skipped: report.skipped.len(),
        },
        installed: report.installed.clone(),
        already_present: report.already_present.clone(),
        skipped: report.skipped.clone(),
    }
}

pub fn transaction_view(report: &InstallReport, dry_run: bool) -> TransactionOutput {
    TransactionOutput {
        dry_run,
        summary: TransactionSummary {
            installed: report.installed.len(),
            removed: report.removed.len(),
            already_satisfied: report.already_satisfied.len(),
            skipped_optional: report.skipped_optional.len(),
        },
        changes: report.changes.iter().map(package_change_view).collect(),
        installed: report
            .installed
            .iter()
            .map(|m| InstalledView {
                mod_id: m.mod_id.clone(),
                version: m.version.clone(),
            })
            .collect(),
        removed: report.removed.iter().map(removed_package_view).collect(),
        already_satisfied: report.already_satisfied.clone(),
        diagnostics: report.diagnostics.iter().map(diagnostic_view).collect(),
        warnings: report.warnings.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use orbit_core::{
        InstallReport, ListedPackage, OutdatedMod, PackageChange, PackageChangeKind,
        RemovedPackage,
        resolver::types::{CandidateDiagnostic, CandidateDiagnosticKind},
    };

    #[test]
    fn json_envelope_wraps_result_with_schema_version_and_command() {
        let view = SearchOutput {
            query: "sodium".into(),
            platforms: vec!["modrinth".into()],
            filters: SearchFilters {
                mc_version: None,
                modloader: None,
            },
            ref_mc_version: None,
            results: Vec::new(),
            truncated: false,
        };
        let envelope = JsonEnvelope::new("search", &view);
        let json = serde_json::to_value(&envelope).unwrap();
        assert_eq!(
            json["schema_version"],
            orbit_machine_protocol::SCHEMA_VERSION
        );
        assert_eq!(json["command"], "search");
        assert_eq!(json["ok"], true);
        assert_eq!(json["result"]["query"], "sodium");
    }

    #[test]
    fn package_environment_json_distinguishes_auto_from_effective_value() {
        let view = PackageEnvironmentOutput {
            package: "sodium".into(),
            configured: None,
            effective: Some("client".into()),
            dry_run: false,
        };
        let envelope = JsonEnvelope::new("env", &view);
        let json = serde_json::to_value(&envelope).unwrap();

        assert!(json["result"]["configured"].is_null());
        assert_eq!(json["result"]["effective"], "client");
    }

    #[test]
    fn package_list_exposes_constraint_and_configured_environment_for_management() {
        let package = ListedPackage {
            mod_id: "sodium".into(),
            version: "0.9.1".into(),
            version_constraint: "*".into(),
            enabled: true,
            remotes: vec!["modrinth:AANobbMI".into()],
            configured_environment: None,
            environment: "client".into(),
            optional: false,
            dependencies: Vec::new(),
            bundled: Vec::new(),
            icon: None,
        };

        let value = serde_json::to_value(listed_package_view(&package, None)).unwrap();
        assert_eq!(value["version_constraint"], "*");
        assert_eq!(value["enabled"], true);
        assert!(value.get("root").is_none());
        assert!(value["configured_environment"].is_null());
        assert_eq!(value["environment"], "client");
    }

    #[test]
    fn diagnostic_kind_maps_to_stable_snake_case_code() {
        assert_eq!(
            diagnostic_kind_code(&CandidateDiagnosticKind::ExcludedByPropagation),
            "excluded_by_propagation"
        );
        assert_eq!(
            diagnostic_kind_code(&CandidateDiagnosticKind::NoCompatibleCandidate),
            "no_compatible_candidate"
        );
    }

    #[test]
    fn change_kind_maps_to_stable_snake_case_code() {
        assert_eq!(change_kind_code(PackageChangeKind::Upgrade), "upgrade");
        assert_eq!(change_kind_code(PackageChangeKind::Downgrade), "downgrade");
    }

    #[test]
    fn package_change_view_omits_filename_and_internal_hashes() {
        let change = PackageChange {
            package: "sodium".into(),
            current_version: Some("0.5.7".into()),
            selected_version: Some("0.5.8".into()),
            filename: Some("sodium-fabric-mc1.21-secret.jar".into()),
            selected_filename: Some("sodium-new.jar".into()),
            selected_description: Some("Modrinth project AANobbMI".into()),
            kind: PackageChangeKind::Upgrade,
        };
        let view = package_change_view(&change);
        let json = serde_json::to_value(&view).unwrap();
        assert!(json["filename"].is_null());
        assert!(json["selected_filename"].is_null());
        assert!(!json.to_string().contains(".jar"));
        assert_eq!(json["kind"], "upgrade");
        assert_eq!(json["selected_description"], "Modrinth project AANobbMI");
    }

    #[test]
    fn transaction_view_never_serializes_hashes_or_filenames() {
        let report = InstallReport {
            installed: Vec::new(),
            removed: vec![RemovedPackage {
                mod_id: "voxy".into(),
                version: "1.0".into(),
                filename: "voxy-secret.jar".into(),
            }],
            changes: Vec::new(),
            already_satisfied: Vec::new(),
            skipped_optional: Vec::new(),
            diagnostics: vec![CandidateDiagnostic {
                package: "voxy".into(),
                selected_version: "1.0".into(),
                candidate_version: "2.0".into(),
                kind: CandidateDiagnosticKind::ExcludedByPropagation,
                facts: vec!["requires sodium =0.8.9".into()],
            }],
            warnings: Vec::new(),
        };
        let view = transaction_view(&report, false);
        let json = serde_json::to_string(&view).unwrap();
        assert!(!json.contains(".jar"));
        assert!(!json.contains("secret"));
        assert!(json.contains("excluded_by_propagation"));
        assert!(json.contains("requires sodium"));
    }

    #[test]
    fn outdated_summary_counts_upgrades() {
        let updates = [
            OutdatedMod {
                mod_id: "sodium".into(),
                current_version: "0.5.7".into(),
                new_version: "0.5.8".into(),
                candidate_id: "secret-hash".into(),
            },
            OutdatedMod {
                mod_id: "lithium".into(),
                current_version: "0.2".into(),
                new_version: "0.3".into(),
                candidate_id: "another-secret".into(),
            },
        ];
        let summary = OutdatedSummary {
            upgrades: updates.len(),
            up_to_date: 0,
        };
        let json = serde_json::to_value(&summary).unwrap();
        assert_eq!(json["upgrades"], 2);
        // candidate_id must not appear in any view-model field.
        assert!(!serde_json::to_string(&summary).unwrap().contains("secret"));
    }

    #[test]
    fn error_json_has_type_error_and_stable_code() {
        let error = ErrorJson::new("info", "mod_not_found", "mod 'foo' not found");
        let json = serde_json::to_value(&error).unwrap();
        assert_eq!(json["type"], "error");
        assert_eq!(json["command"], "info");
        assert_eq!(json["ok"], false);
        assert_eq!(json["code"], "mod_not_found");
        assert_eq!(json["message"], "mod 'foo' not found");
        assert!(json.get("detail").is_none());
    }

    #[test]
    fn search_json_exposes_presentation_metadata_without_artifact_identity() {
        let item = SearchResultItem {
            project_id: "AANobbMI".into(),
            slug: "sodium".into(),
            name: "Sodium".into(),
            description: "Renderer".into(),
            latest_version: "mc26.1-0.9.1".into(),
            downloads: 42,
            mc_versions: vec!["26.1".into()],
            client_side: Some(SideSupport::Required),
            server_side: Some(SideSupport::Unsupported),
            categories: vec!["optimization".into()],
            icon_url: Some("https://cdn.modrinth.com/icon.png".into()),
            accent_color: Some(0x12_34_56),
        };

        let value =
            serde_json::to_value(search_result_view("modrinth", &item, Some("26.1"))).unwrap();
        assert_eq!(value["icon_url"], "https://cdn.modrinth.com/icon.png");
        assert_eq!(value["accent_color"], 0x12_34_56);
        assert_eq!(value["latest_version"], "mc26.1-0.9.1");
        assert_eq!(value["client_side"], "required");
        assert_eq!(value["server_side"], "unsupported");
        assert!(value.get("sha512").is_none());
        assert!(value.get("filename").is_none());
    }
}
