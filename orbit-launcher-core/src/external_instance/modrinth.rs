//! Read-only adapter for released Modrinth App SQLite layouts. Credentials and
//! arbitrary database contents are never read, exported, migrated or modified.
use super::{ExternalInstance, ExternalRuntime, invalid};
use crate::{InstanceKind, LauncherError, LoaderKind};
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use std::path::Path;

pub(super) fn probe(directory: &Path) -> Result<Option<Vec<ExternalInstance>>, LauncherError> {
    let (settings, selected) = if directory.join("app.db").is_file() {
        (directory, None)
    } else if let Some(root) = directory
        .parent()
        .and_then(Path::parent)
        .filter(|p| p.join("app.db").is_file())
    {
        (root, Some(directory))
    } else {
        return Ok(None);
    };
    let db = Connection::open_with_flags(settings.join("app.db"), OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(sql_error)?;
    let config: Option<String> = db
        .query_row("SELECT custom_dir FROM settings WHERE id = 0", [], |r| {
            r.get(0)
        })
        .optional()
        .map_err(sql_error)?
        .flatten();
    let root = config
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| settings.to_path_buf());
    if !root.is_absolute() {
        return Err(invalid("Modrinth custom_dir must be absolute"));
    }
    let root = dunce::canonicalize(root.join("profiles"))?;
    let current: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='instances')",
            [],
            |r| r.get(0),
        )
        .map_err(sql_error)?;
    let query = if current {
        "SELECT i.id, i.path, i.name, c.game_version, c.loader, c.loader_version FROM instances i JOIN instance_content_sets c ON c.id=i.applied_content_set_id AND c.instance_id=i.id"
    } else {
        "SELECT path, path, name, game_version, mod_loader, mod_loader_version FROM profiles"
    };
    let mut statement = db.prepare(query).map_err(sql_error)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, Option<String>>(5)?,
            ))
        })
        .map_err(sql_error)?;
    let mut instances = Vec::new();
    for row in rows {
        let (id, path, name, minecraft, loader, loader_version) = row.map_err(sql_error)?;
        crate::layout::validate_directory_name(&path).map_err(invalid)?;
        let game = dunce::canonicalize(root.join(path))?;
        if !game.starts_with(&root) {
            return Err(invalid(
                "Modrinth instance resolves outside its profiles directory",
            ));
        }
        if selected.is_some_and(|p| p != game) {
            continue;
        }
        let loader: LoaderKind = match loader.as_str() {
            "neo" => LoaderKind::Neoforge,
            _ => loader.parse()?,
        };
        if (loader == LoaderKind::Vanilla) != loader_version.is_none() {
            return Err(invalid("incomplete Modrinth Loader version"));
        }
        instances.push(ExternalInstance { id, name, format:"modrinth".into(), kind:InstanceKind::Client,
            profile_directory:game.clone(), game_directory:Some(game), runtime:Some(ExternalRuntime {minecraft,loader,loader_version}),
            directory_evidence:"Modrinth App applied content set and profiles directory".into(), warnings:vec!["Shared-instance links, accounts, runtime overrides and launcher hooks are not imported.".into()], error:None });
    }
    Ok(Some(instances))
}
fn sql_error(error: rusqlite::Error) -> LauncherError {
    invalid(format!(
        "cannot read the supported Modrinth App database schema: {error}"
    ))
}
