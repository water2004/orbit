//! Forge-family archive adapter.

use std::collections::HashMap;
use std::io::{Read, Seek};

use serde::Deserialize;
use zip::ZipArchive;

use super::JarModMetadata;
use crate::error::OrbitError;
use crate::metadata::{EmbeddedArtifact, LoaderKind};

pub fn try_read<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    loader: LoaderKind,
) -> Result<Option<JarModMetadata>, OrbitError> {
    let targets: &[&str] = match loader {
        LoaderKind::Forge => &["META-INF/mods.toml"],
        LoaderKind::NeoForge => &["META-INF/neoforge.mods.toml", "META-INF/mods.toml"],
        _ => {
            return Err(OrbitError::Other(anyhow::anyhow!(
                "Forge archive adapter received incompatible loader '{}'",
                loader.as_str()
            )));
        }
    };
    let Some((source_name, content)) = super::read_metadata_entry(archive, targets)? else {
        return Ok(None);
    };

    let mut file = crate::metadata::forge::parse_for_loader(&content, loader, &source_name)?;
    substitute_file_properties(&mut file, archive)?;
    let embedded_artifacts = read_jarjar_metadata(archive)?;
    for artifact in &embedded_artifacts {
        archive.by_name(&artifact.path).map_err(|_| {
            OrbitError::Other(anyhow::anyhow!(
                "Jar-in-Jar artifact {} points to missing entry '{}'",
                artifact.id,
                artifact.path
            ))
        })?;
    }
    file.embedded_jars = embedded_artifacts
        .iter()
        .map(|artifact| artifact.path.clone())
        .collect();
    let mut metadata = super::from_mod_file(file)?;
    metadata.embedded_artifacts = embedded_artifacts;
    Ok(Some(metadata))
}

fn substitute_file_properties<R: Read + Seek>(
    file: &mut crate::metadata::ModFileMetadata,
    archive: &mut ZipArchive<R>,
) -> Result<(), OrbitError> {
    let mut properties: HashMap<String, String> =
        file.substitution_properties.clone().into_iter().collect();
    if let Some(version) = implementation_version(archive)? {
        properties.insert("jarVersion".to_string(), version);
    }

    for metadata in &mut file.mods {
        metadata.version = substitute(&metadata.version, &properties)?;
        validate_mod_version(&metadata.version)?;
        metadata.name = substitute(&metadata.name, &properties)?;
        metadata.description = substitute(&metadata.description, &properties)?;
        for author in &mut metadata.authors {
            *author = substitute(author, &properties)?;
        }
        for dependency in &mut metadata.dependencies {
            substitute_dependency(dependency, &properties)?;
        }
    }
    if let Some(language_loader) = &mut file.language_loader {
        language_loader.requirement = substitute(&language_loader.requirement, &properties)?;
    }
    if let Some(license) = &mut file.license {
        *license = substitute(license, &properties)?;
    }
    Ok(())
}

fn validate_mod_version(version: &str) -> Result<(), OrbitError> {
    if version.as_bytes().first().is_some_and(u8::is_ascii_digit) {
        return Ok(());
    }
    Err(OrbitError::Other(anyhow::anyhow!(
        "illegal Forge-family mod version '{version}': Loader versions must start with a digit"
    )))
}

fn substitute_dependency(
    dependency: &mut crate::metadata::DependencyExpression,
    properties: &HashMap<String, String>,
) -> Result<(), OrbitError> {
    match dependency {
        crate::metadata::DependencyExpression::Only(dependency) => {
            dependency.requirement = substitute(&dependency.requirement, properties)?;
            if let Some(reason) = &mut dependency.reason {
                *reason = substitute(reason, properties)?;
            }
            if let Some(unless) = &mut dependency.unless {
                substitute_dependency(unless, properties)?;
            }
        }
        crate::metadata::DependencyExpression::Any(dependencies)
        | crate::metadata::DependencyExpression::All(dependencies) => {
            for dependency in dependencies {
                substitute_dependency(dependency, properties)?;
            }
        }
    }
    Ok(())
}

fn substitute(template: &str, properties: &HashMap<String, String>) -> Result<String, OrbitError> {
    let mut output = template.to_string();
    for (key, value) in properties {
        output = output.replace(&format!("${{file.{key}}}"), value);
    }
    if output.contains("${file.") {
        return Err(OrbitError::Other(anyhow::anyhow!(
            "unresolved Forge file property in mod version '{template}'"
        )));
    }
    Ok(output)
}

fn implementation_version<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
) -> Result<Option<String>, OrbitError> {
    let Some((_, manifest)) = super::read_metadata_entry(archive, &["META-INF/MANIFEST.MF"])?
    else {
        return Ok(None);
    };
    let unfolded = unfold_manifest(&manifest);
    Ok(unfolded.lines().find_map(|line| {
        line.split_once(':').and_then(|(key, value)| {
            key.trim()
                .eq_ignore_ascii_case("Implementation-Version")
                .then(|| value.trim().to_string())
        })
    }))
}

fn unfold_manifest(manifest: &str) -> String {
    let mut output = String::new();
    for line in manifest.replace("\r\n", "\n").lines() {
        if let Some(continuation) = line.strip_prefix(' ') {
            output.push_str(continuation);
        } else {
            if !output.is_empty() {
                output.push('\n');
            }
            output.push_str(line);
        }
    }
    output
}

#[derive(Deserialize)]
struct JarJarMetadata {
    jars: Vec<JarJarEntry>,
}

#[derive(Deserialize)]
struct JarJarEntry {
    identifier: JarIdentifier,
    version: JarVersion,
    path: String,
    #[serde(default, rename = "isObfuscated")]
    is_obfuscated: bool,
}

#[derive(Deserialize)]
struct JarIdentifier {
    group: String,
    artifact: String,
}

#[derive(Deserialize)]
struct JarVersion {
    range: String,
    #[serde(rename = "artifactVersion")]
    artifact_version: String,
}

fn read_jarjar_metadata<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
) -> Result<Vec<EmbeddedArtifact>, OrbitError> {
    let Some((_, content)) =
        super::read_metadata_entry(archive, &["META-INF/jarjar/metadata.json"])?
    else {
        return Ok(Vec::new());
    };
    let metadata: JarJarMetadata = orbit_loader_json::from_str(&content).map_err(|error| {
        OrbitError::Other(anyhow::anyhow!(
            "invalid META-INF/jarjar/metadata.json: {error}"
        ))
    })?;
    let mut artifacts = Vec::with_capacity(metadata.jars.len());
    for jar in metadata.jars {
        let id = format!("{}:{}", jar.identifier.group, jar.identifier.artifact);
        if jar.identifier.group.is_empty()
            || jar.identifier.artifact.is_empty()
            || jar.version.range.is_empty()
            || jar.version.artifact_version.is_empty()
            || jar.path.is_empty()
        {
            return Err(OrbitError::Other(anyhow::anyhow!(
                "Jar-in-Jar entry '{id}' has an empty required field"
            )));
        }
        let version =
            crate::versions::Version::parse(&jar.version.artifact_version, LoaderKind::Forge);
        if !crate::versions::Version::parse_constraint(&jar.version.range, LoaderKind::Forge)
            .contains(&version)
        {
            return Err(OrbitError::Other(anyhow::anyhow!(
                "Jar-in-Jar artifact {id} {} is outside its declared range {}",
                jar.version.artifact_version,
                jar.version.range
            )));
        }
        artifacts.push(EmbeddedArtifact {
            id,
            requirement: jar.version.range,
            version: jar.version.artifact_version,
            path: jar.path,
            obfuscated: jar.is_obfuscated,
        });
    }
    Ok(artifacts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unfolds_manifest_continuations() {
        assert_eq!(
            unfold_manifest("Manifest-Version: 1.0\r\nImplementation-Version: 1.2.\r\n 3\r\n"),
            "Manifest-Version: 1.0\nImplementation-Version: 1.2.3"
        );
    }

    #[test]
    fn rejects_unresolved_version_substitutions() {
        let error = substitute("${file.missing}", &HashMap::new()).unwrap_err();
        assert!(error.to_string().contains("unresolved"));
    }

    #[test]
    fn enforces_the_forge_family_leading_digit_rule() {
        validate_mod_version("1.2.3-preview").unwrap();
        let error = validate_mod_version("v1.2.3").unwrap_err();
        assert!(error.to_string().contains("must start with a digit"));
    }
}
