mod app;
mod cli;
mod output;

use std::process::ExitCode;

use clap::Parser;
use cli::{Cli, OutputFormat};
use output::{ErrorEnvelope, SuccessEnvelope};

fn main() -> ExitCode {
    let cli = Cli::parse();
    let command_name = command_name(&cli.command);
    let runtime =
        orbit_launcher_core::RuntimeContext::load(orbit_launcher_core::RuntimePathOptions {
            config_dir: cli.config_dir.clone(),
            data_dir: cli.data_dir.clone(),
            cache_dir: cli.cache_dir.clone(),
        });
    let runtime = match runtime {
        Ok(runtime) => runtime,
        Err(error) => {
            return render_error(cli.format, command_name, error.code(), &error.to_string());
        }
    };
    let current_dir = match std::env::current_dir() {
        Ok(path) => path,
        Err(error) => return render_error(cli.format, command_name, "io", &error.to_string()),
    };
    match app::execute(cli.command, cli.instance.as_deref(), &current_dir, &runtime) {
        Ok(output) => {
            render_success(cli.format, output);
            ExitCode::SUCCESS
        }
        Err(error) => render_error(cli.format, command_name, error.code(), &error.to_string()),
    }
}

fn command_name(command: &cli::Commands) -> &'static str {
    match command {
        cli::Commands::Config { command } => match command {
            cli::ConfigCommands::Path => "config.path",
            cli::ConfigCommands::List => "config.list",
            cli::ConfigCommands::Get { .. } => "config.get",
            cli::ConfigCommands::Set { .. } => "config.set",
            cli::ConfigCommands::Unset { .. } => "config.unset",
        },
        cli::Commands::Instance { command } => match command {
            cli::InstanceCommands::Create { .. } => "instance.create",
            cli::InstanceCommands::Import { .. } => "instance.import",
            cli::InstanceCommands::List => "instance.list",
            cli::InstanceCommands::Show => "instance.show",
            cli::InstanceCommands::Rename { .. } => "instance.rename",
            cli::InstanceCommands::Remove => "instance.remove",
            cli::InstanceCommands::Default { .. } => "instance.default",
        },
    }
}

fn render_success(format: OutputFormat, output: app::CommandOutput) {
    let command = output.command_name();
    match format {
        OutputFormat::Json => match output {
            app::CommandOutput::ConfigPath(value) => print_json(command, value),
            app::CommandOutput::ConfigList(value) => print_json(command, value),
            app::CommandOutput::ConfigEntry(value) => print_json(command, value),
            app::CommandOutput::ConfigMutation(value) => print_json(command, value),
            app::CommandOutput::InstanceList(value) => print_json(command, value),
            app::CommandOutput::InstanceDetail(value) => print_json(command, value),
            app::CommandOutput::InstanceMutation(value) => print_json(command, value),
            app::CommandOutput::Rename(value) => print_json(command, value),
            app::CommandOutput::Default(value) => print_json(command, value),
        },
        OutputFormat::Text => render_text(output),
    }
}

fn print_json<T: serde::Serialize>(command: &'static str, value: T) {
    let envelope = SuccessEnvelope::new(command, value);
    println!(
        "{}",
        serde_json::to_string(&envelope).expect("launcher output views are serializable")
    );
}

fn render_text(output: app::CommandOutput) {
    match output {
        app::CommandOutput::ConfigPath(view) => println!("{}", view.path.display()),
        app::CommandOutput::ConfigList(view) => {
            for setting in view.settings {
                render_config_entry(&setting);
            }
        }
        app::CommandOutput::ConfigEntry(view) => render_config_entry(&view),
        app::CommandOutput::ConfigMutation(view) => {
            let current = view.current.as_deref().unwrap_or("<unset>");
            let previous = view.previous.as_deref().unwrap_or("<unset>");
            let source = if view.explicit { "explicit" } else { "default" };
            println!("{} = {} ({source}; was {previous})", view.key, current);
        }
        app::CommandOutput::InstanceList(view) => {
            if view.instances.is_empty() {
                println!("No launcher instances are registered.");
            } else {
                for instance in view.instances {
                    let default = if instance.is_default {
                        " [default]"
                    } else {
                        ""
                    };
                    println!(
                        "{}  {}  {}  {}{}",
                        instance.id,
                        instance.name,
                        instance.kind,
                        instance.root.display(),
                        default
                    );
                }
            }
        }
        app::CommandOutput::InstanceDetail(view) => {
            println!("{} ({})", view.instance.name, view.instance.id);
            println!("  root: {}", view.instance.root.display());
            println!("  kind: {}", view.instance.kind);
            println!("  context: {}", view.context.as_str());
            println!("  Minecraft: {}", view.desired.minecraft);
            let loader_version = view.desired.loader_version.as_deref().unwrap_or("n/a");
            println!("  loader: {} {}", view.desired.loader, loader_version);
            println!("  Java: {}", view.desired.java_policy);
        }
        app::CommandOutput::InstanceMutation(view) => {
            println!(
                "{} instance '{}' ({}) at {}",
                view.action.as_str(),
                view.instance.name,
                view.instance.id,
                view.instance.root.display()
            );
            if view.action == output::InstanceMutationAction::Removed {
                println!("Instance files were preserved.");
            }
        }
        app::CommandOutput::Rename(view) => {
            println!(
                "Renamed instance '{}' to '{}' ({}).",
                view.old_name, view.new_name, view.id
            );
        }
        app::CommandOutput::Default(view) => match view.instance {
            Some(instance) => println!("Default instance: {} ({})", instance.name, instance.id),
            None => println!("No default instance is configured."),
        },
    }
}

fn render_config_entry(view: &output::ConfigEntryView) {
    let value = view.value.as_deref().unwrap_or("<unset>");
    let source = if view.explicit { "explicit" } else { "default" };
    println!("{} = {} [{source}]", view.key, value);
}

fn render_error(format: OutputFormat, command: &str, code: &str, message: &str) -> ExitCode {
    match format {
        OutputFormat::Json => {
            let envelope = ErrorEnvelope::new(command, code, message);
            eprintln!(
                "{}",
                serde_json::to_string(&envelope).expect("launcher error view is serializable")
            );
        }
        OutputFormat::Text => eprintln!("error: {message}"),
    }
    match code {
        "argument" => ExitCode::from(2),
        _ => ExitCode::from(1),
    }
}
