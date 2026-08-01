//! Persistent candidate repository split by exact Minecraft/Loader scope.
//!
//! Each scope owns two physical SQLite databases:
//! - `remote.sqlite` contains provider project cursors and artifact locators;
//! - `jars.sqlite` contains only content hashes and loader-declared JAR metadata.
//!
//! Provider project IDs never enter the JAR database or resolver graph.

use std::collections::{BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{Connection, OptionalExtension, params};

use crate::error::OrbitError;
use crate::jar::InspectedJar;
use crate::jar_cache::JarCache;
use crate::loader::LoaderKind;
use crate::manifest::PackageRemote;
use crate::providers::RemoteArtifact;
use crate::resolver::types::CandidateCatalog;

const SCHEMA_VERSION: i64 = 1;

#[derive(Debug, Clone)]
pub struct VersionRepository {
    root: PathBuf,
}

/// Per-command access to candidate bytes and persistent metadata.
///
/// This is only a lightweight parameter object. The global content-addressed
/// JAR cache and the Minecraft/Loader-scoped version repository remain
/// independent stores with separate lifecycles.
#[derive(Debug, Clone, Copy)]
pub struct CandidateStorage<'a> {
    jar_cache: &'a JarCache,
    version_repository: &'a VersionRepository,
}

impl<'a> CandidateStorage<'a> {
    pub fn new(jar_cache: &'a JarCache, version_repository: &'a VersionRepository) -> Self {
        Self {
            jar_cache,
            version_repository,
        }
    }

    pub fn jar_cache(self) -> &'a JarCache {
        self.jar_cache
    }

    pub fn version_repository(self) -> &'a VersionRepository {
        self.version_repository
    }
}

#[derive(Debug, Clone)]
pub(crate) struct RepositoryScope {
    directory: PathBuf,
}

#[derive(Debug, Clone)]
pub(crate) struct StoredRemoteArtifact {
    pub artifact: RemoteArtifact,
    pub sha512: String,
}

impl VersionRepository {
    pub fn open(root: PathBuf) -> Result<Self, OrbitError> {
        std::fs::create_dir_all(&root)?;
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn scope(
        &self,
        minecraft: &str,
        loader: LoaderKind,
    ) -> Result<RepositoryScope, OrbitError> {
        if minecraft.is_empty() || minecraft.trim() != minecraft {
            return Err(OrbitError::Other(anyhow::anyhow!(
                "version repository requires a non-empty exact Minecraft version without surrounding whitespace"
            )));
        }
        let directory = self
            .root
            .join(scope_component(minecraft))
            .join(loader.as_str());
        std::fs::create_dir_all(&directory)?;
        let scope = RepositoryScope { directory };
        scope.initialize()?;
        Ok(scope)
    }
}

impl RepositoryScope {
    fn remote_path(&self) -> PathBuf {
        self.directory.join("remote.sqlite")
    }

    fn jars_path(&self) -> PathBuf {
        self.directory.join("jars.sqlite")
    }

    fn initialize(&self) -> Result<(), OrbitError> {
        let remote = open_database(&self.remote_path())?;
        initialize_database(
            &remote,
            "CREATE TABLE IF NOT EXISTS projects (
                provider TEXT NOT NULL,
                project_id TEXT NOT NULL,
                marker TEXT NOT NULL,
                checked_at INTEGER NOT NULL,
                PRIMARY KEY (provider, project_id)
             );
             CREATE TABLE IF NOT EXISTS artifacts (
                provider TEXT NOT NULL,
                project_id TEXT NOT NULL,
                artifact_key TEXT NOT NULL,
                sha512 TEXT NOT NULL,
                artifact_json TEXT NOT NULL,
                PRIMARY KEY (provider, project_id, artifact_key),
                FOREIGN KEY (provider, project_id)
                    REFERENCES projects(provider, project_id) ON DELETE CASCADE
             );
             CREATE INDEX IF NOT EXISTS artifacts_project
                ON artifacts(provider, project_id);",
        )?;
        let jars = open_database(&self.jars_path())?;
        initialize_database(
            &jars,
            "CREATE TABLE IF NOT EXISTS jars (
                sha512 TEXT PRIMARY KEY,
                sha1 TEXT NOT NULL,
                sha256 TEXT NOT NULL,
                mod_id TEXT NOT NULL,
                version TEXT NOT NULL,
                metadata_json TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS jars_sha1 ON jars(sha1);
             CREATE INDEX IF NOT EXISTS jars_mod_id ON jars(mod_id);",
        )?;
        Ok(())
    }

    pub(crate) fn project_marker(
        &self,
        provider: &str,
        project_id: &str,
    ) -> Result<Option<String>, OrbitError> {
        let connection = open_database(&self.remote_path())?;
        connection
            .query_row(
                "SELECT marker FROM projects WHERE provider = ?1 AND project_id = ?2",
                params![provider, project_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(sql_error)
    }

    pub(crate) fn project_artifacts(
        &self,
        provider: &str,
        project_id: &str,
    ) -> Result<Vec<StoredRemoteArtifact>, OrbitError> {
        let connection = open_database(&self.remote_path())?;
        let mut statement = connection
            .prepare(
                "SELECT sha512, artifact_json FROM artifacts
                 WHERE provider = ?1 AND project_id = ?2 ORDER BY artifact_key",
            )
            .map_err(sql_error)?;
        let rows = statement
            .query_map(params![provider, project_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(sql_error)?;
        let mut artifacts = Vec::new();
        for row in rows {
            let (sha512, json) = row.map_err(sql_error)?;
            artifacts.push(StoredRemoteArtifact {
                artifact: serde_json::from_str(&json).map_err(|error| {
                    OrbitError::Other(anyhow::anyhow!(
                        "remote repository contains invalid artifact metadata: {error}"
                    ))
                })?,
                sha512,
            });
        }
        Ok(artifacts)
    }

    pub(crate) fn replace_project(
        &self,
        provider: &str,
        project_id: &str,
        marker: &str,
        artifacts: &[StoredRemoteArtifact],
    ) -> Result<(), OrbitError> {
        let mut connection = open_database(&self.remote_path())?;
        let transaction = connection.transaction().map_err(sql_error)?;
        transaction
            .execute(
                "INSERT INTO projects(provider, project_id, marker, checked_at)
                 VALUES(?1, ?2, ?3, unixepoch())
                 ON CONFLICT(provider, project_id) DO UPDATE SET
                    marker = excluded.marker,
                    checked_at = excluded.checked_at",
                params![provider, project_id, marker],
            )
            .map_err(sql_error)?;
        transaction
            .execute(
                "DELETE FROM artifacts WHERE provider = ?1 AND project_id = ?2",
                params![provider, project_id],
            )
            .map_err(sql_error)?;
        for stored in artifacts {
            let json = serde_json::to_string(&stored.artifact).map_err(|error| {
                OrbitError::Other(anyhow::anyhow!(
                    "failed to serialize remote artifact metadata: {error}"
                ))
            })?;
            transaction
                .execute(
                    "INSERT INTO artifacts(
                        provider, project_id, artifact_key, sha512, artifact_json
                     ) VALUES(?1, ?2, ?3, ?4, ?5)",
                    params![
                        provider,
                        project_id,
                        artifact_key(&stored.artifact),
                        stored.sha512,
                        json
                    ],
                )
                .map_err(sql_error)?;
        }
        transaction.commit().map_err(sql_error)
    }

    pub(crate) fn find_jar(
        &self,
        sha512: &str,
        sha1: &str,
    ) -> Result<Option<InspectedJar>, OrbitError> {
        let connection = open_database(&self.jars_path())?;
        let row = if !sha512.is_empty() {
            connection
                .query_row(
                    "SELECT sha1, sha256, sha512, metadata_json FROM jars WHERE sha512 = ?1",
                    params![sha512.to_ascii_lowercase()],
                    jar_row,
                )
                .optional()
                .map_err(sql_error)?
        } else if !sha1.is_empty() {
            connection
                .query_row(
                    "SELECT sha1, sha256, sha512, metadata_json FROM jars WHERE sha1 = ?1",
                    params![sha1.to_ascii_lowercase()],
                    jar_row,
                )
                .optional()
                .map_err(sql_error)?
        } else {
            None
        };
        row.map(decode_jar).transpose()
    }

    pub(crate) fn jar_by_sha512(&self, sha512: &str) -> Result<InspectedJar, OrbitError> {
        self.find_jar(sha512, "")?.ok_or_else(|| {
            OrbitError::Other(anyhow::anyhow!(
                "JAR analysis repository is missing content {sha512} referenced by the remote repository"
            ))
        })
    }

    pub(crate) fn store_jar(&self, inspected: &InspectedJar) -> Result<(), OrbitError> {
        let connection = open_database(&self.jars_path())?;
        let metadata_json = serde_json::to_string(&inspected.metadata).map_err(|error| {
            OrbitError::Other(anyhow::anyhow!("failed to serialize JAR metadata: {error}"))
        })?;
        connection
            .execute(
                "INSERT INTO jars(sha512, sha1, sha256, mod_id, version, metadata_json)
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(sha512) DO UPDATE SET
                    sha1 = excluded.sha1,
                    sha256 = excluded.sha256,
                    mod_id = excluded.mod_id,
                    version = excluded.version,
                    metadata_json = excluded.metadata_json",
                params![
                    inspected.sha512.to_ascii_lowercase(),
                    inspected.sha1.to_ascii_lowercase(),
                    inspected.sha256.to_ascii_lowercase(),
                    &inspected.metadata.mod_id,
                    &inspected.metadata.version,
                    metadata_json
                ],
            )
            .map_err(sql_error)?;
        Ok(())
    }

    pub(crate) fn build_catalog(
        &self,
        seed_remotes: &[(PackageRemote, bool)],
    ) -> Result<CandidateCatalog, OrbitError> {
        let mut queue: VecDeque<_> = seed_remotes.iter().cloned().collect();
        let requested: BTreeSet<_> = seed_remotes
            .iter()
            .filter(|(_, requested)| *requested)
            .map(|(remote, _)| remote.clone())
            .collect();
        let mut seen = BTreeSet::new();
        let mut catalog = CandidateCatalog::default();
        while let Some((remote, direct_requested)) = queue.pop_front() {
            let Some((provider, project_id)) = remote_project(&remote) else {
                continue;
            };
            if !seen.insert((provider.to_string(), project_id.clone())) {
                continue;
            }
            let artifacts = self.project_artifacts(provider, &project_id)?;
            for stored in artifacts {
                let inspected = self.jar_by_sha512(&stored.sha512)?;
                for related in &stored.artifact.related_projects {
                    if let Some(related_id) = related.project_id.as_ref() {
                        queue.push_back((package_remote(provider, related_id)?, false));
                    }
                }
                catalog.record(
                    inspected.metadata.clone(),
                    stored.artifact,
                    &inspected,
                    direct_requested || requested.contains(&remote),
                )?;
            }
        }
        Ok(catalog)
    }
}

fn open_database(path: &Path) -> Result<Connection, OrbitError> {
    let connection = Connection::open(path).map_err(sql_error)?;
    connection
        .busy_timeout(Duration::from_secs(30))
        .map_err(sql_error)?;
    connection
        .execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;")
        .map_err(sql_error)?;
    Ok(connection)
}

fn initialize_database(connection: &Connection, schema: &str) -> Result<(), OrbitError> {
    let version: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(sql_error)?;
    match version {
        0 => {
            connection.execute_batch(schema).map_err(sql_error)?;
            connection
                .execute_batch(&format!("PRAGMA user_version = {SCHEMA_VERSION}"))
                .map_err(sql_error)?;
        }
        SCHEMA_VERSION => {}
        other => {
            return Err(OrbitError::Other(anyhow::anyhow!(
                "unsupported version repository schema {other}; expected {SCHEMA_VERSION}"
            )));
        }
    }
    Ok(())
}

fn jar_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<(String, String, String, String)> {
    Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
}

fn decode_jar(row: (String, String, String, String)) -> Result<InspectedJar, OrbitError> {
    let (sha1, sha256, sha512, json) = row;
    Ok(InspectedJar {
        metadata: serde_json::from_str(&json).map_err(|error| {
            OrbitError::Other(anyhow::anyhow!(
                "JAR analysis repository contains invalid metadata: {error}"
            ))
        })?,
        sha1,
        sha256,
        sha512,
    })
}

fn artifact_key(artifact: &RemoteArtifact) -> String {
    format!(
        "{}:{}:{}:{}",
        artifact.provider,
        artifact.version_id().unwrap_or_default(),
        artifact.download_url,
        artifact.filename
    )
}

pub(crate) fn remote_project(remote: &PackageRemote) -> Option<(&'static str, String)> {
    match remote {
        PackageRemote::Modrinth { project_id } => Some(("modrinth", project_id.clone())),
        PackageRemote::Curseforge { project_id } => Some(("curseforge", project_id.to_string())),
        PackageRemote::File { .. } => None,
    }
}

pub(crate) fn package_remote(
    provider: &str,
    project_id: &str,
) -> Result<PackageRemote, OrbitError> {
    match provider {
        "modrinth" => Ok(PackageRemote::Modrinth {
            project_id: project_id.to_string(),
        }),
        "curseforge" => Ok(PackageRemote::Curseforge {
            project_id: project_id.parse().map_err(|_| {
                OrbitError::Other(anyhow::anyhow!(
                    "invalid CurseForge project ID '{project_id}' in version repository"
                ))
            })?,
        }),
        other => Err(OrbitError::Other(anyhow::anyhow!(
            "unsupported provider '{other}' in version repository"
        ))),
    }
}

fn scope_component(value: &str) -> String {
    hex::encode(value.as_bytes())
}

fn sql_error(error: rusqlite::Error) -> OrbitError {
    OrbitError::Other(anyhow::anyhow!(
        "version repository database error: {error}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::{Environment, ModLoadCondition};
    use crate::providers::ModrinthResolvedInfo;

    fn inspected(version: &str, hash: &str) -> InspectedJar {
        InspectedJar {
            metadata: crate::jar::JarModMetadata {
                mod_id: "example".to_string(),
                name: "Example".to_string(),
                version: version.to_string(),
                environment: Environment::Both,
                dependencies: Vec::new(),
                provides: Vec::new(),
                language_loader: None,
                load_condition: ModLoadCondition::Always,
                origin: crate::jar::JarModOrigin::Root,
                embedded_jars: Vec::new(),
                embedded_artifacts: Vec::new(),
                bundled_mods: Vec::new(),
            },
            sha1: format!("sha1-{hash}"),
            sha256: format!("sha256-{hash}"),
            sha512: hash.to_string(),
        }
    }

    fn artifact(project: &str, version: &str) -> RemoteArtifact {
        RemoteArtifact {
            sha1: String::new(),
            sha512: version.to_string(),
            slug: "ignored".to_string(),
            provider: "modrinth".to_string(),
            modrinth: Some(ModrinthResolvedInfo {
                project_id: project.to_string(),
                version_id: version.to_string(),
            }),
            curseforge: None,
            download_url: format!("https://example.invalid/{version}.jar"),
            filename: format!("{version}.jar"),
            related_projects: Vec::new(),
        }
    }

    #[test]
    fn scopes_use_two_physical_databases_and_keep_project_ids_out_of_jar_db() {
        let directory = tempfile::tempdir().unwrap();
        let repository = VersionRepository::open(directory.path().to_path_buf()).unwrap();
        let scope = repository.scope("1.21.1", LoaderKind::Fabric).unwrap();
        let jar = inspected("1.0.0", "content-a");
        scope.store_jar(&jar).unwrap();
        scope
            .replace_project(
                "modrinth",
                "project-a",
                "2026-01-01T00:00:00Z",
                &[StoredRemoteArtifact {
                    artifact: artifact("project-a", "version-a"),
                    sha512: jar.sha512.clone(),
                }],
            )
            .unwrap();

        assert!(scope.remote_path().is_file());
        assert!(scope.jars_path().is_file());
        let jars = std::fs::read(scope.jars_path()).unwrap();
        assert!(!String::from_utf8_lossy(&jars).contains("project-a"));
        let catalog = scope
            .build_catalog(&[(
                PackageRemote::Modrinth {
                    project_id: "project-a".to_string(),
                },
                true,
            )])
            .unwrap();
        assert_eq!(catalog.candidates["example"][0].jar_version, "1.0.0");
    }

    #[test]
    fn minecraft_versions_and_loaders_are_physically_isolated() {
        let directory = tempfile::tempdir().unwrap();
        let repository = VersionRepository::open(directory.path().to_path_buf()).unwrap();
        let fabric = repository.scope("1.21.1", LoaderKind::Fabric).unwrap();
        let forge = repository.scope("1.21.1", LoaderKind::Forge).unwrap();
        let newer = repository.scope("1.21.2", LoaderKind::Fabric).unwrap();
        let uppercase = repository.scope("Release", LoaderKind::Fabric).unwrap();
        let lowercase = repository.scope("release", LoaderKind::Fabric).unwrap();
        let punctuation = repository.scope("a/b", LoaderKind::Fabric).unwrap();
        let literal = repository.scope("a_2Fb", LoaderKind::Fabric).unwrap();
        assert_ne!(fabric.remote_path(), forge.remote_path());
        assert_ne!(fabric.remote_path(), newer.remote_path());
        assert_ne!(uppercase.remote_path(), lowercase.remote_path());
        assert_ne!(punctuation.remote_path(), literal.remote_path());
    }
}
