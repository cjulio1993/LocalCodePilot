use std::path::PathBuf;
use sysinfo::System;

pub mod filesystem;
pub mod installations;
pub mod languages;
pub mod ports;
pub mod toolchains;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperatingSystem {
    Windows,
    Linux,
    Macos,
    Unknown,
}

#[derive(Debug, Clone, Copy)]
pub struct SystemSnapshot {
    pub used_memory_bytes: u64,
    pub total_memory_bytes: u64,
}

pub trait Platform {
    fn operating_system(&self) -> OperatingSystem;
    fn home_directory(&self) -> Option<PathBuf>;
    fn snapshot(&mut self) -> SystemSnapshot;
}

pub struct NativePlatform {
    /// `None` when the operating system refused the query; memory then reads as zero.
    system: Option<System>,
}

impl Default for NativePlatform {
    fn default() -> Self {
        Self {
            system: System::new().ok(),
        }
    }
}

impl Platform for NativePlatform {
    fn operating_system(&self) -> OperatingSystem {
        if cfg!(target_os = "windows") {
            OperatingSystem::Windows
        } else if cfg!(target_os = "linux") {
            OperatingSystem::Linux
        } else if cfg!(target_os = "macos") {
            OperatingSystem::Macos
        } else {
            OperatingSystem::Unknown
        }
    }

    fn home_directory(&self) -> Option<PathBuf> {
        std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)
    }

    fn snapshot(&mut self) -> SystemSnapshot {
        let Some(system) = self.system.as_mut() else {
            return SystemSnapshot {
                used_memory_bytes: 0,
                total_memory_bytes: 0,
            };
        };
        system.refresh_memory();
        SystemSnapshot {
            used_memory_bytes: system.used_memory(),
            total_memory_bytes: system.total_memory(),
        }
    }
}
pub mod containers;

// TEMPORARIO (T015): aviso de proposito so no Windows, para provar que o CI bloqueia. Sera revertido.
#[cfg(windows)]
#[allow(dead_code)]
fn ci_lint_probe(value: bool) -> bool {
    if !value {
        return false;
    }
    true
}
