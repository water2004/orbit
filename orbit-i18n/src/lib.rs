use std::borrow::Cow;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

rust_i18n::i18n!("locales", fallback = "en");

/// User-facing language selection shared by both CLIs and the native GUI.
///
/// `System` is intentionally the default. It is resolved once at CLI startup
/// and whenever the GUI applies its persisted presentation preferences.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LanguageMode {
    #[default]
    #[serde(rename = "system")]
    System,
    #[serde(rename = "en")]
    English,
    #[serde(rename = "zh-CN")]
    SimplifiedChinese,
}

impl LanguageMode {
    pub const ALL: [Self; 3] = [Self::System, Self::English, Self::SimplifiedChinese];

    pub const fn argument(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::English => "en",
            Self::SimplifiedChinese => "zh-CN",
        }
    }

    pub fn effective_locale(self) -> &'static str {
        match self {
            Self::System => system_locale(),
            Self::English => "en",
            Self::SimplifiedChinese => "zh-CN",
        }
    }

    pub fn label(self) -> Cow<'static, str> {
        match self {
            Self::System => text("Follow system"),
            Self::English => text("English"),
            Self::SimplifiedChinese => text("Simplified Chinese"),
        }
    }
}

impl fmt::Display for LanguageMode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.argument())
    }
}

impl FromStr for LanguageMode {
    type Err = LanguageParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "system" => Ok(Self::System),
            "en" | "en-US" | "en_US" => Ok(Self::English),
            "zh-CN" | "zh_CN" | "zh-Hans" | "zh_Hans" => Ok(Self::SimplifiedChinese),
            _ => Err(LanguageParseError(value.to_string())),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanguageParseError(String);

impl fmt::Display for LanguageParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&text_with(
            "unsupported language '%{language}'; expected system, en, or zh-CN",
            [("language", self.0.clone())],
        ))
    }
}

impl std::error::Error for LanguageParseError {}

/// Read an explicit global `--language` override without otherwise parsing the
/// command line. This lets Clap render its own help in the requested language.
/// Missing or invalid values deliberately fall back to `System`; the real Clap
/// parse still reports invalid values through its normal argument error path.
pub fn requested_from_args<I, S>(arguments: I) -> LanguageMode
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    let mut arguments = arguments.into_iter().skip(1).peekable();
    while let Some(argument) = arguments.next() {
        let argument = argument.as_ref().to_string_lossy();
        if let Some(value) = argument.strip_prefix("--language=") {
            return value.parse().unwrap_or_default();
        }
        if argument == "--language" {
            return arguments
                .next()
                .and_then(|value| value.as_ref().to_str().and_then(|value| value.parse().ok()))
                .unwrap_or_default();
        }
    }
    LanguageMode::System
}

/// Resolve and install the process-wide presentation locale.
pub fn install(mode: LanguageMode) -> &'static str {
    let locale = mode.effective_locale();
    rust_i18n::set_locale(locale);
    locale
}

pub fn locale() -> String {
    rust_i18n::locale().to_string()
}

/// Translate a presentation key in the active locale.
///
/// English source strings are deliberately used as keys. That keeps CLI help
/// and presentation-only text localizable at their rendering boundary without
/// leaking localization into core/domain crates.
pub fn text(key: &str) -> Cow<'static, str> {
    let locale = locale();
    text_for_locale(&locale, key)
}

/// Translate without changing process-global state. This is useful for tests
/// and for presentation code that needs to compare two language choices.
pub fn text_for(mode: LanguageMode, key: &str) -> Cow<'static, str> {
    text_for_locale(mode.effective_locale(), key)
}

fn text_for_locale(locale: &str, key: &str) -> Cow<'static, str> {
    let translated = rust_i18n::t!(key.to_string(), locale = locale).into_owned();
    if translated != key {
        return translated.into();
    }

    // Clap intentionally removes a trailing sentence period when deriving
    // short help. Accept that normalization without duplicating every catalog
    // key, while still requiring an actual translation for the alternate key.
    let alternate = if let Some(without_period) = key.strip_suffix('.') {
        without_period.to_string()
    } else {
        format!("{key}.")
    };
    let translated = rust_i18n::t!(alternate.clone(), locale = locale).into_owned();
    if translated != alternate {
        translated.into()
    } else {
        key.to_string().into()
    }
}

/// Translate a template and substitute named `%{name}` placeholders.
///
/// Placeholder names are part of the catalog contract. Values are formatted
/// by the caller so domain types stay outside this presentation-only crate.
pub fn text_with<const N: usize>(key: &str, arguments: [(&str, String); N]) -> String {
    let mut result = text(key).into_owned();
    for (name, value) in arguments {
        result = result.replace(&format!("%{{{name}}}"), &value);
    }
    result
}

#[macro_export]
macro_rules! tr {
    ($key:expr $(,)?) => {
        $crate::text($key)
    };
    ($key:expr, $($name:ident = $value:expr),+ $(,)?) => {
        $crate::text_with(
            $key,
            [$(
                (stringify!($name), format!("{}", $value))
            ),+],
        )
    };
}

fn system_locale() -> &'static str {
    let locale = sys_locale::get_locale().unwrap_or_else(|| "en".to_string());
    let normalized = locale.replace('_', "-").to_ascii_lowercase();
    if normalized == "zh" || normalized.starts_with("zh-cn") || normalized.starts_with("zh-hans") {
        "zh-CN"
    } else {
        "en"
    }
}

#[cfg(feature = "clap")]
pub fn localize_clap(mut command: clap::Command) -> clap::Command {
    use clap::builder::StyledStr;

    fn translated(value: Option<&StyledStr>) -> Option<String> {
        value.map(|value| text(&value.to_string()).into_owned())
    }

    fn visit(mut command: clap::Command, chinese: bool) -> clap::Command {
        if let Some(value) = translated(command.get_about()) {
            command = command.about(value);
        }
        if let Some(value) = translated(command.get_long_about()) {
            command = command.long_about(value);
        }
        if let Some(value) = translated(command.get_before_help()) {
            command = command.before_help(value);
        }
        if let Some(value) = translated(command.get_after_help()) {
            command = command.after_help(value);
        }

        command = command
            .subcommand_help_heading(if chinese { "命令" } else { "Commands" })
            .subcommand_value_name(if chinese { "命令" } else { "COMMAND" })
            .next_help_heading(if chinese { "选项" } else { "Options" })
            .mut_args(|mut argument| {
                let help = argument.get_help().map(ToString::to_string);
                let long_help = argument.get_long_help().map(ToString::to_string);
                let positional = argument.is_positional();
                if let Some(help) = help {
                    argument = argument.help(text(&help).into_owned());
                }
                if let Some(help) = long_help {
                    argument = argument.long_help(text(&help).into_owned());
                }
                if chinese && argument.get_action().takes_values() {
                    // Clap's built-in "Possible values:" prose is not
                    // localizable. Every affected option documents its stable
                    // values in the translated help, so suppress only that
                    // duplicate generated suffix.
                    argument = argument.hide_possible_values(true);
                }
                argument.help_heading(if chinese {
                    if positional { "参数" } else { "选项" }
                } else if positional {
                    "Arguments"
                } else {
                    "Options"
                })
            })
            .mut_subcommands(|subcommand| visit(subcommand, chinese));

        if chinese {
            command.help_template(
                "{before-help}{name} {version}\n{about-with-newline}\n用法：{usage}\n\n{all-args}{after-help}",
            )
        } else {
            command
        }
    }

    let chinese = locale() == "zh-CN";
    // Translate the derive-provided copy before Clap normalizes short help,
    // then build to materialize implicit help/version entries and visit once
    // more for those generated entries.
    command = visit(command, chinese);
    command.build();
    visit(command, chinese)
}

/// Parse a localized command tree and render parse failures in the active
/// presentation language. Machine-facing option names and values stay stable.
#[cfg(feature = "clap")]
pub fn get_matches(command: clap::Command) -> clap::ArgMatches {
    let command = localize_clap(command);
    match command.try_get_matches() {
        Ok(matches) => matches,
        Err(error) if locale() == "zh-CN" => exit_chinese_clap_error(error),
        Err(error) => error.exit(),
    }
}

#[cfg(feature = "clap")]
fn exit_chinese_clap_error(error: clap::Error) -> ! {
    use clap::error::ErrorKind;

    if matches!(
        error.kind(),
        ErrorKind::DisplayHelp
            | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
            | ErrorKind::DisplayVersion
    ) {
        error.exit();
    }

    eprintln!("{}", render_chinese_clap_error(&error));
    std::process::exit(error.exit_code());
}

#[cfg(feature = "clap")]
fn render_chinese_clap_error(error: &clap::Error) -> String {
    use clap::error::{ContextKind, ContextValue, ErrorKind};
    use std::error::Error as _;

    fn value(error: &clap::Error, kind: ContextKind) -> Option<String> {
        error.get(kind).map(ToString::to_string)
    }

    fn values(error: &clap::Error, kind: ContextKind) -> Option<Vec<String>> {
        match error.get(kind) {
            Some(ContextValue::Strings(values)) => Some(values.clone()),
            Some(ContextValue::String(value)) => Some(vec![value.clone()]),
            _ => None,
        }
    }

    let argument = value(error, ContextKind::InvalidArg).unwrap_or_default();
    let subcommand = value(error, ContextKind::InvalidSubcommand).unwrap_or_default();
    let invalid_value = value(error, ContextKind::InvalidValue).unwrap_or_default();
    let message = match error.kind() {
        ErrorKind::ArgumentConflict => {
            let subject = if !argument.is_empty() {
                text_with("argument '%{argument}'", [("argument", argument.clone())])
            } else if !subcommand.is_empty() {
                text_with(
                    "subcommand '%{subcommand}'",
                    [("subcommand", subcommand.clone())],
                )
            } else {
                text("this command-line item").into_owned()
            };
            let conflicts = values(error, ContextKind::PriorArg).unwrap_or_default();
            if conflicts.len() == 1 && conflicts.first() == Some(&argument) {
                text_with(
                    "%{subject} cannot be used multiple times",
                    [("subject", subject)],
                )
            } else if conflicts.is_empty() {
                text_with(
                    "%{subject} conflicts with another supplied option",
                    [("subject", subject)],
                )
            } else {
                text_with(
                    "%{subject} cannot be used with %{conflicts}",
                    [("subject", subject), ("conflicts", conflicts.join(", "))],
                )
            }
        }
        ErrorKind::NoEquals => text_with(
            "an equals sign is required when assigning a value to '%{argument}'",
            [("argument", argument)],
        ),
        ErrorKind::InvalidValue if invalid_value.is_empty() => text_with(
            "a value is required for '%{argument}' but none was supplied",
            [("argument", argument)],
        ),
        ErrorKind::InvalidValue => text_with(
            "invalid value '%{value}' for '%{argument}'",
            [("value", invalid_value), ("argument", argument)],
        ),
        ErrorKind::InvalidSubcommand => text_with(
            "unrecognized subcommand '%{subcommand}'",
            [("subcommand", subcommand)],
        ),
        ErrorKind::MissingRequiredArgument => text_with(
            "the following required arguments were not provided: %{arguments}",
            [(
                "arguments",
                values(error, ContextKind::InvalidArg)
                    .unwrap_or_default()
                    .join(", "),
            )],
        ),
        ErrorKind::MissingSubcommand => text_with(
            "'%{command}' requires a subcommand but one was not provided",
            [("command", subcommand)],
        ),
        ErrorKind::InvalidUtf8 => text("invalid UTF-8 was detected in an argument").into_owned(),
        ErrorKind::TooManyValues => text_with(
            "unexpected value '%{value}' for '%{argument}'; no more values were expected",
            [("value", invalid_value), ("argument", argument)],
        ),
        ErrorKind::TooFewValues => text_with(
            "'%{argument}' requires at least %{expected} values, but %{actual} were provided",
            [
                ("argument", argument),
                (
                    "expected",
                    value(error, ContextKind::MinValues).unwrap_or_default(),
                ),
                (
                    "actual",
                    value(error, ContextKind::ActualNumValues).unwrap_or_default(),
                ),
            ],
        ),
        ErrorKind::ValueValidation => {
            let detail = error
                .source()
                .map(ToString::to_string)
                .unwrap_or_else(|| text("the value failed validation").into_owned());
            text_with(
                "invalid value '%{value}' for '%{argument}': %{detail}",
                [
                    ("value", invalid_value),
                    ("argument", argument),
                    ("detail", detail),
                ],
            )
        }
        ErrorKind::WrongNumberOfValues => text_with(
            "'%{argument}' requires %{expected} values, but %{actual} were provided",
            [
                ("argument", argument),
                (
                    "expected",
                    value(error, ContextKind::ExpectedNumValues).unwrap_or_default(),
                ),
                (
                    "actual",
                    value(error, ContextKind::ActualNumValues).unwrap_or_default(),
                ),
            ],
        ),
        ErrorKind::UnknownArgument => text_with(
            "unexpected argument '%{argument}'",
            [("argument", argument)],
        ),
        ErrorKind::Io | ErrorKind::Format => error
            .source()
            .map(ToString::to_string)
            .unwrap_or_else(|| text("command-line output failed").into_owned()),
        _ => text("invalid command-line arguments").into_owned(),
    };

    let mut rendered = text_with("error: %{message}", [("message", message)]);
    if let Some(possible) = values(error, ContextKind::ValidValue)
        .or_else(|| values(error, ContextKind::ValidSubcommand))
        .filter(|values| !values.is_empty())
    {
        rendered.push_str("\n  ");
        rendered.push_str(&text_with(
            "Possible values: %{values}",
            [("values", possible.join(", "))],
        ));
    }
    for (kind, label) in [
        (
            ContextKind::SuggestedSubcommand,
            "Similar subcommand: %{value}",
        ),
        (ContextKind::SuggestedArg, "Similar argument: %{value}"),
        (ContextKind::SuggestedValue, "Similar value: %{value}"),
    ] {
        if let Some(suggestion) = value(error, kind).filter(|value| !value.is_empty()) {
            rendered.push_str("\n  ");
            rendered.push_str(&text_with(label, [("value", suggestion)]));
        }
    }
    if let Some(usage) = value(error, ContextKind::Usage) {
        let usage = usage
            .strip_prefix("Usage: ")
            .or_else(|| usage.strip_prefix("用法："))
            .unwrap_or(&usage);
        rendered.push_str("\n\n");
        rendered.push_str(&text_with(
            "Usage: %{usage}",
            [("usage", usage.to_string())],
        ));
    }
    rendered.push('\n');
    rendered.push_str(&text("For more information, try '--help'."));
    rendered
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_languages_are_stable() {
        assert_eq!(LanguageMode::English.effective_locale(), "en");
        assert_eq!(LanguageMode::SimplifiedChinese.effective_locale(), "zh-CN");
        assert_eq!("zh-CN".parse(), Ok(LanguageMode::SimplifiedChinese));
    }

    #[test]
    fn chinese_catalog_is_available() {
        assert_eq!(
            text_for(LanguageMode::SimplifiedChinese, "Follow system"),
            "跟随系统"
        );
        assert_eq!(
            text_for(LanguageMode::SimplifiedChinese, "Settings"),
            "设置"
        );
        assert_eq!(text_for(LanguageMode::English, "Settings"), "Settings");
    }

    #[test]
    fn named_template_values_survive_reordering() {
        let template = text_for(
            LanguageMode::SimplifiedChinese,
            "Installed %{kind} instance %{id}.",
        );
        assert_eq!(
            template
                .replace("%{kind}", "client")
                .replace("%{id}", "demo"),
            "已安装 client 实例 demo。"
        );
    }

    #[test]
    fn command_line_override_is_optional_and_global() {
        assert_eq!(
            requested_from_args(["orbit", "install"]),
            LanguageMode::System
        );
        assert_eq!(
            requested_from_args(["orbit", "install", "--language", "zh-CN"]),
            LanguageMode::SimplifiedChinese
        );
        assert_eq!(
            requested_from_args(["orbit", "--language=en", "install"]),
            LanguageMode::English
        );
    }

    #[cfg(feature = "clap")]
    #[test]
    fn chinese_clap_localization_accepts_flags_and_value_options() {
        use clap::{Arg, ArgAction, Command};

        install(LanguageMode::SimplifiedChinese);
        let command = Command::new("orbit-test")
            .arg(
                Arg::new("verbose")
                    .long("verbose")
                    .action(ArgAction::SetTrue),
            )
            .arg(
                Arg::new("language")
                    .long("language")
                    .value_parser(["system", "en", "zh-CN"]),
            );

        let command = localize_clap(command);
        command.clone().debug_assert();

        let unknown = command
            .clone()
            .try_get_matches_from(["orbit-test", "--unknown"])
            .expect_err("unknown option must fail");
        let rendered = render_chinese_clap_error(&unknown);
        assert!(rendered.contains("错误：无法识别参数“--unknown”"));
        assert!(rendered.contains("更多信息请使用“--help”"));

        let invalid = command
            .try_get_matches_from(["orbit-test", "--language", "invalid"])
            .expect_err("invalid stable language value must fail");
        let rendered = render_chinese_clap_error(&invalid);
        assert!(rendered.contains("可用值：system, en, zh-CN"));
        install(LanguageMode::English);
    }
}
