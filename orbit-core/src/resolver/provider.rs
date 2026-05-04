use pubgrub::solver::{DependencyProvider, Dependencies};
use pubgrub::range::Range;
use std::collections::HashMap;
use std::error::Error;

use crate::resolver::types::PackageId;
use crate::versions::Version;
use crate::resolver::FetchRetryError;

/// PubGrub 的数据源——一个只读的内存视图
#[derive(Default)]
pub struct OrbitDependencyProvider {
    /// package → 已知可用版本列表（从新到旧排序）
    pub versions: HashMap<PackageId, Vec<Version>>,
    /// (package, version) → 前置依赖列表
    pub dependencies: HashMap<(PackageId, Version), Vec<(PackageId, Range<Version>)>>,
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
        deps: Vec<(PackageId, Range<Version>)>,
    ) {
        self.dependencies.insert((pkg, version), deps);
    }
}

impl DependencyProvider<PackageId, Version> for OrbitDependencyProvider {
    fn choose_package_version<T: std::borrow::Borrow<PackageId>, U: std::borrow::Borrow<Range<Version>>>(
        &self,
        potential_packages: impl Iterator<Item = (T, U)>,
    ) -> Result<(T, Option<Version>), Box<dyn Error>> {
        for (pkg, range) in potential_packages {
            if let Some(versions) = self.versions.get(pkg.borrow()) {
                for v in versions {
                    if range.borrow().contains(v) {
                        return Ok((pkg, Some(v.clone())));
                    }
                }
                return Ok((pkg, None));
            } else {
                return Err(Box::new(FetchRetryError::MissingVersions(pkg.borrow().clone())));
            }
        }
        Err("Empty potential packages".into())
    }

    fn get_dependencies(
        &self,
        package: &PackageId,
        version: &Version,
    ) -> Result<Dependencies<PackageId, Version>, Box<dyn Error>> {
        match self.dependencies.get(&(package.clone(), version.clone())) {
            Some(deps) => {
                Ok(Dependencies::Known(deps.clone().into_iter().collect()))
            }
            None => Err(Box::new(FetchRetryError::MissingDependencies(package.clone(), version.clone()))),
        }
    }
}
