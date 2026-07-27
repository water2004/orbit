use std::path::Path;

use orbit_launcher_core::{
    ConfigKey, ContextIntent, CreateInstanceRequest, InstanceRegistry, RuntimeContext,
    create_instance, get_config, import_instance, list_config, remove_instance, rename_instance,
    resolve_instance, resolve_instance_root, set_config, set_default_instance, unset_config,
};

use crate::cli::{Commands, ConfigCommands, DefaultCommands, InstanceCommands};
use crate::output::{
    ConfigEntryView, ConfigListView, ConfigMutationAction, ConfigMutationView, ConfigPathView,
    DefaultView, InstanceDetailView, InstanceListView, InstanceMutationAction,
    InstanceMutationView, InstanceView, RenameView,
};

#[derive(Debug)]
pub enum CommandOutput {
    ConfigPath(ConfigPathView),
    ConfigList(ConfigListView),
    ConfigEntry(ConfigEntryView),
    ConfigMutation(ConfigMutationView),
    InstanceList(InstanceListView),
    InstanceDetail(InstanceDetailView),
    InstanceMutation(InstanceMutationView),
    Rename(RenameView),
    Default(DefaultView),
}

impl CommandOutput {
    pub fn command_name(&self) -> &'static str {
        match self {
            Self::ConfigPath(_) => "config.path",
            Self::ConfigList(_) => "config.list",
            Self::ConfigEntry(_) => "config.get",
            Self::ConfigMutation(view) => view.action.command_name(),
            Self::InstanceList(_) => "instance.list",
            Self::InstanceDetail(_) => "instance.show",
            Self::InstanceMutation(view) => view.action.command_name(),
            Self::Rename(_) => "instance.rename",
            Self::Default(_) => "instance.default",
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error(transparent)]
    Core(#[from] orbit_launcher_core::LauncherError),
    #[error("invalid command usage: {0}")]
    Argument(String),
}

impl AppError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Core(error) => error.code(),
            Self::Argument(_) => "argument",
        }
    }
}

pub fn execute(
    command: Commands,
    instance_selector: Option<&str>,
    current_dir: &Path,
    runtime: &RuntimeContext,
) -> Result<CommandOutput, AppError> {
    match command {
        Commands::Config { command } => {
            if instance_selector.is_some() {
                return Err(AppError::Argument(
                    "--instance is not valid for configuration commands".to_string(),
                ));
            }
            execute_config(command, runtime)
        }
        Commands::Instance { command } => {
            if instance_selector.is_some() && !command.accepts_instance_context() {
                return Err(AppError::Argument(
                    "--instance is not valid for this instance subcommand".to_string(),
                ));
            }
            execute_instance(command, instance_selector, current_dir, runtime)
        }
    }
}

fn execute_config(
    command: ConfigCommands,
    runtime: &RuntimeContext,
) -> Result<CommandOutput, AppError> {
    let path = runtime.paths().config_file();
    match command {
        ConfigCommands::Path => Ok(CommandOutput::ConfigPath(ConfigPathView { path })),
        ConfigCommands::List => Ok(CommandOutput::ConfigList(ConfigListView {
            settings: list_config(&path)?
                .into_iter()
                .map(ConfigEntryView::from)
                .collect(),
        })),
        ConfigCommands::Get { key } => {
            let key = key.parse::<ConfigKey>()?;
            Ok(CommandOutput::ConfigEntry(get_config(&path, key)?.into()))
        }
        ConfigCommands::Set { key, value } => {
            let key = key.parse::<ConfigKey>()?;
            Ok(CommandOutput::ConfigMutation(ConfigMutationView::new(
                set_config(&path, key, &value)?,
                ConfigMutationAction::Set,
            )))
        }
        ConfigCommands::Unset { key } => {
            let key = key.parse::<ConfigKey>()?;
            Ok(CommandOutput::ConfigMutation(ConfigMutationView::new(
                unset_config(&path, key)?,
                ConfigMutationAction::Unset,
            )))
        }
    }
}

fn execute_instance(
    command: InstanceCommands,
    selector: Option<&str>,
    current_dir: &Path,
    runtime: &RuntimeContext,
) -> Result<CommandOutput, AppError> {
    let registry_path = runtime.paths().instances_file();
    match command {
        InstanceCommands::Create {
            name,
            root,
            kind,
            minecraft,
            loader,
            loader_version,
        } => {
            let root = resolve_instance_root(current_dir, root.as_deref())?;
            let created = create_instance(
                runtime.paths(),
                CreateInstanceRequest {
                    root,
                    name,
                    kind: kind.into(),
                    minecraft_requirement: minecraft,
                    loader_kind: loader.into(),
                    loader_requirement: loader_version,
                },
            )?;
            Ok(CommandOutput::InstanceMutation(InstanceMutationView {
                instance: InstanceView::from_entry(&created.entry, None),
                action: InstanceMutationAction::Created,
                files_deleted: false,
            }))
        }
        InstanceCommands::Import { root } => {
            let root = resolve_instance_root(current_dir, root.as_deref())?;
            let imported = import_instance(runtime.paths(), &root)?;
            let registry = InstanceRegistry::load(&registry_path)?;
            Ok(CommandOutput::InstanceMutation(InstanceMutationView {
                instance: InstanceView::from_entry(&imported.entry, registry.default_instance),
                action: InstanceMutationAction::Imported,
                files_deleted: false,
            }))
        }
        InstanceCommands::List => {
            let registry = InstanceRegistry::load(&registry_path)?;
            let instances = registry
                .instances
                .iter()
                .map(|entry| InstanceView::from_entry(entry, registry.default_instance))
                .collect();
            Ok(CommandOutput::InstanceList(InstanceListView { instances }))
        }
        InstanceCommands::Show => {
            let registry = InstanceRegistry::load(&registry_path)?;
            let resolved =
                resolve_instance(&registry, selector, current_dir, ContextIntent::ReadOnly)?;
            Ok(CommandOutput::InstanceDetail(InstanceDetailView::new(
                &resolved.entry,
                &resolved.manifest,
                registry.default_instance,
                resolved.source,
            )))
        }
        InstanceCommands::Rename { new_name } => {
            let registry = InstanceRegistry::load(&registry_path)?;
            let resolved =
                resolve_instance(&registry, selector, current_dir, ContextIntent::Sensitive)?;
            let renamed =
                rename_instance(runtime.paths(), &resolved.entry.id.to_string(), &new_name)?;
            Ok(CommandOutput::Rename(RenameView {
                id: renamed.id.to_string(),
                old_name: renamed.old_name,
                new_name: renamed.new_name,
            }))
        }
        InstanceCommands::Remove => {
            let registry = InstanceRegistry::load(&registry_path)?;
            let resolved =
                resolve_instance(&registry, selector, current_dir, ContextIntent::Sensitive)?;
            let removed = remove_instance(runtime.paths(), &resolved.entry.id.to_string())?;
            Ok(CommandOutput::InstanceMutation(InstanceMutationView {
                instance: InstanceView::from_entry(&removed.entry, None),
                action: InstanceMutationAction::Removed,
                files_deleted: false,
            }))
        }
        InstanceCommands::Default { command } => {
            let selected = match command {
                DefaultCommands::Set { instance } => {
                    set_default_instance(runtime.paths(), Some(&instance))?
                }
                DefaultCommands::Clear => set_default_instance(runtime.paths(), None)?,
                DefaultCommands::Show => InstanceRegistry::load(&registry_path)?
                    .default_entry()
                    .cloned(),
            };
            Ok(CommandOutput::Default(DefaultView {
                instance: selected
                    .as_ref()
                    .map(|entry| InstanceView::from_entry(entry, Some(entry.id))),
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{DefaultCommands, InstanceKindArg, LoaderKindArg};
    use orbit_launcher_core::{ContextSource, RuntimePathOptions};

    fn runtime(directory: &Path) -> RuntimeContext {
        RuntimeContext::load(RuntimePathOptions {
            config_dir: Some(directory.join("config")),
            data_dir: Some(directory.join("data")),
            cache_dir: Some(directory.join("cache")),
        })
        .unwrap()
    }

    #[test]
    fn current_directory_and_explicit_global_id_resolve_the_same_instance() {
        let directory = tempfile::tempdir().unwrap();
        let instance_root = directory.path().join("instance");
        std::fs::create_dir_all(&instance_root).unwrap();
        let instance_root = dunce::canonicalize(instance_root).unwrap();
        let runtime = runtime(directory.path());

        let created = execute_instance(
            InstanceCommands::Create {
                name: "server".to_string(),
                root: None,
                kind: InstanceKindArg::Server,
                minecraft: "1.21.1".to_string(),
                loader: LoaderKindArg::Fabric,
                loader_version: Some("stable".to_string()),
            },
            None,
            &instance_root,
            &runtime,
        )
        .unwrap();
        let CommandOutput::InstanceMutation(created) = created else {
            panic!("unexpected create output");
        };

        let local =
            execute_instance(InstanceCommands::Show, None, &instance_root, &runtime).unwrap();
        let CommandOutput::InstanceDetail(local) = local else {
            panic!("unexpected local show output");
        };
        assert_eq!(local.context, ContextSource::CurrentDirectory);

        let unrelated = directory.path().join("unrelated");
        std::fs::create_dir_all(&unrelated).unwrap();
        let global = execute_instance(
            InstanceCommands::Show,
            Some(&created.instance.id),
            &unrelated,
            &runtime,
        )
        .unwrap();
        let CommandOutput::InstanceDetail(global) = global else {
            panic!("unexpected global show output");
        };
        assert_eq!(global.context, ContextSource::Explicit);
        assert_eq!(global.instance.id, created.instance.id);
    }

    #[test]
    fn default_is_read_only_and_cannot_target_a_rename() {
        let directory = tempfile::tempdir().unwrap();
        let instance_root = directory.path().join("instance");
        std::fs::create_dir_all(&instance_root).unwrap();
        let instance_root = dunce::canonicalize(instance_root).unwrap();
        let runtime = runtime(directory.path());
        execute_instance(
            InstanceCommands::Create {
                name: "client".to_string(),
                root: None,
                kind: InstanceKindArg::Client,
                minecraft: "latest-release".to_string(),
                loader: LoaderKindArg::Vanilla,
                loader_version: None,
            },
            None,
            &instance_root,
            &runtime,
        )
        .unwrap();
        execute_instance(
            InstanceCommands::Default {
                command: DefaultCommands::Set {
                    instance: "client".to_string(),
                },
            },
            None,
            &instance_root,
            &runtime,
        )
        .unwrap();

        let unrelated = directory.path().join("unrelated");
        std::fs::create_dir_all(&unrelated).unwrap();
        let error = execute_instance(
            InstanceCommands::Rename {
                new_name: "renamed".to_string(),
            },
            None,
            &unrelated,
            &runtime,
        )
        .unwrap_err();
        assert_eq!(error.code(), "explicit_instance_required");
    }

    #[test]
    fn config_commands_use_typed_core_mutations() {
        let directory = tempfile::tempdir().unwrap();
        let runtime = runtime(directory.path());
        let output = execute(
            Commands::Config {
                command: ConfigCommands::Set {
                    key: "cache.max-size".to_string(),
                    value: "4 GiB".to_string(),
                },
            },
            None,
            directory.path(),
            &runtime,
        )
        .unwrap();
        let CommandOutput::ConfigMutation(view) = output else {
            panic!("unexpected config output");
        };
        assert_eq!(view.key, "cache.max-size");
        assert_eq!(view.current.as_deref(), Some("4 GiB"));
        assert!(view.explicit);

        let error = execute(
            Commands::Config {
                command: ConfigCommands::Get {
                    key: "unknown".to_string(),
                },
            },
            Some("server"),
            directory.path(),
            &runtime,
        )
        .unwrap_err();
        assert_eq!(error.code(), "argument");
    }
}
