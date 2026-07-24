//! In-memory [`pubgrub::DependencyProvider`] used by Orbit's orchestration layer.

use pubgrub::Ranges;
use pubgrub::{Dependencies, DependencyProvider};
use std::collections::HashMap;

use crate::resolver::types::PackageId;
use crate::versions::Version;

type PackageVersionKey = (PackageId, Version);
type PackageDependencies = Vec<(PackageId, Ranges<Version>)>;

#[derive(Debug)]
pub enum ProviderError {
    MissingVersions(PackageId),
    MissingDependencies(PackageId, Version),
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

/// PubGrub 的数据源——一个只读的内存视图
#[derive(Default)]
pub struct OrbitDependencyProvider {
    /// package → 已知可用版本列表（从新到旧排序）
    pub versions: HashMap<PackageId, Vec<Version>>,
    /// (package, version) → 前置依赖列表
    pub dependencies: HashMap<PackageVersionKey, PackageDependencies>,
    /// (package, version) → 解析后的完整 Mod 数据
    pub resolved_mods: HashMap<(PackageId, Version), crate::providers::ResolvedMod>,
}

impl OrbitDependencyProvider {
    pub fn new() -> Self {
        Self::default()
    }

    /// 向缓存中添加一个包及其版本信息（由编排层在求解前调用）
    pub fn add_package_versions(&mut self, pkg: PackageId, versions: Vec<Version>) {
        self.versions.insert(pkg, versions);
    }

    /// 仅在 PubGrub 返回 UnknownDependencies 时调用，添加具体版本的依赖
    pub fn add_package_deps(
        &mut self,
        pkg: PackageId,
        version: Version,
        deps: Vec<(PackageId, Ranges<Version>)>,
    ) {
        self.dependencies.insert((pkg, version), deps);
    }
}

impl DependencyProvider for OrbitDependencyProvider {
    type P = PackageId;
    type V = Version;
    type VS = Ranges<Version>;
    type Priority = usize;
    type M = String;
    type Err = ProviderError;

    fn prioritize(
        &self,
        _package: &Self::P,
        range: &Self::VS,
        _package_conflicts_counts: &pubgrub::PackageResolutionStatistics,
    ) -> Self::Priority {
        // Lower = less specific = higher priority. Ranges with fewer segments first.
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
            Some(versions) => {
                for v in versions {
                    if range.contains(v) {
                        return Ok(Some(v.clone()));
                    }
                }
                Ok(None)
            }
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
}
