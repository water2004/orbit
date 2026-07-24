use std::collections::{BTreeMap, HashMap};
use std::ops::Bound;

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
    /// Provider-artifact identity; independent from JAR-declared mod ID/version.
    pub id: String,
    /// Download filename used to distinguish concrete package candidates in plans.
    pub filename: String,
    pub jar_version: String,
    pub dependencies: Vec<DependencyExpression>,
    pub environment: Environment,
    pub provides: Vec<ProvidedMod>,
    pub language_loader: Option<LanguageLoaderRequirement>,
    pub embedded_artifacts: Vec<EmbeddedArtifact>,
    pub bundled: Vec<BundledCandidate>,
}

pub type ResolvedCandidates = HashMap<String, RemoteArtifact>;

#[derive(Debug, Clone, Default)]
pub struct CandidateCatalog {
    pub candidates: HashMap<String, Vec<CandidateVersion>>,
    pub resolved: ResolvedCandidates,
    /// Provider lookup key (slug or project id) to the JAR-declared package id.
    pub source_packages: HashMap<(String, String), String>,
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
        metadata: crate::jar::JarModMetadata,
    ) -> Self {
        Self {
            id,
            filename,
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
}

impl CandidateCatalog {
    pub(crate) fn record(
        &mut self,
        metadata: crate::jar::JarModMetadata,
        artifact: RemoteArtifact,
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
        let candidate_id = artifact.candidate_id();
        let candidate = CandidateVersion::from_jar_metadata(
            candidate_id.clone(),
            artifact.filename.clone(),
            metadata,
        );
        let existing = self
            .candidates
            .get(&package)
            .and_then(|versions| versions.iter().find(|existing| existing.id == candidate.id));
        if existing.is_some_and(|existing| existing != &candidate) {
            return Err(crate::error::OrbitError::Other(anyhow::anyhow!(
                "provider artifact '{}' was parsed with different metadata from its JAR",
                candidate.id
            )));
        }
        let duplicate = existing.is_some();
        self.record_source_alias(&artifact.provider, &artifact.slug, &package)?;
        if let Some(project_id) = artifact.project_id() {
            self.record_source_alias(&artifact.provider, &project_id, &package)?;
        }
        if !duplicate {
            self.candidates
                .entry(package.clone())
                .or_default()
                .push(candidate);
        }
        self.resolved.entry(candidate_id).or_insert(artifact);
        Ok(package)
    }

    fn record_source_alias(
        &mut self,
        provider: &str,
        alias: &str,
        package: &str,
    ) -> Result<(), crate::error::OrbitError> {
        let key = (provider.to_string(), alias.to_string());
        if let Some(existing) = self.source_packages.get(&key)
            && existing != package
        {
            return Err(crate::error::OrbitError::Other(anyhow::anyhow!(
                "{provider} locator '{alias}' returned JARs declaring different mod IDs: \
                 '{existing}' and '{package}'"
            )));
        }
        self.source_packages.insert(key, package.to_string());
        Ok(())
    }

    pub(crate) fn package_for_locator(
        &self,
        locator: &str,
    ) -> Result<Option<String>, crate::error::OrbitError> {
        let mut packages = self
            .source_packages
            .iter()
            .filter_map(|((_, alias), package)| (alias == locator).then_some(package))
            .collect::<std::collections::BTreeSet<_>>();
        if packages.len() > 1 {
            return Err(crate::error::OrbitError::Other(anyhow::anyhow!(
                "provider locator '{locator}' is ambiguous after JAR inspection: {}",
                packages
                    .iter()
                    .map(|package| package.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
        }
        Ok(packages.pop_first().cloned())
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
    pub kind: PackageChangeKind,
}

#[derive(Debug, Clone, Default)]
pub struct ResolutionPortfolio {
    pub alternatives: Vec<ResolutionReport>,
}

pub type ResolutionSelector = Box<dyn FnOnce(&[ResolutionReport]) -> usize + Send>;
