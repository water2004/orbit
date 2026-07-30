use std::collections::{BTreeMap, HashMap};
use std::ops::Bound;

use crate::lockfile::ArtifactSource;
use crate::manifest::PackageRemote;
use crate::metadata::{
    DependencyExpression, EmbeddedArtifact, Environment, LanguageLoaderRequirement,
    ModLoadCondition, ProvidedMod,
};
use crate::providers::RemoteArtifact;
use crate::versions::Version;
use pubgrub::Ranges;

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum SolverVersion {
    Version {
        semantic: Version,
        identity: VersionIdentity,
    },
    LoadPreference(bool),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum VersionIdentity {
    LowerBound,
    Platform,
    Candidate(CandidateIdentity),
    UpperBound,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct CandidateIdentity {
    pub(crate) owner: String,
    pub(crate) source: String,
    pub(crate) path: Vec<usize>,
    pub(crate) location: CandidateLocation,
    pub(crate) installed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum CandidateLocation {
    Nested,
    SameFile,
    Root,
}

impl SolverVersion {
    pub(crate) fn platform(version: Version) -> Self {
        Self::Version {
            semantic: version,
            identity: VersionIdentity::Platform,
        }
    }

    pub(crate) fn candidate(version: Version, identity: CandidateIdentity) -> Self {
        Self::Version {
            semantic: version,
            identity: VersionIdentity::Candidate(identity),
        }
    }

    fn lower_bound(version: Version) -> Self {
        Self::Version {
            semantic: version,
            identity: VersionIdentity::LowerBound,
        }
    }

    fn upper_bound(version: Version) -> Self {
        Self::Version {
            semantic: version,
            identity: VersionIdentity::UpperBound,
        }
    }

    pub(crate) fn domain(&self) -> Option<&Version> {
        match self {
            Self::Version { semantic, .. } => Some(semantic),
            Self::LoadPreference(_) => None,
        }
    }

    pub(crate) fn candidate_identity(&self) -> Option<&CandidateIdentity> {
        match self {
            Self::Version {
                identity: VersionIdentity::Candidate(identity),
                ..
            } => Some(identity),
            _ => None,
        }
    }

    /// Return every solver representation of the same physical realization.
    ///
    /// The lock graph prefixes content identities with `lock:` so it can keep
    /// installed candidates preferentially ordered. A freshly downloaded
    /// candidate uses the bare content identity. Those are two representations
    /// of one JAR, not two choices for the user, and Pareto enumeration must
    /// exclude them as one equivalence class.
    pub(crate) fn same_realization(&self) -> Ranges<Self> {
        let Self::Version {
            semantic,
            identity: VersionIdentity::Candidate(identity),
        } = self
        else {
            return Ranges::singleton(self.clone());
        };

        let mut sources = vec![identity.source.clone()];
        if let Some(content) = identity
            .source
            .strip_prefix("lock:sha512:")
            .map(|digest| format!("sha512:{digest}"))
            .or_else(|| {
                identity
                    .source
                    .strip_prefix("lock:sha256:")
                    .map(|digest| format!("sha256:{digest}"))
            })
        {
            sources.push(content);
        } else if identity.source.starts_with("sha512:") || identity.source.starts_with("sha256:") {
            sources.push(format!("lock:{}", identity.source));
        }
        sources.sort();
        sources.dedup();

        sources
            .into_iter()
            .flat_map(|source| {
                [false, true].map(move |installed| {
                    let mut identity = identity.clone();
                    identity.source = source.clone();
                    identity.installed = installed;
                    SolverVersion::candidate(semantic.clone(), identity)
                })
            })
            .fold(Ranges::empty(), |range, version| {
                range.union(&Ranges::singleton(version))
            })
    }
}

impl From<Version> for SolverVersion {
    fn from(version: Version) -> Self {
        Self::platform(version)
    }
}

impl std::fmt::Display for SolverVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Version { semantic, .. } => semantic.fmt(f),
            Self::LoadPreference(true) => f.write_str("load"),
            Self::LoadPreference(false) => f.write_str("omit"),
        }
    }
}

pub(crate) fn solver_range(range: Ranges<Version>) -> Ranges<SolverVersion> {
    range
        .into_iter()
        .map(|(start, end)| (map_start_bound(start), map_end_bound(end)))
        .collect()
}

fn map_start_bound(bound: Bound<Version>) -> Bound<SolverVersion> {
    match bound {
        Bound::Included(version) => Bound::Included(SolverVersion::lower_bound(version)),
        Bound::Excluded(version) => Bound::Excluded(SolverVersion::upper_bound(version)),
        Bound::Unbounded => Bound::Unbounded,
    }
}

fn map_end_bound(bound: Bound<Version>) -> Bound<SolverVersion> {
    match bound {
        Bound::Included(version) => Bound::Included(SolverVersion::upper_bound(version)),
        Bound::Excluded(version) => Bound::Excluded(SolverVersion::lower_bound(version)),
        Bound::Unbounded => Bound::Unbounded,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum SolverPackage {
    Root,
    Mod(String),
    EmbeddedArtifact(String),
    LoadPreference {
        parent: CandidateIdentity,
        mod_id: String,
    },
    Platform(String),
}

impl SolverPackage {
    pub(crate) fn logical(package: impl Into<String>) -> Self {
        let package = package.into();
        if crate::resolver::graph::is_platform_package(&package) {
            Self::Platform(package)
        } else {
            Self::Mod(package)
        }
    }

    pub(crate) fn top_level_mod_id(&self) -> Option<&str> {
        match self {
            Self::Mod(mod_id) => Some(mod_id),
            _ => None,
        }
    }

    pub(crate) fn user_label(&self) -> &str {
        match self {
            Self::Root => "the project",
            Self::Mod(mod_id)
            | Self::EmbeddedArtifact(mod_id)
            | Self::LoadPreference { mod_id, .. }
            | Self::Platform(mod_id) => mod_id,
        }
    }
}

impl std::fmt::Display for SolverPackage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Root => write!(f, "the project"),
            Self::Mod(id) => write!(f, "{id}"),
            Self::EmbeddedArtifact(id) => {
                write!(f, "{id} embedded artifact")
            }
            Self::LoadPreference { parent, mod_id } => write!(
                f,
                "{mod_id} nested load preference in {} from {}",
                parent.owner, parent.source
            ),
            Self::Platform(id) => write!(f, "{id}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateVersion {
    /// Locally computed content identity; never intended for user-facing output.
    pub id: String,
    /// Download filename used only to materialize the selected artifact.
    pub filename: String,
    /// Human-readable provider labels, without provider ids or content hashes.
    pub display_sources: Vec<String>,
    pub jar_version: String,
    pub dependencies: Vec<DependencyExpression>,
    pub environment: Environment,
    pub provides: Vec<ProvidedMod>,
    pub language_loader: Option<LanguageLoaderRequirement>,
    pub embedded_artifacts: Vec<EmbeddedArtifact>,
    pub bundled: Vec<BundledCandidate>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedArtifact {
    pub filename: String,
    pub sha1: String,
    pub sha256: String,
    pub sha512: String,
    pub sources: Vec<ArtifactSource>,
}

pub type ResolvedCandidates = HashMap<String, ResolvedArtifact>;

#[derive(Debug, Clone, Default)]
pub struct CandidateCatalog {
    pub candidates: HashMap<String, Vec<CandidateVersion>>,
    pub resolved: ResolvedCandidates,
    /// Loader package metadata read from the actual launcher library JAR.
    pub loader_package: Option<PlatformCandidate>,
    /// Provider lookup key (slug or project id) to every JAR-declared package id.
    ///
    /// A provider project is only a download locator. Its artifacts may change
    /// `mod_id` over time, so it must not impose a single package identity.
    pub remote_packages: HashMap<PackageRemote, std::collections::BTreeSet<String>>,
    /// JAR-declared packages found directly in the remote(s) named by `add`.
    pub requested_packages: std::collections::BTreeSet<String>,
    /// Canonical provider project/file remote for each directly requested
    /// JAR-declared package. Recursive dependency projects are excluded.
    pub requested_remotes: HashMap<String, std::collections::BTreeSet<PackageRemote>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlatformCandidate {
    pub mod_id: String,
    pub version: String,
    pub dependencies: Vec<DependencyExpression>,
    pub environment: Environment,
    pub provides: Vec<ProvidedMod>,
    pub language_loader: Option<LanguageLoaderRequirement>,
    pub embedded_artifacts: Vec<EmbeddedArtifact>,
    pub bundled: Vec<BundledCandidate>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundledCandidate {
    pub mod_id: String,
    pub version: String,
    pub load_condition: ModLoadCondition,
    pub origin: crate::jar::JarModOrigin,
    pub environment: Environment,
    pub dependencies: Vec<DependencyExpression>,
    pub provides: Vec<ProvidedMod>,
    pub language_loader: Option<LanguageLoaderRequirement>,
    pub embedded_artifacts: Vec<EmbeddedArtifact>,
    pub bundled: Vec<BundledCandidate>,
}

impl CandidateVersion {
    pub fn from_jar_metadata(
        id: String,
        filename: String,
        display_source: String,
        metadata: crate::jar::JarModMetadata,
    ) -> Self {
        Self {
            id,
            filename,
            display_sources: vec![display_source],
            jar_version: metadata.version,
            dependencies: metadata.dependencies,
            environment: metadata.environment,
            provides: metadata.provides,
            language_loader: metadata.language_loader,
            embedded_artifacts: metadata.embedded_artifacts,
            bundled: metadata
                .bundled_mods
                .into_iter()
                .map(BundledCandidate::from_jar)
                .collect(),
        }
    }

    fn metadata_equivalent(&self, other: &Self) -> bool {
        self.jar_version == other.jar_version
            && self.dependencies == other.dependencies
            && self.environment == other.environment
            && self.provides == other.provides
            && self.language_loader == other.language_loader
            && self.embedded_artifacts == other.embedded_artifacts
            && self.bundled == other.bundled
    }

    fn add_display_source(&mut self, source: String) {
        if !self.display_sources.contains(&source) {
            self.display_sources.push(source);
            self.display_sources.sort();
        }
    }

    pub fn display_description(&self) -> String {
        let source = if self.display_sources.is_empty() {
            "downloaded artifact".to_string()
        } else {
            self.display_sources.join(", ")
        };
        let dependency_count = self
            .dependencies
            .iter()
            .flat_map(DependencyExpression::relations)
            .filter(|dependency| dependency.kind.installs_target())
            .count();
        let bundled_count = bundled_module_count(&self.bundled);
        let mut details = Vec::new();
        if dependency_count > 0 {
            details.push(counted_label(
                dependency_count,
                "dependency constraint",
                "dependency constraints",
            ));
        }
        if bundled_count > 0 {
            details.push(counted_label(
                bundled_count,
                "bundled module",
                "bundled modules",
            ));
        }
        if self.environment != Environment::Both {
            details.push(format!("{} environment", self.environment.as_str()));
        }
        if details.is_empty() {
            source
        } else {
            format!("{source} · {}", details.join(" · "))
        }
    }
}

fn counted_label(count: usize, singular: &str, plural: &str) -> String {
    format!("{count} {}", if count == 1 { singular } else { plural })
}

fn bundled_module_count(bundled: &[BundledCandidate]) -> usize {
    bundled
        .iter()
        .map(|module| 1 + bundled_module_count(&module.bundled))
        .sum()
}

impl PlatformCandidate {
    pub(crate) fn from_jar_metadata(metadata: crate::jar::JarModMetadata) -> Self {
        Self {
            mod_id: metadata.mod_id,
            version: metadata.version,
            dependencies: metadata.dependencies,
            environment: metadata.environment,
            provides: metadata.provides,
            language_loader: metadata.language_loader,
            embedded_artifacts: metadata.embedded_artifacts,
            bundled: metadata
                .bundled_mods
                .into_iter()
                .map(BundledCandidate::from_jar)
                .collect(),
        }
    }
}

impl CandidateCatalog {
    #[cfg(test)]
    pub(crate) fn record_test(
        &mut self,
        metadata: crate::jar::JarModMetadata,
        artifact: RemoteArtifact,
    ) -> Result<String, crate::error::OrbitError> {
        let identity = artifact
            .version_id()
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| format!("{}:{}", artifact.provider, artifact.download_url));
        let inspected = crate::jar::InspectedJar {
            metadata: metadata.clone(),
            sha1: format!("sha1-{identity}"),
            sha256: format!("sha256-{identity}"),
            sha512: format!("sha512-{identity}"),
        };
        self.record(metadata, artifact, &inspected, true)
    }

    pub(crate) fn record(
        &mut self,
        metadata: crate::jar::JarModMetadata,
        artifact: RemoteArtifact,
        inspected: &crate::jar::InspectedJar,
        requested: bool,
    ) -> Result<String, crate::error::OrbitError> {
        if metadata.mod_id.is_empty() {
            return Err(crate::error::OrbitError::Other(anyhow::anyhow!(
                "downloaded artifact '{}' has an empty JAR mod_id",
                artifact.filename
            )));
        }
        if metadata.version.is_empty() {
            return Err(crate::error::OrbitError::Other(anyhow::anyhow!(
                "downloaded artifact '{}' has an empty JAR version",
                artifact.filename
            )));
        }
        let package = metadata.mod_id.clone();
        let candidate_id = format!("sha512:{}", inspected.sha512);
        let display_source = artifact.display_source();
        let candidate = CandidateVersion::from_jar_metadata(
            candidate_id.clone(),
            artifact.filename.clone(),
            display_source.clone(),
            metadata,
        );
        let existing = self.candidates.get(&package).and_then(|versions| {
            versions
                .iter()
                .position(|existing| existing.id == candidate.id)
        });
        if existing
            .is_some_and(|index| !self.candidates[&package][index].metadata_equivalent(&candidate))
        {
            return Err(crate::error::OrbitError::Other(anyhow::anyhow!(
                "the same downloaded content was parsed with inconsistent JAR metadata"
            )));
        }
        let duplicate = existing.is_some();
        let remote = artifact.package_remote()?;
        self.remote_packages
            .entry(remote.clone())
            .or_default()
            .insert(package.clone());
        if requested {
            self.requested_packages.insert(package.clone());
            self.requested_remotes
                .entry(package.clone())
                .or_default()
                .insert(remote);
        }
        if !duplicate {
            self.candidates
                .entry(package.clone())
                .or_default()
                .push(candidate);
        } else if let Some(index) = existing {
            self.candidates
                .get_mut(&package)
                .expect("candidate package exists")[index]
                .add_display_source(display_source);
        }
        let source = artifact.artifact_source()?;
        let resolved = self
            .resolved
            .entry(candidate_id)
            .or_insert_with(|| ResolvedArtifact {
                filename: artifact.filename.clone(),
                sha1: inspected.sha1.clone(),
                sha256: inspected.sha256.clone(),
                sha512: inspected.sha512.clone(),
                sources: Vec::new(),
            });
        if !resolved.sources.contains(&source) {
            resolved.sources.push(source);
            resolved.sources.sort_by_key(|source| format!("{source:?}"));
        }
        Ok(package)
    }

    pub(crate) fn record_local(
        &mut self,
        inspected: crate::jar::InspectedJar,
        path: String,
        filename: String,
        requested: bool,
    ) -> Result<String, crate::error::OrbitError> {
        let remote = PackageRemote::File { path: path.clone() };
        let package = inspected.metadata.mod_id.clone();
        if package.is_empty() {
            return Err(crate::error::OrbitError::Other(anyhow::anyhow!(
                "local JAR has an empty mod_id"
            )));
        }
        if inspected.metadata.version.is_empty() {
            return Err(crate::error::OrbitError::Other(anyhow::anyhow!(
                "local package '{package}' has an empty JAR version"
            )));
        }
        let candidate_id = format!("sha512:{}", inspected.sha512);
        let candidate = CandidateVersion::from_jar_metadata(
            candidate_id.clone(),
            filename.clone(),
            "local file".to_string(),
            inspected.metadata,
        );
        if let Some(existing) = self
            .candidates
            .get(&package)
            .and_then(|versions| versions.iter().find(|existing| existing.id == candidate.id))
            && !existing.metadata_equivalent(&candidate)
        {
            return Err(crate::error::OrbitError::Other(anyhow::anyhow!(
                "the same content hash was parsed with different JAR metadata"
            )));
        }
        if !self
            .candidates
            .get(&package)
            .is_some_and(|versions| versions.iter().any(|existing| existing.id == candidate.id))
        {
            self.candidates
                .entry(package.clone())
                .or_default()
                .push(candidate);
        }
        self.remote_packages
            .entry(remote.clone())
            .or_default()
            .insert(package.clone());
        if requested {
            self.requested_packages.insert(package.clone());
            self.requested_remotes
                .entry(package.clone())
                .or_default()
                .insert(remote);
        }
        let source = ArtifactSource::File { path };
        let resolved = self
            .resolved
            .entry(candidate_id)
            .or_insert_with(|| ResolvedArtifact {
                filename,
                sha1: inspected.sha1,
                sha256: inspected.sha256,
                sha512: inspected.sha512,
                sources: Vec::new(),
            });
        if !resolved.sources.contains(&source) {
            resolved.sources.push(source);
            resolved.sources.sort_by_key(|source| format!("{source:?}"));
        }
        Ok(package)
    }

    #[cfg(test)]
    pub(crate) fn packages_for_remote(&self, remote: &PackageRemote) -> Vec<String> {
        self.remote_packages
            .get(remote)
            .into_iter()
            .flat_map(|packages| packages.iter().cloned())
            .collect()
    }

    pub(crate) fn remotes_for_package(&self, package: &str) -> Vec<PackageRemote> {
        self.remote_packages
            .iter()
            .filter(|(_, packages)| packages.contains(package))
            .map(|(remote, _)| remote.clone())
            .collect()
    }

    pub(crate) fn requested_remotes_for_package(&self, package: &str) -> Vec<PackageRemote> {
        self.requested_remotes
            .get(package)
            .into_iter()
            .flat_map(|remotes| remotes.iter().cloned())
            .collect()
    }

    pub(crate) fn package_remotes(&self) -> HashMap<String, Vec<PackageRemote>> {
        let mut result: HashMap<String, Vec<PackageRemote>> = HashMap::new();
        for (remote, packages) in &self.remote_packages {
            for package in packages {
                result
                    .entry(package.clone())
                    .or_default()
                    .push(remote.clone());
            }
        }
        for remotes in result.values_mut() {
            remotes.sort();
            remotes.dedup();
        }
        result
    }
}

impl BundledCandidate {
    fn from_jar(metadata: crate::jar::JarModMetadata) -> Self {
        Self {
            mod_id: metadata.mod_id,
            version: metadata.version,
            load_condition: metadata.load_condition,
            origin: metadata.origin,
            environment: metadata.environment,
            dependencies: metadata.dependencies,
            provides: metadata.provides,
            language_loader: metadata.language_loader,
            embedded_artifacts: metadata.embedded_artifacts,
            bundled: metadata
                .bundled_mods
                .into_iter()
                .map(Self::from_jar)
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateDiagnosticKind {
    NoCompatibleCandidate,
    ExcludedByPropagation,
    Backtracked,
    Unexplained,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateDiagnostic {
    pub package: String,
    pub selected_version: String,
    pub candidate_version: String,
    pub kind: CandidateDiagnosticKind,
    pub facts: Vec<String>,
}

impl std::fmt::Display for CandidateDiagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.kind {
            CandidateDiagnosticKind::NoCompatibleCandidate => write!(
                f,
                "{} stayed at {}; no compatible remote candidate was discovered",
                self.package, self.selected_version
            )?,
            CandidateDiagnosticKind::ExcludedByPropagation => write!(
                f,
                "{} stayed at {}; candidate {} was excluded by dependency propagation",
                self.package, self.selected_version, self.candidate_version
            )?,
            CandidateDiagnosticKind::Backtracked => write!(
                f,
                "{} stayed at {}; candidate {} was tried, then backtracked after a conflict",
                self.package, self.selected_version, self.candidate_version
            )?,
            CandidateDiagnosticKind::Unexplained => write!(
                f,
                "{} stayed at {}; candidate {} was not selected, but no excluding derivation was recorded",
                self.package, self.selected_version, self.candidate_version
            )?,
        }
        for fact in &self.facts {
            write!(f, "\n  - {fact}")?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default)]
pub struct ResolutionReport {
    /// Selected semantic version per top-level package.
    pub selected_versions: BTreeMap<String, String>,
    /// Selected candidate identity per top-level package, including installed candidates.
    pub selected_sources: BTreeMap<String, String>,
    /// Selected remote candidate per top-level package; installed candidates are omitted.
    pub selected_candidates: BTreeMap<String, String>,
    /// Complete top-level package changes relative to the installed set.
    pub changes: Vec<PackageChange>,
    pub diagnostics: Vec<CandidateDiagnostic>,
    pub warnings: Vec<String>,
}

impl ResolutionReport {
    pub fn has_upgrade(&self) -> bool {
        self.changes
            .iter()
            .any(|change| change.kind == PackageChangeKind::Upgrade)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageChangeKind {
    Install,
    Upgrade,
    Downgrade,
    Replace,
    Remove,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageChange {
    pub package: String,
    pub current_version: Option<String>,
    pub selected_version: Option<String>,
    /// Existing top-level JAR removed or replaced by this change.
    pub filename: Option<String>,
    /// Concrete top-level JAR selected for installation, when known.
    pub selected_filename: Option<String>,
    /// Human-readable candidate provenance and relevant declared constraints.
    /// Content hashes and physical filenames must not be placed here.
    pub selected_description: Option<String>,
    pub kind: PackageChangeKind,
}

#[derive(Debug, Clone, Default)]
pub struct ResolutionPortfolio {
    pub alternatives: Vec<ResolutionReport>,
}

pub type ResolutionSelector =
    Box<dyn FnOnce(&[ResolutionReport]) -> Result<usize, crate::OrbitError> + Send>;
