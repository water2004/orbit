//! Parsers for exact dedicated-server launch formats.
//!
//! Loader-specific installers expose different local launch specifications,
//! but the rest of Orbit consumes one model: the actual Minecraft JAR, loader
//! JAR, loader version, and runtime classpath. This module is the only place
//! that understands those installer formats.

use std::collections::{BTreeMap, HashMap};
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};

use crate::error::OrbitError;
use crate::loader::LoaderKind;
use crate::metadata::mojang::McVersion;
use crate::metadata::version_profile::MavenCoord;

use super::ServerRuntimeSpec;

const FABRIC_INSTALLER_MAIN: &str = "net.fabricmc.installer.ServerLauncher";
const FABRIC_SERVER_MAINS: &[&str] = &[
    "net.fabricmc.loader.impl.launch.server.FabricServerLauncher",
    "net.fabricmc.loader.launch.server.FabricServerLauncher",
    "net.fabricmc.loader.impl.launch.knot.KnotServer",
];
const QUILT_SERVER_MAINS: &[&str] = &["org.quiltmc.loader.impl.launch.server.QuiltServerLauncher"];
const FORGE_SHIM_MAIN: &str = "net.minecraftforge.bootstrap.shim.Main";

#[derive(Debug)]
struct Manifest {
    main_class: Option<String>,
    class_path: Vec<String>,
}

#[derive(Debug)]
struct LaunchClasspath {
    launch_jar: PathBuf,
    jars: Vec<PathBuf>,
}

pub(super) fn discover_fabric_bootstraps(
    instance_dir: &Path,
) -> Result<Vec<ServerRuntimeSpec>, OrbitError> {
    let mut candidates = Vec::new();
    for path in direct_jars(instance_dir)? {
        let Some(manifest) = read_manifest(&path)? else {
            continue;
        };
        if manifest.main_class.as_deref() != Some(FABRIC_INSTALLER_MAIN) {
            continue;
        }
        let properties = read_jar_properties(&path, "install.properties")?.ok_or_else(|| {
            other(format!(
                "Fabric server launcher '{}' has no install.properties",
                path.display()
            ))
        })?;
        let loader_version = required_property(&properties, "fabric-loader-version", &path)?;
        let minecraft_version = required_property(&properties, "game-version", &path)?;
        let runtime_dir = instance_dir.join(".fabric").join("server");
        let launch_jar = runtime_dir.join(format!(
            "fabric-loader-server-{loader_version}-minecraft-{minecraft_version}.jar"
        ));
        let outer_game_jar = runtime_dir.join(format!("{minecraft_version}-server.jar"));
        let classpath = read_launch_classpath(instance_dir, &launch_jar)?;
        let (loader_jar, actual_loader_version) =
            identify_loader_jar(&classpath.jars, LoaderKind::Fabric)?;
        if actual_loader_version != loader_version {
            return Err(other(format!(
                "Fabric server launcher '{}' selects loader {}, but '{}' declares {}",
                path.display(),
                loader_version,
                loader_jar.display(),
                actual_loader_version
            )));
        }
        let (minecraft_jar, minecraft, mut bundler_jars) =
            resolve_minecraft_game_jar(instance_dir, &outer_game_jar, Some(&minecraft_version))?;
        let mut runtime_jars = classpath.jars;
        runtime_jars.push(classpath.launch_jar);
        runtime_jars.append(&mut bundler_jars);
        normalize_runtime_jars(&mut runtime_jars);
        candidates.push(ServerRuntimeSpec {
            loader: LoaderKind::Fabric,
            loader_version,
            minecraft,
            minecraft_jar,
            loader_jar,
            runtime_jars,
            evidence: format!(
                "Fabric server launcher {}",
                display_relative(instance_dir, &path)
            ),
        });
    }
    Ok(candidates)
}

pub(super) fn discover_direct_launch_jars(
    instance_dir: &Path,
) -> Result<Vec<ServerRuntimeSpec>, OrbitError> {
    let mut candidates = Vec::new();
    for path in direct_jars(instance_dir)? {
        let Some(manifest) = read_manifest(&path)? else {
            continue;
        };
        let Some(main_class) = manifest.main_class.as_deref() else {
            continue;
        };
        let loader = if FABRIC_SERVER_MAINS.contains(&main_class) {
            LoaderKind::Fabric
        } else if QUILT_SERVER_MAINS.contains(&main_class) {
            LoaderKind::Quilt
        } else {
            continue;
        };
        if manifest.class_path.is_empty() {
            return Err(other(format!(
                "{loader} server launch JAR '{}' has no manifest Class-Path",
                path.display()
            )));
        }
        let classpath = resolve_manifest_classpath(instance_dir, &path, &manifest.class_path)?;
        let (loader_jar, loader_version) = identify_loader_jar(&classpath.jars, loader)?;
        let property_name = format!("{loader}-server-launcher.properties");
        let configured_server = read_external_properties(instance_dir, &property_name)?
            .and_then(|properties| properties.get("serverJar").cloned())
            .unwrap_or_else(|| "server.jar".to_string());
        let outer_game_jar = resolve_instance_path(
            instance_dir,
            instance_dir,
            Path::new(&configured_server),
            &format!("{loader} serverJar"),
        )?;
        let (minecraft_jar, minecraft, mut bundler_jars) =
            resolve_minecraft_game_jar(instance_dir, &outer_game_jar, None)?;
        let mut runtime_jars = classpath.jars;
        runtime_jars.push(classpath.launch_jar);
        runtime_jars.append(&mut bundler_jars);
        normalize_runtime_jars(&mut runtime_jars);
        candidates.push(ServerRuntimeSpec {
            loader,
            loader_version,
            minecraft,
            minecraft_jar,
            loader_jar,
            runtime_jars,
            evidence: format!(
                "{loader} server launch JAR {}",
                display_relative(instance_dir, &path)
            ),
        });
    }
    Ok(candidates)
}

pub(super) fn discover_forge_shims(
    instance_dir: &Path,
) -> Result<Vec<ServerRuntimeSpec>, OrbitError> {
    let mut candidates = Vec::new();
    for path in direct_jars(instance_dir)? {
        let Some(manifest) = read_manifest(&path)? else {
            continue;
        };
        if manifest.main_class.as_deref() != Some(FORGE_SHIM_MAIN) {
            continue;
        }
        let list = read_jar_utf8(&path, "bootstrap-shim.list")?.ok_or_else(|| {
            other(format!(
                "Forge bootstrap shim '{}' has no bootstrap-shim.list",
                path.display()
            ))
        })?;
        let entries = parse_forge_shim_list(instance_dir, &path, &list)?;
        let server = exactly_one_coordinate(&entries, "net.minecraftforge", "forge", "server")?;
        let universal =
            exactly_one_coordinate(&entries, "net.minecraftforge", "forge", "universal")?;
        if server.0.version != universal.0.version {
            return Err(other(format!(
                "Forge bootstrap shim '{}' mixes server {} with universal {}",
                path.display(),
                server.0.version,
                universal.0.version
            )));
        }
        let (minecraft_version, loader_version) = split_forge_version(&server.0.version)
            .ok_or_else(|| {
                other(format!(
                    "Forge bootstrap shim '{}' has invalid Forge coordinate version '{}'",
                    path.display(),
                    server.0.version
                ))
            })?;
        let minecraft = read_minecraft_metadata(instance_dir, &server.1, &minecraft_version)?;
        let mut runtime_jars = entries
            .values()
            .map(|(_, path)| path.clone())
            .collect::<Vec<_>>();
        runtime_jars.push(path.clone());
        for token in manifest.class_path {
            runtime_jars.push(resolve_manifest_token(instance_dir, &path, &token)?);
        }
        normalize_runtime_jars(&mut runtime_jars);
        candidates.push(ServerRuntimeSpec {
            loader: LoaderKind::Forge,
            loader_version,
            minecraft,
            minecraft_jar: server.1.clone(),
            loader_jar: universal.1.clone(),
            runtime_jars,
            evidence: format!(
                "Forge bootstrap shim {}",
                display_relative(instance_dir, &path)
            ),
        });
    }
    Ok(candidates)
}

pub(super) fn discover_modlauncher_argfiles(
    instance_dir: &Path,
) -> Result<Vec<ServerRuntimeSpec>, OrbitError> {
    let roots = [
        (
            LoaderKind::Forge,
            instance_dir
                .join("libraries")
                .join("net")
                .join("minecraftforge")
                .join("forge"),
        ),
        (
            LoaderKind::NeoForge,
            instance_dir
                .join("libraries")
                .join("net")
                .join("neoforged")
                .join("neoforge"),
        ),
    ];
    let mut candidates = Vec::new();
    for (loader, root) in roots {
        if !root.is_dir() {
            continue;
        }
        for entry in std::fs::read_dir(&root)? {
            let version_dir = entry?.path();
            if !version_dir.is_dir() {
                continue;
            }
            let args_path = version_dir.join(platform_argfile_name());
            if !args_path.is_file() {
                continue;
            }
            if let Some(candidate) =
                parse_modlauncher_argfile(instance_dir, loader, &version_dir, &args_path)?
            {
                candidates.push(candidate);
            }
        }
    }
    Ok(candidates)
}

fn parse_modlauncher_argfile(
    instance_dir: &Path,
    loader: LoaderKind,
    version_dir: &Path,
    args_path: &Path,
) -> Result<Option<ServerRuntimeSpec>, OrbitError> {
    let content = std::fs::read_to_string(args_path).map_err(|error| {
        other(format!(
            "cannot read server argument file '{}': {error}",
            args_path.display()
        ))
    })?;
    let tokens = tokenize_java_argfile(&content)?;
    if tokens.is_empty() {
        return Err(other(format!(
            "server argument file '{}' is empty",
            args_path.display()
        )));
    }
    // Forge's new bootstrap-shim argfile delegates to a richer, signed local
    // specification. It is parsed by discover_forge_shims instead.
    if option_value(&tokens, "-jar").is_some_and(|value| value.contains("-shim.jar")) {
        return Ok(None);
    }
    let launch_target = option_value(&tokens, "--launchTarget");
    let has_server_main = tokens
        .iter()
        .any(|token| token == "net.neoforged.fml.startup.Server");
    if !launch_target.is_some_and(|target| target.to_ascii_lowercase().contains("server"))
        && !has_server_main
    {
        return Ok(None);
    }

    let directory_version = version_dir
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            other(format!(
                "non-UTF-8 loader version path '{}'",
                version_dir.display()
            ))
        })?;
    let (minecraft_version, loader_version) = match loader {
        LoaderKind::Forge => {
            let (coordinate_mc, coordinate_loader) = split_forge_version(directory_version)
                .ok_or_else(|| {
                    other(format!(
                        "invalid Forge server coordinate directory '{}'",
                        version_dir.display()
                    ))
                })?;
            validate_optional_option(&tokens, "--fml.mcVersion", &coordinate_mc, args_path)?;
            validate_optional_option(&tokens, "--fml.forgeVersion", &coordinate_loader, args_path)?;
            (coordinate_mc, coordinate_loader)
        }
        LoaderKind::NeoForge => {
            let minecraft = required_option(&tokens, "--fml.mcVersion", args_path)?;
            let loader_version = required_option(&tokens, "--fml.neoForgeVersion", args_path)?;
            if loader_version != directory_version {
                return Err(other(format!(
                    "NeoForge argument file '{}' selects {}, but its coordinate directory is {}",
                    args_path.display(),
                    loader_version,
                    directory_version
                )));
            }
            (minecraft, loader_version)
        }
        LoaderKind::Fabric | LoaderKind::Quilt => unreachable!(),
    };

    let loader_jar = exactly_one_named_jar(
        instance_dir,
        version_dir,
        &format!("{loader}-{directory_version}-universal.jar"),
        &format!("{loader} universal"),
    )?;
    let minecraft_jar = match loader {
        LoaderKind::Forge => exactly_one_named_jar(
            instance_dir,
            version_dir,
            &format!("forge-{directory_version}-server.jar"),
            "Forge patched server",
        )?,
        LoaderKind::NeoForge => {
            let directory = instance_dir
                .join("libraries")
                .join("net")
                .join("neoforged")
                .join("minecraft-server-patched")
                .join(&loader_version);
            exactly_one_named_jar(
                instance_dir,
                &directory,
                &format!("minecraft-server-patched-{loader_version}.jar"),
                "NeoForge patched server",
            )?
        }
        LoaderKind::Fabric | LoaderKind::Quilt => unreachable!(),
    };
    let minecraft = read_minecraft_metadata(instance_dir, &minecraft_jar, &minecraft_version)?;
    let mut runtime_jars = runtime_paths_from_args(instance_dir, &tokens)?;
    runtime_jars.push(loader_jar.clone());
    runtime_jars.push(minecraft_jar.clone());
    normalize_runtime_jars(&mut runtime_jars);
    Ok(Some(ServerRuntimeSpec {
        loader,
        loader_version,
        minecraft,
        minecraft_jar,
        loader_jar,
        runtime_jars,
        evidence: format!(
            "{} server arguments {}",
            loader,
            display_relative(instance_dir, args_path)
        ),
    }))
}

fn read_launch_classpath(
    instance_dir: &Path,
    launch_jar: &Path,
) -> Result<LaunchClasspath, OrbitError> {
    let manifest = read_manifest(launch_jar)?.ok_or_else(|| {
        other(format!(
            "server launch JAR '{}' has no manifest",
            launch_jar.display()
        ))
    })?;
    if !FABRIC_SERVER_MAINS.contains(&manifest.main_class.as_deref().unwrap_or_default()) {
        return Err(other(format!(
            "Fabric generated launch JAR '{}' has unexpected Main-Class '{}'",
            launch_jar.display(),
            manifest.main_class.unwrap_or_default()
        )));
    }
    if manifest.class_path.is_empty() {
        return Err(other(format!(
            "server launch JAR '{}' has no manifest Class-Path",
            launch_jar.display()
        )));
    }
    resolve_manifest_classpath(instance_dir, launch_jar, &manifest.class_path)
}

fn resolve_manifest_classpath(
    instance_dir: &Path,
    launch_jar: &Path,
    tokens: &[String],
) -> Result<LaunchClasspath, OrbitError> {
    require_file(launch_jar, "server launch JAR")?;
    let jars = tokens
        .iter()
        .map(|token| resolve_manifest_token(instance_dir, launch_jar, token))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(LaunchClasspath {
        launch_jar: canonical_within(instance_dir, launch_jar, "server launch JAR")?,
        jars,
    })
}

fn resolve_manifest_token(
    instance_dir: &Path,
    launch_jar: &Path,
    token: &str,
) -> Result<PathBuf, OrbitError> {
    let launch_jar = launch_jar.canonicalize().map_err(|error| {
        other(format!(
            "cannot resolve server launch JAR '{}': {error}",
            launch_jar.display()
        ))
    })?;
    let base = url::Url::from_file_path(&launch_jar).map_err(|_| {
        other(format!(
            "cannot convert '{}' to a file URL",
            launch_jar.display()
        ))
    })?;
    let resolved = base.join(token).map_err(|error| {
        other(format!(
            "invalid manifest Class-Path entry '{token}' in '{}': {error}",
            launch_jar.display()
        ))
    })?;
    if resolved.scheme() != "file" || resolved.query().is_some() || resolved.fragment().is_some() {
        return Err(other(format!(
            "manifest Class-Path entry '{token}' in '{}' is not a local file",
            launch_jar.display()
        )));
    }
    let path = resolved.to_file_path().map_err(|_| {
        other(format!(
            "manifest Class-Path entry '{token}' in '{}' is not a valid file path",
            launch_jar.display()
        ))
    })?;
    canonical_within(instance_dir, &path, "manifest Class-Path JAR")
}

fn identify_loader_jar(
    classpath: &[PathBuf],
    loader: LoaderKind,
) -> Result<(PathBuf, String), OrbitError> {
    let expected_id = crate::loader::semantics(loader).canonical_package;
    let mut matches = Vec::new();
    for path in classpath {
        if let Some(metadata) = crate::jar::read_mod_metadata_if_present(path, loader)?
            && metadata.mod_id == expected_id
        {
            matches.push((path.clone(), metadata.version));
        }
    }
    match matches.as_slice() {
        [(path, version)] => Ok((path.clone(), version.clone())),
        [] => Err(other(format!(
            "{loader} server classpath contains no JAR declaring mod_id '{expected_id}'"
        ))),
        matches => Err(other(format!(
            "{loader} server classpath contains multiple loader JARs: {}",
            matches
                .iter()
                .map(|(path, version)| format!("{} ({version})", path.display()))
                .collect::<Vec<_>>()
                .join(", ")
        ))),
    }
}

fn resolve_minecraft_game_jar(
    instance_dir: &Path,
    outer_jar: &Path,
    expected_version: Option<&str>,
) -> Result<(PathBuf, McVersion, Vec<PathBuf>), OrbitError> {
    require_file(outer_jar, "Minecraft server JAR")?;
    let versions = read_jar_utf8(outer_jar, "META-INF/versions.list")?;
    let Some(versions) = versions else {
        let jar = canonical_within(instance_dir, outer_jar, "Minecraft server JAR")?;
        let version = crate::jar::read_minecraft_version(&jar)?;
        validate_minecraft_version(&version, expected_version, outer_jar)?;
        return Ok((jar, version, Vec::new()));
    };

    let version_entries = parse_bundler_list(&versions, "META-INF/versions.list", outer_jar)?;
    let matching = version_entries
        .into_iter()
        .filter(|entry| expected_version.is_none_or(|expected| entry.id == expected))
        .collect::<Vec<_>>();
    let entry = match matching.as_slice() {
        [entry] => entry,
        [] => {
            return Err(other(format!(
                "Minecraft server bundler '{}' contains no requested version{}",
                outer_jar.display(),
                expected_version
                    .map(|version| format!(" '{version}'"))
                    .unwrap_or_default()
            )));
        }
        entries => {
            return Err(other(format!(
                "Minecraft server bundler '{}' contains multiple matching versions: {}",
                outer_jar.display(),
                entries
                    .iter()
                    .map(|entry| entry.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
        }
    };
    // Mojang's bundler list already includes its version directory in the
    // third field (for example `26.1.2/server-26.1.2.jar`). It is relative to
    // `versions/`, not to `versions/<id>/`.
    let game_jar = instance_dir.join("versions").join(&entry.path);
    verify_listed_file(
        instance_dir,
        &game_jar,
        &entry.hash,
        "bundled Minecraft version",
    )?;
    let game_jar = canonical_within(instance_dir, &game_jar, "bundled Minecraft version JAR")?;
    let minecraft = crate::jar::read_minecraft_version(&game_jar)?;
    validate_minecraft_version(&minecraft, Some(&entry.id), &game_jar)?;

    let mut runtime_jars = vec![canonical_within(
        instance_dir,
        outer_jar,
        "Minecraft server bundler",
    )?];
    if let Some(libraries) = read_jar_utf8(outer_jar, "META-INF/libraries.list")? {
        for library in parse_bundler_list(&libraries, "META-INF/libraries.list", outer_jar)? {
            let path = instance_dir.join("libraries").join(&library.path);
            verify_listed_file(
                instance_dir,
                &path,
                &library.hash,
                "bundled Minecraft library",
            )?;
            runtime_jars.push(canonical_within(
                instance_dir,
                &path,
                "bundled Minecraft library",
            )?);
        }
    }
    normalize_runtime_jars(&mut runtime_jars);
    Ok((game_jar, minecraft, runtime_jars))
}

#[derive(Debug)]
struct BundlerEntry {
    hash: String,
    id: String,
    path: PathBuf,
}

fn parse_bundler_list(
    content: &str,
    entry_name: &str,
    jar: &Path,
) -> Result<Vec<BundlerEntry>, OrbitError> {
    let mut entries = Vec::new();
    for (index, line) in content.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let fields = line.split('\t').collect::<Vec<_>>();
        if fields.len() != 3 || fields.iter().any(|field| field.is_empty()) {
            return Err(other(format!(
                "invalid {entry_name} line {} in '{}'",
                index + 1,
                jar.display()
            )));
        }
        let relative = PathBuf::from(fields[2]);
        if relative.is_absolute()
            || relative
                .components()
                .any(|component| matches!(component, std::path::Component::ParentDir))
        {
            return Err(other(format!(
                "unsafe path '{}' in {entry_name} of '{}'",
                fields[2],
                jar.display()
            )));
        }
        entries.push(BundlerEntry {
            hash: fields[0].to_ascii_lowercase(),
            id: fields[1].to_string(),
            path: relative,
        });
    }
    if entries.is_empty() {
        return Err(other(format!(
            "{entry_name} in '{}' is empty",
            jar.display()
        )));
    }
    Ok(entries)
}

fn parse_forge_shim_list(
    instance_dir: &Path,
    shim: &Path,
    content: &str,
) -> Result<BTreeMap<String, (MavenCoord, PathBuf)>, OrbitError> {
    // Forge's bootstrap shim resolves every listed Maven path under the
    // installation's libraries directory. The root shim manifest, by
    // contrast, stores Class-Path entries relative to the root shim itself.
    let libraries_dir = instance_dir.join("libraries");
    let mut entries = BTreeMap::new();
    for (index, line) in content.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let fields = line.split('\t').collect::<Vec<_>>();
        if fields.len() != 3 {
            return Err(other(format!(
                "invalid bootstrap-shim.list line {} in '{}'",
                index + 1,
                shim.display()
            )));
        }
        let coordinate = MavenCoord::parse(fields[1]).ok_or_else(|| {
            other(format!(
                "invalid Maven coordinate '{}' in '{}'",
                fields[1],
                shim.display()
            ))
        })?;
        let path = resolve_instance_path(
            instance_dir,
            &libraries_dir,
            Path::new(fields[2]),
            "Forge shim library",
        )?;
        verify_listed_file(instance_dir, &path, fields[0], "Forge shim library")?;
        if entries
            .insert(fields[1].to_string(), (coordinate, path))
            .is_some()
        {
            return Err(other(format!(
                "duplicate coordinate '{}' in '{}'",
                fields[1],
                shim.display()
            )));
        }
    }
    if entries.is_empty() {
        return Err(other(format!(
            "bootstrap-shim.list in '{}' is empty",
            shim.display()
        )));
    }
    Ok(entries)
}

fn exactly_one_coordinate<'a>(
    entries: &'a BTreeMap<String, (MavenCoord, PathBuf)>,
    group: &str,
    artifact: &str,
    classifier: &str,
) -> Result<&'a (MavenCoord, PathBuf), OrbitError> {
    let matches = entries
        .values()
        .filter(|(coordinate, _)| {
            coordinate.group_id == group
                && coordinate.artifact_id == artifact
                && coordinate.classifier.as_deref() == Some(classifier)
        })
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [entry] => Ok(*entry),
        [] => Err(other(format!(
            "Forge bootstrap shim contains no {group}:{artifact}:*:{classifier} entry"
        ))),
        _ => Err(other(format!(
            "Forge bootstrap shim contains multiple {group}:{artifact}:*:{classifier} entries"
        ))),
    }
}

fn runtime_paths_from_args(
    instance_dir: &Path,
    tokens: &[String],
) -> Result<Vec<PathBuf>, OrbitError> {
    let mut raw_lists = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        if matches!(
            token.as_str(),
            "-p" | "--module-path" | "-cp" | "-classpath" | "--class-path"
        ) {
            let value = tokens
                .get(index + 1)
                .ok_or_else(|| other(format!("Java argument '{token}' has no path-list value")))?;
            raw_lists.push(value.as_str());
        } else if let Some(value) = token
            .strip_prefix("--module-path=")
            .or_else(|| token.strip_prefix("--class-path="))
            .or_else(|| token.strip_prefix("-DlegacyClassPath="))
        {
            raw_lists.push(value);
        }
    }
    let mut paths = Vec::new();
    for raw in raw_lists {
        for path in std::env::split_paths(raw) {
            paths.push(resolve_instance_path(
                instance_dir,
                instance_dir,
                &path,
                "server argument classpath JAR",
            )?);
        }
    }
    normalize_runtime_jars(&mut paths);
    Ok(paths)
}

fn tokenize_java_argfile(content: &str) -> Result<Vec<String>, OrbitError> {
    let mut tokens = Vec::new();
    let mut token = String::new();
    let mut quote = None;
    let mut escaped = false;
    let mut comment = false;
    for character in content.chars() {
        if comment {
            if character == '\n' {
                comment = false;
            }
            continue;
        }
        if escaped {
            token.push(character);
            escaped = false;
            continue;
        }
        if let Some(delimiter) = quote {
            match character {
                '\\' => escaped = true,
                character if character == delimiter => quote = None,
                _ => token.push(character),
            }
            continue;
        }
        match character {
            '\'' | '"' => quote = Some(character),
            '#' if token.is_empty() => comment = true,
            character if character.is_whitespace() => {
                if !token.is_empty() {
                    tokens.push(std::mem::take(&mut token));
                }
            }
            _ => token.push(character),
        }
    }
    if escaped || quote.is_some() {
        return Err(other("unterminated quote or escape in Java argument file"));
    }
    if !token.is_empty() {
        tokens.push(token);
    }
    Ok(tokens)
}

fn option_value<'a>(tokens: &'a [String], name: &str) -> Option<&'a str> {
    tokens.iter().enumerate().find_map(|(index, token)| {
        if token == name {
            tokens.get(index + 1).map(String::as_str)
        } else {
            token
                .strip_prefix(name)
                .and_then(|value| value.strip_prefix('='))
        }
    })
}

fn required_option(tokens: &[String], name: &str, path: &Path) -> Result<String, OrbitError> {
    option_value(tokens, name)
        .filter(|value| !value.trim().is_empty())
        .map(ToString::to_string)
        .ok_or_else(|| {
            other(format!(
                "server argument file '{}' has no {name} value",
                path.display()
            ))
        })
}

fn validate_optional_option(
    tokens: &[String],
    name: &str,
    expected: &str,
    path: &Path,
) -> Result<(), OrbitError> {
    if let Some(actual) = option_value(tokens, name)
        && actual != expected
    {
        return Err(other(format!(
            "server argument file '{}' declares {name} {actual}, expected {expected}",
            path.display()
        )));
    }
    Ok(())
}

fn read_minecraft_metadata(
    instance_dir: &Path,
    runtime_jar: &Path,
    expected_version: &str,
) -> Result<McVersion, OrbitError> {
    if let Ok(version) = crate::jar::read_minecraft_version(runtime_jar) {
        validate_minecraft_version(&version, Some(expected_version), runtime_jar)?;
        return Ok(version);
    }

    let mut metadata_jars = vec![instance_dir.join("server.jar")];
    let server_library = instance_dir
        .join("libraries")
        .join("net")
        .join("minecraft")
        .join("server")
        .join(expected_version);
    metadata_jars.extend(direct_jars(&server_library)?);
    for path in metadata_jars {
        let Ok(version) = crate::jar::read_minecraft_version(&path) else {
            continue;
        };
        if version.id == expected_version {
            return Ok(version);
        }
    }
    Err(other(format!(
        "runtime game JAR '{}' and installed Minecraft server artifacts contain no version.json for '{}'",
        runtime_jar.display(),
        expected_version
    )))
}

fn validate_minecraft_version(
    actual: &McVersion,
    expected: Option<&str>,
    jar: &Path,
) -> Result<(), OrbitError> {
    if let Some(expected) = expected
        && actual.id != expected
    {
        return Err(other(format!(
            "Minecraft JAR '{}' declares version '{}', expected '{}'",
            jar.display(),
            actual.id,
            expected
        )));
    }
    Ok(())
}

fn split_forge_version(version: &str) -> Option<(String, String)> {
    version.split_once('-').and_then(|(minecraft, loader)| {
        (minecraft.contains('.')
            && minecraft
                .chars()
                .all(|character| character.is_ascii_digit() || character == '.')
            && loader
                .chars()
                .next()
                .is_some_and(|character| character.is_ascii_digit()))
        .then(|| (minecraft.to_string(), loader.to_string()))
    })
}

fn exactly_one_named_jar(
    instance_dir: &Path,
    directory: &Path,
    filename: &str,
    label: &str,
) -> Result<PathBuf, OrbitError> {
    let path = directory.join(filename);
    canonical_within(instance_dir, &path, label)
}

fn verify_listed_file(
    instance_dir: &Path,
    path: &Path,
    expected_hash: &str,
    label: &str,
) -> Result<(), OrbitError> {
    let path = canonical_within(instance_dir, path, label)?;
    let expected = expected_hash.trim().to_ascii_lowercase();
    let actual = match expected.len() {
        40 => crate::jar::compute_sha1(&path)?,
        64 => crate::jar::compute_sha256(&path)?,
        _ => {
            return Err(other(format!(
                "{label} '{}' has unsupported hash '{}'",
                path.display(),
                expected_hash
            )));
        }
    };
    if actual != expected {
        return Err(other(format!(
            "{label} '{}' does not match its launcher-declared hash",
            path.display()
        )));
    }
    Ok(())
}

fn resolve_instance_path(
    instance_dir: &Path,
    base: &Path,
    path: &Path,
    label: &str,
) -> Result<PathBuf, OrbitError> {
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    };
    canonical_within(instance_dir, &candidate, label)
}

fn canonical_within(instance_dir: &Path, path: &Path, label: &str) -> Result<PathBuf, OrbitError> {
    require_file(path, label)?;
    let root = instance_dir.canonicalize().map_err(|error| {
        other(format!(
            "cannot resolve dedicated-server directory '{}': {error}",
            instance_dir.display()
        ))
    })?;
    let path = path.canonicalize().map_err(|error| {
        other(format!(
            "cannot resolve {label} '{}': {error}",
            path.display()
        ))
    })?;
    if !path.starts_with(&root) {
        return Err(other(format!(
            "{label} '{}' resolves outside dedicated-server directory '{}'",
            path.display(),
            root.display()
        )));
    }
    Ok(path)
}

fn require_file(path: &Path, label: &str) -> Result<(), OrbitError> {
    if !path.is_file() {
        return Err(other(format!(
            "{label} '{}' does not exist",
            path.display()
        )));
    }
    Ok(())
}

fn direct_jars(directory: &Path) -> Result<Vec<PathBuf>, OrbitError> {
    if !directory.is_dir() {
        return Ok(Vec::new());
    }
    let mut paths = Vec::new();
    for entry in std::fs::read_dir(directory)? {
        let path = entry?.path();
        if path.is_file()
            && path
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("jar"))
        {
            paths.push(path);
        }
    }
    paths.sort();
    Ok(paths)
}

fn read_manifest(path: &Path) -> Result<Option<Manifest>, OrbitError> {
    let Some(content) = read_jar_utf8(path, "META-INF/MANIFEST.MF")? else {
        return Ok(None);
    };
    let mut attributes = HashMap::<String, String>::new();
    let mut current_key: Option<String> = None;
    for line in content.replace("\r\n", "\n").split('\n') {
        if line.is_empty() {
            break;
        }
        if let Some(continuation) = line.strip_prefix(' ') {
            let key = current_key.as_ref().ok_or_else(|| {
                other(format!(
                    "manifest continuation without an attribute in '{}'",
                    path.display()
                ))
            })?;
            attributes.get_mut(key).unwrap().push_str(continuation);
            continue;
        }
        let (key, value) = line.split_once(": ").ok_or_else(|| {
            other(format!(
                "invalid manifest line in '{}': {line}",
                path.display()
            ))
        })?;
        let normalized = key.to_ascii_lowercase();
        if attributes
            .insert(normalized.clone(), value.to_string())
            .is_some()
        {
            return Err(other(format!(
                "duplicate manifest attribute '{key}' in '{}'",
                path.display()
            )));
        }
        current_key = Some(normalized);
    }
    let class_path = attributes
        .remove("class-path")
        .map(|value| value.split_whitespace().map(ToString::to_string).collect())
        .unwrap_or_default();
    Ok(Some(Manifest {
        main_class: attributes.remove("main-class"),
        class_path,
    }))
}

fn read_jar_properties(
    path: &Path,
    name: &str,
) -> Result<Option<HashMap<String, String>>, OrbitError> {
    let Some(bytes) = read_jar_entry(path, name)? else {
        return Ok(None);
    };
    java_properties::read(Cursor::new(bytes))
        .map(Some)
        .map_err(|error| {
            other(format!(
                "invalid Java properties entry '{name}' in '{}': {error}",
                path.display()
            ))
        })
}

fn read_external_properties(
    instance_dir: &Path,
    name: &str,
) -> Result<Option<HashMap<String, String>>, OrbitError> {
    let path = instance_dir.join(name);
    if !path.is_file() {
        return Ok(None);
    }
    let file = std::fs::File::open(&path)?;
    java_properties::read(file).map(Some).map_err(|error| {
        other(format!(
            "invalid Java properties file '{}': {error}",
            path.display()
        ))
    })
}

fn required_property(
    properties: &HashMap<String, String>,
    name: &str,
    jar: &Path,
) -> Result<String, OrbitError> {
    properties
        .get(name)
        .filter(|value| !value.trim().is_empty())
        .cloned()
        .ok_or_else(|| {
            other(format!(
                "server launcher '{}' has no '{name}' property",
                jar.display()
            ))
        })
}

fn read_jar_utf8(path: &Path, name: &str) -> Result<Option<String>, OrbitError> {
    let Some(bytes) = read_jar_entry(path, name)? else {
        return Ok(None);
    };
    String::from_utf8(bytes).map(Some).map_err(|error| {
        other(format!(
            "JAR entry '{name}' in '{}' is not UTF-8: {error}",
            path.display()
        ))
    })
}

fn read_jar_entry(path: &Path, name: &str) -> Result<Option<Vec<u8>>, OrbitError> {
    let file = std::fs::File::open(path).map_err(|error| {
        other(format!(
            "cannot open server JAR '{}': {error}",
            path.display()
        ))
    })?;
    let mut archive = zip::ZipArchive::new(file).map_err(|error| {
        other(format!(
            "cannot open server JAR '{}' as ZIP: {error}",
            path.display()
        ))
    })?;
    let mut entry = match archive.by_name(name) {
        Ok(entry) => entry,
        Err(zip::result::ZipError::FileNotFound) => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let mut bytes = Vec::new();
    entry.read_to_end(&mut bytes)?;
    Ok(Some(bytes))
}

fn normalize_runtime_jars(paths: &mut Vec<PathBuf>) {
    paths.sort();
    paths.dedup();
}

fn platform_argfile_name() -> &'static str {
    if cfg!(windows) {
        "win_args.txt"
    } else {
        "unix_args.txt"
    }
}

fn display_relative(instance_dir: &Path, path: &Path) -> String {
    path.strip_prefix(instance_dir)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn other(message: impl Into<String>) -> OrbitError {
    OrbitError::Other(anyhow::anyhow!(message.into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    fn discover_server_runtime(
        instance_dir: &Path,
    ) -> Result<Option<ServerRuntimeSpec>, OrbitError> {
        super::super::discover_server_runtime(instance_dir)
    }

    #[test]
    fn tokenizes_java_argument_files_without_treating_windows_paths_as_escapes() {
        let tokens = tokenize_java_argfile(
            "# comment\n-classpath\n\"libraries/a.jar;libraries/b jar.jar\"\n--launchTarget forge_server",
        )
        .unwrap();
        assert_eq!(
            tokens,
            vec![
                "-classpath",
                "libraries/a.jar;libraries/b jar.jar",
                "--launchTarget",
                "forge_server"
            ]
        );
    }

    #[test]
    fn discovers_a_quilt_server_from_its_exact_manifest_classpath() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        std::fs::write(root.join("eula.txt"), "eula=true").unwrap();
        let loader = root.join("libraries/quilt-loader.jar");
        std::fs::create_dir_all(loader.parent().unwrap()).unwrap();
        write_jar(
            &loader,
            &[(
                "quilt.mod.json",
                br#"{"schema_version":1,"quilt_loader":{"group":"org.quiltmc","id":"quilt_loader","version":"0.29.0","metadata":{"name":"Quilt Loader"}}}"#,
            )],
        );
        write_jar(
            &root.join("server.jar"),
            &[("version.json", minecraft_version_json("1.21.1").as_bytes())],
        );
        write_jar(
            &root.join("quilt-server-launch.jar"),
            &[(
                "META-INF/MANIFEST.MF",
                b"Manifest-Version: 1.0\r\nMain-Class: org.quiltmc.loader.impl.launch.server.QuiltServerLauncher\r\nClass-Path: libraries/quilt-loader.jar\r\n\r\n",
            )],
        );

        let spec = discover_server_runtime(root).unwrap().unwrap();
        assert_eq!(spec.loader, LoaderKind::Quilt);
        assert_eq!(spec.loader_version, "0.29.0");
        assert_eq!(spec.minecraft.id, "1.21.1");
        assert_eq!(spec.loader_jar, loader.canonicalize().unwrap());
    }

    #[test]
    fn discovers_the_official_fabric_bootstrap_and_bundler_runtime() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        std::fs::write(root.join("eula.txt"), "eula=true").unwrap();
        let runtime_dir = root.join(".fabric/server");
        let loader = root.join("libraries/net/fabricmc/fabric-loader.jar");
        let minecraft = root.join("versions/26.1.2/server-26.1.2.jar");
        let bundled_library = root.join("libraries/com/mojang/example.jar");
        std::fs::create_dir_all(&runtime_dir).unwrap();
        std::fs::create_dir_all(loader.parent().unwrap()).unwrap();
        std::fs::create_dir_all(minecraft.parent().unwrap()).unwrap();
        std::fs::create_dir_all(bundled_library.parent().unwrap()).unwrap();
        write_jar(
            &loader,
            &[(
                "fabric.mod.json",
                br#"{"schemaVersion":1,"id":"fabricloader","version":"0.19.2","name":"Fabric Loader"}"#,
            )],
        );
        write_jar(
            &minecraft,
            &[("version.json", minecraft_version_json("26.1.2").as_bytes())],
        );
        write_jar(&bundled_library, &[("com/mojang/Example.class", b"class")]);
        let minecraft_hash = crate::jar::compute_sha256(&minecraft).unwrap();
        let library_hash = crate::jar::compute_sha256(&bundled_library).unwrap();
        write_jar(
            &runtime_dir.join("26.1.2-server.jar"),
            &[
                (
                    "META-INF/versions.list",
                    format!("{minecraft_hash}\t26.1.2\t26.1.2/server-26.1.2.jar\n").as_bytes(),
                ),
                (
                    "META-INF/libraries.list",
                    format!("{library_hash}\tcom.mojang:example:1\tcom/mojang/example.jar\n")
                        .as_bytes(),
                ),
            ],
        );
        write_jar(
            &runtime_dir.join("fabric-loader-server-0.19.2-minecraft-26.1.2.jar"),
            &[(
                "META-INF/MANIFEST.MF",
                b"Manifest-Version: 1.0\r\nMain-Class: net.fabricmc.loader.impl.launch.server.FabricServerLauncher\r\nClass-Path: ../../libraries/net/fabricmc/fabric-loader.jar\r\n\r\n",
            )],
        );
        write_jar(
            &root.join("fabric-server-launch.jar"),
            &[
                (
                    "META-INF/MANIFEST.MF",
                    b"Manifest-Version: 1.0\r\nMain-Class: net.fabricmc.installer.ServerLauncher\r\n\r\n",
                ),
                (
                    "install.properties",
                    b"fabric-loader-version=0.19.2\ngame-version=26.1.2\n",
                ),
            ],
        );

        let spec = discover_server_runtime(root).unwrap().unwrap();
        assert_eq!(spec.loader, LoaderKind::Fabric);
        assert_eq!(spec.loader_version, "0.19.2");
        assert_eq!(spec.minecraft.id, "26.1.2");
        assert_eq!(spec.minecraft_jar, minecraft.canonicalize().unwrap());
        assert!(
            spec.runtime_jars
                .contains(&bundled_library.canonicalize().unwrap())
        );

        let loaders =
            crate::platform_detection::detect_loader_candidates(root, "26.1.2", None).unwrap();
        let certain = loaders
            .iter()
            .filter(|candidate| candidate.certain)
            .collect::<Vec<_>>();
        assert_eq!(certain.len(), 1);
        assert_eq!(certain[0].loader, LoaderKind::Fabric);
        assert_eq!(certain[0].versions, vec!["0.19.2"]);

        let platform = crate::platform_detection::discover_platform_for_init(
            root, "26.1.2", "fabric", "0.19.2",
        )
        .unwrap();
        assert_eq!(platform.minecraft_jar, minecraft.canonicalize().unwrap());
        assert_eq!(platform.loader_jar, loader.canonicalize().unwrap());
        assert_eq!(
            platform.physical_environment,
            crate::metadata::Environment::Server
        );
        let snapshot = platform.snapshot(root).unwrap();
        assert_eq!(
            snapshot.physical_environment,
            crate::metadata::Environment::Server
        );
        assert!(
            snapshot
                .runtime_jars
                .iter()
                .any(|artifact| artifact.path.ends_with("com/mojang/example.jar"))
        );
        let rediscovered = crate::platform_detection::rediscover_current_platform(root).unwrap();
        assert_eq!(rediscovered.minecraft_version.id, "26.1.2");
        assert_eq!(rediscovered.loader, LoaderKind::Fabric);
        assert_eq!(rediscovered.loader_version, "0.19.2");
        assert_eq!(
            rediscovered.minecraft_jar,
            minecraft.canonicalize().unwrap()
        );
    }

    #[test]
    fn discovers_forge_from_the_hash_verified_bootstrap_shim() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        std::fs::write(root.join("server.properties"), "online-mode=true").unwrap();
        let coordinate_dir = root.join("libraries/net/minecraftforge/forge/26.1.2-64.0.14");
        std::fs::create_dir_all(&coordinate_dir).unwrap();
        let server = coordinate_dir.join("forge-26.1.2-64.0.14-server.jar");
        let universal = coordinate_dir.join("forge-26.1.2-64.0.14-universal.jar");
        write_jar(
            &server,
            &[("version.json", minecraft_version_json("26.1.2").as_bytes())],
        );
        write_jar(&universal, &[("META-INF/forge-test", b"loader")]);
        let server_hash = crate::jar::compute_sha256(&server).unwrap();
        let universal_hash = crate::jar::compute_sha256(&universal).unwrap();
        let list = format!(
            "{server_hash}\tnet.minecraftforge:forge:26.1.2-64.0.14:server\tnet/minecraftforge/forge/26.1.2-64.0.14/forge-26.1.2-64.0.14-server.jar\n\
             {universal_hash}\tnet.minecraftforge:forge:26.1.2-64.0.14:universal\tnet/minecraftforge/forge/26.1.2-64.0.14/forge-26.1.2-64.0.14-universal.jar\n"
        );
        write_jar(
            &root.join("forge-26.1.2-64.0.14-shim.jar"),
            &[
                (
                    "META-INF/MANIFEST.MF",
                    b"Manifest-Version: 1.0\r\nMain-Class: net.minecraftforge.bootstrap.shim.Main\r\n\r\n",
                ),
                ("bootstrap-shim.list", list.as_bytes()),
            ],
        );

        let spec = discover_server_runtime(root).unwrap().unwrap();
        assert_eq!(spec.loader, LoaderKind::Forge);
        assert_eq!(spec.loader_version, "64.0.14");
        assert_eq!(spec.minecraft.id, "26.1.2");
        assert_eq!(spec.minecraft_jar, server.canonicalize().unwrap());
        assert_eq!(spec.loader_jar, universal.canonicalize().unwrap());
    }

    #[test]
    fn rejects_a_forge_bootstrap_shim_with_a_mismatched_hash() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        std::fs::write(root.join("eula.txt"), "eula=true").unwrap();
        let coordinate_dir = root.join("libraries/net/minecraftforge/forge/26.1.2-64.0.14");
        std::fs::create_dir_all(&coordinate_dir).unwrap();
        let server = coordinate_dir.join("forge-26.1.2-64.0.14-server.jar");
        let universal = coordinate_dir.join("forge-26.1.2-64.0.14-universal.jar");
        write_jar(
            &server,
            &[("version.json", minecraft_version_json("26.1.2").as_bytes())],
        );
        write_jar(&universal, &[("META-INF/forge-test", b"loader")]);
        let universal_hash = crate::jar::compute_sha256(&universal).unwrap();
        let list = format!(
            "{}\tnet.minecraftforge:forge:26.1.2-64.0.14:server\tnet/minecraftforge/forge/26.1.2-64.0.14/forge-26.1.2-64.0.14-server.jar\n\
             {universal_hash}\tnet.minecraftforge:forge:26.1.2-64.0.14:universal\tnet/minecraftforge/forge/26.1.2-64.0.14/forge-26.1.2-64.0.14-universal.jar\n",
            "0".repeat(64)
        );
        write_jar(
            &root.join("forge-26.1.2-64.0.14-shim.jar"),
            &[
                (
                    "META-INF/MANIFEST.MF",
                    b"Manifest-Version: 1.0\r\nMain-Class: net.minecraftforge.bootstrap.shim.Main\r\n\r\n",
                ),
                ("bootstrap-shim.list", list.as_bytes()),
            ],
        );

        let error = discover_server_runtime(root).unwrap_err().to_string();
        assert!(error.contains("does not match its launcher-declared hash"));
    }

    #[test]
    fn discovers_neoforge_from_the_platform_argument_file() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        std::fs::write(root.join("eula.txt"), "eula=true").unwrap();
        let version = "26.1.2.87";
        let coordinate_dir = root.join(format!("libraries/net/neoforged/neoforge/{version}"));
        let patched_dir = root.join(format!(
            "libraries/net/neoforged/minecraft-server-patched/{version}"
        ));
        std::fs::create_dir_all(&coordinate_dir).unwrap();
        std::fs::create_dir_all(&patched_dir).unwrap();
        let universal = coordinate_dir.join(format!("neoforge-{version}-universal.jar"));
        let minecraft = patched_dir.join(format!("minecraft-server-patched-{version}.jar"));
        let runtime = root.join("libraries/net/neoforged/runtime.jar");
        std::fs::create_dir_all(runtime.parent().unwrap()).unwrap();
        write_jar(&universal, &[("META-INF/neoforge-test", b"loader")]);
        write_jar(
            &minecraft,
            &[("version.json", minecraft_version_json("26.1.2").as_bytes())],
        );
        write_jar(&runtime, &[("net/neoforged/Runtime.class", b"class")]);
        let classpath = std::env::join_paths([Path::new("libraries/net/neoforged/runtime.jar")])
            .unwrap()
            .to_string_lossy()
            .into_owned();
        std::fs::write(
            coordinate_dir.join(platform_argfile_name()),
            format!(
                "-classpath\n{classpath}\nnet.neoforged.fml.startup.Server\n\
                 --fml.neoForgeVersion {version}\n--fml.mcVersion 26.1.2\n"
            ),
        )
        .unwrap();

        let spec = discover_server_runtime(root).unwrap().unwrap();
        assert_eq!(spec.loader, LoaderKind::NeoForge);
        assert_eq!(spec.loader_version, version);
        assert_eq!(spec.minecraft.id, "26.1.2");
        assert_eq!(spec.loader_jar, universal.canonicalize().unwrap());
        assert!(spec.runtime_jars.contains(&runtime.canonicalize().unwrap()));
    }

    #[test]
    fn discovers_modlauncher_forge_from_the_platform_argument_file() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        std::fs::write(root.join("server.properties"), "online-mode=true").unwrap();
        let coordinate = "1.20.1-47.4.22";
        let coordinate_dir = root.join(format!("libraries/net/minecraftforge/forge/{coordinate}"));
        std::fs::create_dir_all(&coordinate_dir).unwrap();
        let universal = coordinate_dir.join(format!("forge-{coordinate}-universal.jar"));
        let minecraft = coordinate_dir.join(format!("forge-{coordinate}-server.jar"));
        let runtime = root.join("libraries/net/minecraftforge/fmlloader.jar");
        std::fs::create_dir_all(runtime.parent().unwrap()).unwrap();
        write_jar(&universal, &[("META-INF/forge-test", b"loader")]);
        write_jar(
            &minecraft,
            &[("version.json", minecraft_version_json("1.20.1").as_bytes())],
        );
        write_jar(&runtime, &[("net/minecraftforge/FML.class", b"class")]);
        let classpath =
            std::env::join_paths([Path::new("libraries/net/minecraftforge/fmlloader.jar")])
                .unwrap()
                .to_string_lossy()
                .into_owned();
        std::fs::write(
            coordinate_dir.join(platform_argfile_name()),
            format!(
                "-DlegacyClassPath={classpath}\ncpw.mods.bootstraplauncher.BootstrapLauncher\n\
                 --launchTarget forgeserver\n--fml.forgeVersion 47.4.22\n\
                 --fml.mcVersion 1.20.1\n"
            ),
        )
        .unwrap();

        let spec = discover_server_runtime(root).unwrap().unwrap();
        assert_eq!(spec.loader, LoaderKind::Forge);
        assert_eq!(spec.loader_version, "47.4.22");
        assert_eq!(spec.minecraft.id, "1.20.1");
        assert_eq!(spec.minecraft_jar, minecraft.canonicalize().unwrap());
        assert_eq!(spec.loader_jar, universal.canonicalize().unwrap());
        assert!(spec.runtime_jars.contains(&runtime.canonicalize().unwrap()));
    }

    #[test]
    fn rejects_multiple_different_server_installations() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        std::fs::write(root.join("eula.txt"), "eula=true").unwrap();
        for (loader, version, main, metadata_name, metadata) in [
            (
                "fabric",
                "0.19.2",
                FABRIC_SERVER_MAINS[0],
                "fabric.mod.json",
                r#"{"schemaVersion":1,"id":"fabricloader","version":"0.19.2","name":"Fabric Loader"}"#,
            ),
            (
                "quilt",
                "0.29.0",
                QUILT_SERVER_MAINS[0],
                "quilt.mod.json",
                r#"{"schema_version":1,"quilt_loader":{"group":"org.quiltmc","id":"quilt_loader","version":"0.29.0","metadata":{"name":"Quilt Loader"}}}"#,
            ),
        ] {
            let loader_jar = root.join(format!("libraries/{loader}.jar"));
            std::fs::create_dir_all(loader_jar.parent().unwrap()).unwrap();
            write_jar(&loader_jar, &[(metadata_name, metadata.as_bytes())]);
            write_jar(
                &root.join(format!("{loader}-server-launch.jar")),
                &[(
                    "META-INF/MANIFEST.MF",
                    format!(
                        "Manifest-Version: 1.0\r\nMain-Class: {main}\r\nClass-Path: libraries/{loader}.jar\r\n\r\n"
                    )
                    .as_bytes(),
                )],
            );
            assert!(!version.is_empty());
        }
        write_jar(
            &root.join("server.jar"),
            &[("version.json", minecraft_version_json("1.21.1").as_bytes())],
        );

        let error = discover_server_runtime(root).unwrap_err().to_string();
        assert!(error.contains("multiple installed dedicated-server runtimes"));
    }

    #[test]
    fn rejects_a_server_marker_without_a_complete_loader_runtime() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join("server.properties"),
            "online-mode=true",
        )
        .unwrap();

        let error = discover_server_runtime(directory.path())
            .unwrap_err()
            .to_string();

        assert!(error.contains("no complete supported loader runtime"));
    }

    fn minecraft_version_json(version: &str) -> String {
        format!(
            r#"{{"id":"{version}","name":"{version}","world_version":4534,"protocol_version":1,"pack_version":{{"resource_major":65,"resource_minor":0,"data_major":82,"data_minor":0}},"java_version":21,"stable":true}}"#
        )
    }

    fn write_jar(path: &Path, entries: &[(&str, &[u8])]) {
        let file = std::fs::File::create(path).unwrap();
        let mut archive = zip::ZipWriter::new(file);
        for (name, bytes) in entries {
            archive
                .start_file(*name, SimpleFileOptions::default())
                .unwrap();
            archive.write_all(bytes).unwrap();
        }
        archive.finish().unwrap();
    }
}
