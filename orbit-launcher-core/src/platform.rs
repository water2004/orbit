use crate::error::LauncherError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperatingSystem {
    Windows,
    Linux,
    MacOs,
}

impl OperatingSystem {
    pub const fn mojang_name(self) -> &'static str {
        match self {
            Self::Windows => "windows",
            Self::Linux => "linux",
            Self::MacOs => "osx",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Architecture {
    X86,
    X86_64,
    Arm64,
}

impl Architecture {
    pub const fn rule_name(self) -> &'static str {
        match self {
            Self::X86 => "x86",
            Self::X86_64 => "x86_64",
            Self::Arm64 => "aarch64",
        }
    }

    pub const fn bits(self) -> &'static str {
        match self {
            Self::X86 => "32",
            Self::X86_64 | Self::Arm64 => "64",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostPlatform {
    pub os: OperatingSystem,
    pub architecture: Architecture,
    pub os_version: String,
}

impl HostPlatform {
    pub fn native() -> Result<Self, LauncherError> {
        let os = match std::env::consts::OS {
            "windows" => OperatingSystem::Windows,
            "linux" => OperatingSystem::Linux,
            "macos" => OperatingSystem::MacOs,
            value => {
                return Err(LauncherError::UnsupportedRequirement(format!(
                    "operating system '{value}' is unsupported"
                )));
            }
        };
        let architecture = match std::env::consts::ARCH {
            "x86" => Architecture::X86,
            "x86_64" => Architecture::X86_64,
            "aarch64" => Architecture::Arm64,
            value => {
                return Err(LauncherError::UnsupportedRequirement(format!(
                    "architecture '{value}' is unsupported"
                )));
            }
        };
        let os_version = os_info::get().version().to_string();
        if os_version.trim().is_empty() {
            return Err(LauncherError::UnsupportedRequirement(
                "the operating-system version could not be determined for Mojang rule evaluation"
                    .to_string(),
            ));
        }
        Ok(Self {
            os,
            architecture,
            os_version,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mojang_names_and_native_width_are_explicit() {
        assert_eq!(OperatingSystem::MacOs.mojang_name(), "osx");
        assert_eq!(Architecture::X86.bits(), "32");
        assert_eq!(Architecture::Arm64.rule_name(), "aarch64");
    }
}
