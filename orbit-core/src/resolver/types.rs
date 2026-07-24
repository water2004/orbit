use std::collections::{BTreeMap, HashMap};
use std::ops::Bound;

use crate::metadata::{
    DependencyExpression, EmbeddedArtifact, Environment, LanguageLoaderRequirement, ProvidedMod,
};
use crate::providers::RemoteArtifact;
use crate::versions::Version;
use pubgrub::Ranges;

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum SolverVersion {
    Domain(Version),
    ProviderChoice(u32),
}

impl SolverVersion {
    pub(crate) fn domain(&self) -> Option<&Version> {
        match self {
            Self::Domain(version) => Some(version),
            Self::ProviderChoice(_) => None,
        }
    }
}

impl From<Version> for SolverVersion {
    fn from(version: Version) -> Self {
        Self::Domain(version)
    }
}

impl std::fmt::Display for SolverVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Domain(version) => version.fmt(f),
            Self::ProviderChoice(choice) => write!(f, "provider choice {choice}"),
        }
    }
}

pub(crate) fn solver_range(range: Ranges<Version>) -> Ranges<SolverVersion> {
    range
        .into_iter()
        .map(|(start, end)| (map_bound(start), map_bound(end)))
        .collect()
}

fn map_bound(bound: Bound<Version>) -> Bound<SolverVersion> {
    match bound {
        Bound::Included(version) => Bound::Included(version.into()),
        Bound::Excluded(version) => Bound::Excluded(version.into()),
        Bound::Unbounded => Bound::Unbounded,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum LogicalPackage {
    Capability(String),
    EmbeddedArtifact(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum SolverPackage {
    Root,
    Mod(String),
    Bundled {
        owner: String,
        owner_version: Version,
        path: Vec<usize>,
        mod_id: String,
    },
    Logical(LogicalPackage),
    ProviderChoice {
        logical: LogicalPackage,
        logical_version: Version,
    },
    Platform(String),
}

impl SolverPackage {
    pub(crate) fn top_level(package: impl Into<String>) -> Self {
        Self::Mod(package.into())
    }

    pub(crate) fn logical(package: impl Into<String>) -> Self {
        let package = package.into();
        if crate::resolver::graph::is_platform_package(&package) {
            Self::Platform(package)
        } else {
            Self::Logical(LogicalPackage::Capability(package))
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
            Self::Mod(mod_id) | Self::Bundled { mod_id, .. } | Self::Platform(mod_id) => mod_id,
            Self::Logical(LogicalPackage::Capability(mod_id))
            | Self::Logical(LogicalPackage::EmbeddedArtifact(mod_id))
            | Self::ProviderChoice {
                logical:
                    LogicalPackage::Capability(mod_id) | LogicalPackage::EmbeddedArtifact(mod_id),
                ..
            } => mod_id,
        }
    }
}

impl std::fmt::Display for SolverPackage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Root => write!(f, "the project"),
            Self::Mod(mod_id) => write!(f, "{mod_id}"),
            Self::Bundled {
                owner,
                owner_version,
                path,
                mod_id,
            } => write!(
                f,
                "{mod_id} bundled in {owner} {owner_version} at {}",
                path.iter()
                    .map(usize::to_string)
                    .collect::<Vec<_>>()
                    .join(".")
            ),
            Self::Logical(LogicalPackage::Capability(id)) => write!(f, "{id} capability"),
            Self::Logical(LogicalPackage::EmbeddedArtifact(id)) => {
                write!(f, "{id} embedded artifact")
            }
            Self::ProviderChoice {
                logical,
                logical_version,
            } => {
                let kind = match logical {
                    LogicalPackage::Capability(_) => "capability",
                    LogicalPackage::EmbeddedArtifact(_) => "embedded artifact",
                };
                write!(f, "{} {logical_version} {kind} provider", self.user_label())
            }
            Self::Platform(id) => write!(f, "{id}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateVersion {
    pub jar_version: String,
    pub dependencies: Vec<DependencyExpression>,
    pub environment: Environment,
    pub provides: Vec<ProvidedMod>,
    pub language_loader: Option<LanguageLoaderRequirement>,
    pub embedded_artifacts: Vec<EmbeddedArtifact>,
    pub bundled: Vec<BundledCandidate>,
}

pub type ResolvedCandidateKey = (String, String);
pub type ResolvedCandidates = HashMap<ResolvedCandidateKey, RemoteArtifact>;

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
    pub environment: Environment,
    pub dependencies: Vec<DependencyExpression>,
    pub provides: Vec<ProvidedMod>,
    pub language_loader: Option<LanguageLoaderRequirement>,
    pub embedded_artifacts: Vec<EmbeddedArtifact>,
    pub bundled: Vec<BundledCandidate>,
}

impl CandidateVersion {
    pub fn from_jar_metadata(metadata: crate::jar::JarModMetadata) -> Self {
        Self {
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
        let candidate = CandidateVersion::from_jar_metadata(metadata);
        let identity = (package.clone(), candidate.jar_version.clone());
        let existing = self.candidates.get(&package).and_then(|versions| {
            versions
                .iter()
                .find(|existing| existing.jar_version == candidate.jar_version)
        });
        if existing.is_some_and(|existing| existing != &candidate) {
            return Err(crate::error::OrbitError::Other(anyhow::anyhow!(
                "multiple JARs declare identity '{} {}' with different metadata",
                identity.0,
                identity.1
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
        self.resolved.entry(identity).or_insert(artifact);
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
    pub upgrades: BTreeMap<String, String>,
    pub diagnostics: Vec<CandidateDiagnostic>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ResolutionPortfolio {
    pub alternatives: Vec<ResolutionReport>,
}

pub type ResolutionSelector = Box<dyn FnOnce(&[ResolutionReport]) -> usize + Send>;
