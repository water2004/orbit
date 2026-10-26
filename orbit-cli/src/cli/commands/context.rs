use anyhow::{Context, Result};
use std::path::PathBuf;

/// 全局 CLI 上下文，传递给所有命令 handler。
#[derive(Debug, Clone)]
pub struct CliContext {
    pub command: &'static str,
    pub machine_sequence: std::sync::Arc<std::sync::atomic::AtomicU64>,
    pub verbose: bool,
    pub quiet: bool,
    pub yes: bool,
    pub dry_run: bool,
    pub instance: Option<String>,
    pub workspace: std::sync::Arc<std::sync::OnceLock<orbit_core::WorkspaceContext>>,
    pub language: orbit_i18n::LanguageMode,
    pub runtime: orbit_core::RuntimeContext,
    /// 输出格式与进度协议，由全局 `--output-format` / `--progress-format` 决定。
    pub output: crate::cli::output::OutputCfg,
}

impl CliContext {
    pub fn print_result(&self, arguments: std::fmt::Arguments<'_>) {
        if !self.quiet {
            print!("{arguments}");
        }
    }

    pub fn print_result_line(&self, arguments: std::fmt::Arguments<'_>) {
        if !self.quiet {
            println!("{arguments}");
        }
    }

    pub fn print_information_line(&self, arguments: std::fmt::Arguments<'_>) {
        if !self.quiet {
            eprintln!("{arguments}");
        }
    }

    pub fn print_json<T: serde::Serialize>(&self, view: &T) {
        if !self.quiet {
            crate::cli::output::print_json(self.command, view);
        }
    }

    pub fn print_verbose_runtime(&self) {
        if !self.verbose || self.quiet {
            return;
        }
        let config = self.runtime.config();
        eprintln!(
            "{}",
            tr!(
                "Runtime context: config %{config}; JAR cache %{cache}; version repository %{repository}; network timeout %{timeout}s, %{retries} retries; %{downloads} shared downloads",
                config = self.runtime.paths().config_file().display(),
                cache = self.runtime.paths().cache_dir().display(),
                repository = self.runtime.paths().repository_dir().display(),
                timeout = config.network.timeout,
                retries = config.network.max_retries,
                downloads = config.core.max_concurrent_downloads
            )
        );
    }

    pub fn workspace_context(&self) -> Result<&orbit_core::WorkspaceContext> {
        if let Some(context) = self.workspace.get() {
            return Ok(context);
        }
        let current_dir = std::env::current_dir()
            .with_context(|| tr!("Failed to get the current directory").into_owned())?;
        let resolved = orbit_core::resolve_workspace_context(
            self.runtime.paths().instances_file(),
            self.instance.as_deref(),
            &current_dir,
        )
        .map_err(localize_workspace_error)?;
        if self.verbose && !self.quiet {
            eprintln!(
                "{}",
                tr!(
                    "Resolved instance directory: %{path}",
                    path = resolved.directory.display()
                )
            );
        }
        // All cloned presentation callbacks share the command's immutable target.
        let _ = self.workspace.set(resolved);
        Ok(self.workspace.get().expect("workspace was just resolved"))
    }

    pub fn instance_dir(&self) -> Result<PathBuf> {
        Ok(self.workspace_context()?.directory.clone())
    }

    pub fn require_explicit_mutation_target(&self) -> Result<()> {
        self.workspace_context()?
            .require_mutation_target()
            .map_err(localize_workspace_error)
    }
}

fn localize_workspace_error(error: orbit_core::WorkspaceContextError) -> anyhow::Error {
    use orbit_core::WorkspaceContextError::*;
    match error {
        Registry(error) => error.into(),
        UnknownInstance(name) => anyhow::anyhow!(
            "{}",
            tr!(
                "Unknown instance '%{name}'. Run 'orbit instance list' to see registered instances.",
                name = name
            )
        ),
        ImplicitMutation { name, path } => anyhow::anyhow!(
            "{}",
            tr!(
                "Refusing to modify default instance '%{name}' outside its project directory; pass --instance '%{name}' or change to %{path}",
                name = name,
                path = path.display()
            )
        ),
    }
}
