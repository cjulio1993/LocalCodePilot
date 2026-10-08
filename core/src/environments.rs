use crate::runtimes::RuntimeKind;
use std::{collections::BTreeMap, path::PathBuf};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Environment {
    values: BTreeMap<String, String>,
}

impl Environment {
    pub fn set(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.values.insert(key.into(), value.into());
    }
    pub fn get(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(String::as_str)
    }
    pub fn values(&self) -> &BTreeMap<String, String> {
        &self.values
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequirementSource {
    Manifest(PathBuf),
    VersionFile(PathBuf),
    Recommended,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionConstraint {
    pub value: String,
    pub recommended: bool,
}

impl VersionConstraint {
    pub fn declared(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            recommended: false,
        }
    }

    pub fn recommended(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            recommended: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequirementStatus {
    Pending,
    Available {
        version: String,
        path: PathBuf,
    },
    Missing,
    Incompatible {
        installed_version: String,
        required_version: String,
        path: PathBuf,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeRequirement {
    pub runtime: RuntimeKind,
    pub command: String,
    pub version: VersionConstraint,
    pub source: RequirementSource,
    pub status: RequirementStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallationStepKind {
    Command {
        program: String,
        args: Vec<String>,
        working_directory: PathBuf,
    },
    RefreshEnvironment,
    VerifyCommand {
        program: String,
        args: Vec<String>,
        version_constraint: Option<String>,
    },
    InstallComposer {
        install_directory: PathBuf,
    },
    ConfigurePhpExtension {
        extension: String,
    },
    CreateCorepackShim {
        manager: String,
        install_directory: PathBuf,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallationStep {
    pub label: String,
    pub detail: String,
    pub source: Option<String>,
    pub requires_elevation: bool,
    pub kind: InstallationStepKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallationPlan {
    pub project_path: PathBuf,
    pub title: String,
    pub requirements: Vec<RuntimeRequirement>,
    pub steps: Vec<InstallationStep>,
}

impl InstallationPlan {
    pub fn requires_elevation(&self) -> bool {
        self.steps.iter().any(|step| step.requires_elevation)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallationResult {
    pub completed_steps: usize,
    pub total_steps: usize,
    pub message: String,
}
