//! In-memory [`pubgrub::DependencyProvider`] used by Orbit's orchestration layer.

use pubgrub::Ranges;
use pubgrub::{Dependencies, DependencyProvider, IncompatibilityConstraint};
use std::collections::HashMap;

use crate::resolver::types::{SolverPackage, SolverVersion};

type PackageVersionKey = (SolverPackage, SolverVersion);
type PackageDependencies = Vec<(SolverPackage, Ranges<SolverVersion>)>;
pub(crate) type PackageIncompatibilities =
    Vec<IncompatibilityConstraint<SolverPackage, Ranges<SolverVersion>, String>>;

#[derive(Debug)]
pub(crate) enum ProviderError {
    MissingVersions(SolverPackage),
    MissingDependencies(SolverPackage, SolverVersion),
}

impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingVersions(package) => write!(f, "missing versions for {package}"),
            Self::MissingDependencies(package, version) => {
                write!(f, "missing dependencies for {package} {version}")
            }
        }
    }
}

impl std::error::Error for ProviderError {}

/// Immutable in-memory view consumed by PubGrub after graph construction.
#[derive(Default)]
pub(crate) struct OrbitDependencyProvider {
    /// package → known available versions. Selection is ordered semantically, not by insertion.
    pub(crate) versions: HashMap<SolverPackage, Vec<SolverVersion>>,
    /// Exact package version to its dependency ranges.
    pub(crate) dependencies: HashMap<PackageVersionKey, PackageDependencies>,
    pub(crate) incompatibilities: HashMap<PackageVersionKey, PackageIncompatibilities>,
}

impl OrbitDependencyProvider {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn add_package_versions(
        &mut self,
        pkg: SolverPackage,
        versions: Vec<SolverVersion>,
    ) {
        self.versions.insert(pkg, versions);
    }

    pub(crate) fn add_package_deps(
        &mut self,
        pkg: SolverPackage,
        version: SolverVersion,
        deps: Vec<(SolverPackage, Ranges<SolverVersion>)>,
    ) {
        self.dependencies.insert((pkg, version), deps);
    }

    pub(crate) fn add_package_incompatibilities(
        &mut self,
        pkg: SolverPackage,
        version: SolverVersion,
        incompatibilities: PackageIncompatibilities,
    ) {
        self.incompatibilities
            .insert((pkg, version), incompatibilities);
    }

    pub(crate) fn extend_package_incompatibilities(
        &mut self,
        pkg: SolverPackage,
        version: SolverVersion,
        incompatibilities: PackageIncompatibilities,
    ) {
        self.incompatibilities
            .entry((pkg, version))
            .or_default()
            .extend(incompatibilities);
    }
}

impl DependencyProvider for OrbitDependencyProvider {
    type P = SolverPackage;
    type V = SolverVersion;
    type VS = Ranges<SolverVersion>;
    type Priority = usize;
    type M = String;
    type Err = ProviderError;

    fn prioritize(
        &self,
        _package: &Self::P,
        range: &Self::VS,
        _package_conflicts_counts: &pubgrub::PackageResolutionStatistics,
    ) -> Self::Priority {
        // Prefer constrained packages over packages that still allow every version.
        if range == &Ranges::full() {
            return 0;
        }
        range.bounding_range().map(|_| 1).unwrap_or(0)
    }

    fn choose_version(
        &self,
        package: &Self::P,
        range: &Self::VS,
    ) -> Result<Option<Self::V>, Self::Err> {
        match self.versions.get(package) {
            Some(versions) => Ok(versions
                .iter()
                .filter(|version| range.contains(version))
                .max()
                .cloned()),
            None => Err(ProviderError::MissingVersions(package.clone())),
        }
    }

    fn get_dependencies(
        &self,
        package: &Self::P,
        version: &Self::V,
    ) -> Result<Dependencies<Self::P, Self::VS, Self::M>, Self::Err> {
        match self.dependencies.get(&(package.clone(), version.clone())) {
            Some(deps) => Ok(Dependencies::Available(deps.iter().cloned().collect())),
            None => Err(ProviderError::MissingDependencies(
                package.clone(),
                version.clone(),
            )),
        }
    }

    fn get_incompatibilities(
        &self,
        package: &Self::P,
        version: &Self::V,
    ) -> Result<PackageIncompatibilities, Self::Err> {
        Ok(self
            .incompatibilities
            .get(&(package.clone(), version.clone()))
            .cloned()
            .unwrap_or_default())
    }
}
