use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::cli::{
    ConfigCommands,
    commands::CliContext,
    output::{
        ConfigEntryOutput, ConfigEntryView, ConfigListOutput, ConfigPathOutput, ConfigValueView,
        OutputFormat,
    },
};

pub async fn handle(command: ConfigCommands, ctx: &CliContext) -> Result<()> {
    let path = ctx.runtime.paths().config_file();
    let display_path = display_path(path);

    match command {
        ConfigCommands::Path => match ctx.output.format {
            OutputFormat::Text => ctx.print_result_line(format_args!("{}", display_path.display())),
            OutputFormat::Json => ctx.print_json(
                "config",
                &ConfigPathOutput {
                    subcommand: "path".to_string(),
                    config_path: display_path.to_string_lossy().into_owned(),
                },
            ),
        },
        ConfigCommands::List => {
            let config = orbit_core::GlobalConfig::load_stored(path)?;
            let entries = orbit_core::ConfigKey::ALL
                .into_iter()
                .map(|key| entry_view(key, &config))
                .collect::<Vec<_>>();
            match ctx.output.format {
                OutputFormat::Text => {
                    ctx.print_result_line(format_args!(
                        "{}",
                        crate::cli::output::config_entries_table(&entries)
                    ));
                }
                OutputFormat::Json => ctx.print_json(
                    "config",
                    &ConfigListOutput {
                        subcommand: "list".to_string(),
                        config_path: display_path.to_string_lossy().into_owned(),
                        entries,
                    },
                ),
            }
        }
        ConfigCommands::Get { key } => {
            let key = orbit_core::ConfigKey::parse(&key)?;
            let config = orbit_core::GlobalConfig::load_stored(path)?;
            render_entry("get", display_path, false, entry_view(key, &config), ctx);
        }
        ConfigCommands::Set { key, value } => {
            let key = orbit_core::ConfigKey::parse(&key)?;
            let mut config = orbit_core::GlobalConfig::load_stored(path)?;
            key.set(&mut config, &value)?;

            if key == orbit_core::ConfigKey::CoreDefaultInstance {
                let instance_name = config
                    .core
                    .default_instance
                    .as_deref()
                    .expect("setting the default instance always produces a value");
                let registry =
                    orbit_core::InstancesRegistry::load(ctx.runtime.paths().instances_file())?;
                if registry.find(instance_name).is_none() {
                    anyhow::bail!(
                        "{}",
                        tr!(
                            "Instance '%{instance}' was not found; run 'orbit instances list' to see registered instances",
                            instance = instance_name
                        )
                    );
                }
                if !ctx.dry_run {
                    orbit_core::set_default_instance(ctx.runtime.paths(), instance_name)?;
                }
            } else if !ctx.dry_run {
                orbit_core::persist_config_field(path, key, &config)?;
            }

            render_entry(
                "set",
                display_path,
                ctx.dry_run,
                entry_view(key, &config),
                ctx,
            );
        }
        ConfigCommands::Unset { key } => {
            let key = orbit_core::ConfigKey::parse(&key)?;
            let mut config = orbit_core::GlobalConfig::load_stored(path)?;
            key.unset(&mut config);

            if !ctx.dry_run {
                if key == orbit_core::ConfigKey::CoreDefaultInstance {
                    orbit_core::clear_default_instance(ctx.runtime.paths())?;
                } else {
                    orbit_core::persist_config_field(path, key, &config)?;
                }
            }

            render_entry(
                "unset",
                display_path,
                ctx.dry_run,
                entry_view(key, &config),
                ctx,
            );
        }
    }
    Ok(())
}

fn display_path(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

fn entry_view(key: orbit_core::ConfigKey, config: &orbit_core::GlobalConfig) -> ConfigEntryView {
    let value = match key.get(config) {
        orbit_core::ConfigValue::Absent => None,
        orbit_core::ConfigValue::Text(_) if key.is_sensitive() => {
            Some(ConfigValueView::Text("<redacted>".to_string()))
        }
        orbit_core::ConfigValue::Text(value) => Some(ConfigValueView::Text(value)),
        orbit_core::ConfigValue::Integer(value) => Some(ConfigValueView::Integer(value)),
    };
    ConfigEntryView {
        key: key.as_str().to_string(),
        value_type: key.value_type().to_string(),
        sensitive: key.is_sensitive(),
        value,
    }
}

fn render_entry(
    subcommand: &str,
    path: PathBuf,
    dry_run: bool,
    entry: ConfigEntryView,
    ctx: &CliContext,
) {
    match ctx.output.format {
        OutputFormat::Text => {
            let prefix = if dry_run { tr!("[dry-run] ") } else { tr!("") };
            let value = match &entry.value {
                Some(ConfigValueView::Text(value)) => value.clone(),
                Some(ConfigValueView::Integer(value)) => value.to_string(),
                None => "—".to_string(),
            };
            ctx.print_result_line(format_args!("{prefix}{} = {value}", entry.key));
        }
        OutputFormat::Json => ctx.print_json(
            "config",
            &ConfigEntryOutput {
                subcommand: subcommand.to_string(),
                config_path: path.to_string_lossy().into_owned(),
                dry_run,
                entry,
            },
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_are_redacted_at_the_output_boundary() {
        let mut config = orbit_core::GlobalConfig::default();
        config.auth.curseforge_api_key = Some("private-secret".to_string());

        let view = entry_view(orbit_core::ConfigKey::AuthCurseforgeApiKey, &config);
        let serialized = serde_json::to_string(&view).unwrap();

        assert!(view.sensitive);
        assert!(serialized.contains("<redacted>"));
        assert!(!serialized.contains("private-secret"));
    }
}
