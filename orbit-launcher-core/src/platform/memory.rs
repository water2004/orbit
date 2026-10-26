//! Host memory facts and presentation ranges are Launcher-owned. This is a
//! reference range, not a promise that Java can allocate all physical memory.
use crate::{LauncherError, instance::MemoryConfig};

#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct MemoryCapabilities {
    pub physical_memory_mib: u64,
    pub slider_max_mib: u32,
}
impl MemoryCapabilities {
    pub fn native(memory: MemoryConfig) -> Result<Self, LauncherError> {
        memory.validate()?;
        Self::from_total(native_total_bytes()? / (1024 * 1024), memory)
    }
    fn from_total(total: u64, memory: MemoryConfig) -> Result<Self, LauncherError> {
        if total == 0 {
            return Err(LauncherError::UnsupportedRequirement(
                "physical memory capacity is unavailable".into(),
            ));
        }
        Ok(Self {
            physical_memory_mib: total,
            slider_max_mib: u32::try_from(total).unwrap_or(u32::MAX).max(memory.max_mib),
        })
    }
}

#[cfg(windows)]
fn native_total_bytes() -> Result<u64, LauncherError> {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let mut status = MEMORYSTATUSEX {
        dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
        ..Default::default()
    };
    // SAFETY: status is initialized to the documented struct size and writable
    // for the entire call. No pointers escape.
    if unsafe { GlobalMemoryStatusEx(&mut status) } == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(status.ullTotalPhys)
}

#[cfg(target_os = "linux")]
fn native_total_bytes() -> Result<u64, LauncherError> {
    parse_linux_total(&std::fs::read_to_string("/proc/meminfo")?)
}
#[cfg(any(target_os = "linux", test))]
fn parse_linux_total(text: &str) -> Result<u64, LauncherError> {
    text.lines()
        .find_map(|line| {
            let mut fields = line.split_whitespace();
            if fields.next()? != "MemTotal:" {
                return None;
            }
            let size = fields.next()?.parse::<u64>().ok()?;
            (fields.next()? == "kB" && fields.next().is_none())
                .then(|| size.checked_mul(1024))
                .flatten()
        })
        .ok_or_else(|| {
            LauncherError::UnsupportedRequirement(
                "/proc/meminfo does not contain a valid MemTotal value".into(),
            )
        })
}

#[cfg(target_os = "macos")]
fn native_total_bytes() -> Result<u64, LauncherError> {
    let mut total = 0_u64;
    let mut length = std::mem::size_of_val(&total);
    // SAFETY: the read-only sysctl name is NUL terminated. The writable buffer
    // has exactly length bytes and its returned length is checked.
    if unsafe {
        libc::sysctlbyname(
            c"hw.memsize".as_ptr(),
            (&mut total as *mut u64).cast(),
            &mut length,
            std::ptr::null_mut(),
            0,
        )
    } != 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    if length != std::mem::size_of_val(&total) {
        return Err(LauncherError::UnsupportedRequirement(
            "hw.memsize returned an invalid size".into(),
        ));
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn linux_memory_units_and_range_are_not_guessed() {
        assert_eq!(
            parse_linux_total("MemTotal: 16777216 kB\nMemFree: 42 kB").unwrap(),
            16 * 1024 * 1024 * 1024
        );
        for bad in [
            "MemFree: 42 kB",
            "MemTotal: 42 MB",
            "MemTotal: 18446744073709551615 kB",
        ] {
            assert!(parse_linux_total(bad).is_err());
        }
        let memory = MemoryConfig {
            min_mib: 512,
            max_mib: 32768,
        };
        assert_eq!(
            MemoryCapabilities::from_total(8192, memory)
                .unwrap()
                .slider_max_mib,
            32768
        );
        assert!(MemoryCapabilities::from_total(0, memory).is_err());
    }
    #[test]
    fn native_memory_is_available_without_running_a_game() {
        assert!(
            MemoryCapabilities::native(MemoryConfig::default())
                .unwrap()
                .physical_memory_mib
                > 0
        );
    }
}
