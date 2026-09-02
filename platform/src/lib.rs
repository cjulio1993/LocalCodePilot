use std::path::PathBuf;
use sysinfo::System;

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
    system: System,
}

impl Default for NativePlatform {
    fn default() -> Self {
        Self {
            system: System::new(),
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
        self.system.refresh_memory();
        SystemSnapshot {
            used_memory_bytes: self.system.used_memory(),
            total_memory_bytes: self.system.total_memory(),
        }
    }
}
