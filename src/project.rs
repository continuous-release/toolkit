//! Independent per-project versioning configuration.
//!
//! The config is deliberately build-system neutral: `cr.json` names projects,
//! their paths, tag prefixes, optional version-file adapters, and artifacts.
//! A project only releases when a commit since its last tag touched one of its
//! paths (or a shared path).

use crate::adapters::{get_adapter, Adapter};
use crate::version::validate_prefix;
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};

/// The on-disk `cr.json` shape.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectsConfig {
    /// Only independent versioning is supported. Omitted or `true` is accepted;
    /// an explicit `false` is rejected rather than silently ignored.
    #[serde(default)]
    pub independent: Option<bool>,
    #[serde(default)]
    pub shared_paths: Vec<String>,
    #[serde(default)]
    pub projects: BTreeMap<String, ProjectConfig>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectConfig {
    pub path: String,
    #[serde(default)]
    pub tag_prefix: Option<String>,
    #[serde(default)]
    pub adapter: Option<String>,
    #[serde(default)]
    pub file: Option<String>,
    #[serde(default)]
    pub artifacts: Vec<String>,
    #[serde(default)]
    pub paths: Vec<String>,
}

/// A validated project, with paths resolved for reporting and filesystem use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    pub name: String,
    /// Normalized repository-relative directory, such as `packages/core`.
    pub relative_path: String,
    /// Absolute directory the project lives in.
    pub path: PathBuf,
    pub tag_prefix: String,
    pub adapter: Option<Adapter>,
    /// Adapter file relative to the project directory.
    pub file: Option<String>,
    /// Artifact globs relative to the project directory.
    pub artifacts: Vec<String>,
    /// Repository-relative path filters, including the project path by default.
    pub paths: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ProjectSet {
    pub config_path: PathBuf,
    pub root: PathBuf,
    pub shared_paths: Vec<String>,
    pub projects: Vec<Project>,
}

impl ProjectSet {
    /// Load and validate a config file. `root` is the repository root that all
    /// project and path entries are relative to.
    pub fn load(path: &Path, root: &Path) -> Result<Self> {
        let text = fs::read_to_string(path)
            .with_context(|| format!("reading project config {}", path.display()))?;
        let config: ProjectsConfig = serde_json::from_str(&text)
            .with_context(|| format!("parsing project config {}", path.display()))?;
        Self::from_config(config, path, root)
    }

    pub fn from_config(config: ProjectsConfig, path: &Path, root: &Path) -> Result<Self> {
        if config.independent == Some(false) {
            bail!(
                "{}: independent versioning is required; set \"independent\" to true or omit it",
                path.display()
            );
        }
        if config.projects.is_empty() {
            bail!("{}: projects cannot be empty", path.display());
        }

        let mut shared_paths = normalize_paths(&config.shared_paths, "sharedPaths")?;
        dedupe(&mut shared_paths);

        let mut projects = Vec::with_capacity(config.projects.len());
        for (name, project) in config.projects {
            validate_name(&name)?;
            let relative_path = normalize_path(&project.path, "path")?;
            let absolute = root.join(&relative_path);
            if !absolute.is_dir() {
                bail!(
                    "{}: project {name} path {relative_path} does not exist or is not a directory",
                    path.display()
                );
            }

            let tag_prefix = project
                .tag_prefix
                .clone()
                .unwrap_or_else(|| format!("{name}-v"));
            validate_prefix(&tag_prefix)
                .with_context(|| format!("{}: project {name} tagPrefix", path.display()))?;

            let adapter = match project.adapter.as_deref() {
                Some(adapter) => Some(
                    get_adapter(adapter)
                        .with_context(|| format!("{}: project {name} adapter", path.display()))?,
                ),
                None => None,
            };
            if project.file.is_some() && adapter.is_none() {
                bail!(
                    "{}: project {name} sets file without an adapter",
                    path.display()
                );
            }
            let file = match project.file.as_deref() {
                Some(file) => Some(
                    normalize_file(file, "file")
                        .with_context(|| format!("{}: project {name} file", path.display()))?,
                ),
                None => None,
            };

            let mut paths = if project.paths.is_empty() {
                vec![relative_path.clone()]
            } else {
                normalize_paths(&project.paths, "paths")
                    .with_context(|| format!("{}: project {name}", path.display()))?
            };
            dedupe(&mut paths);

            let mut artifacts = Vec::with_capacity(project.artifacts.len());
            for artifact in &project.artifacts {
                artifacts.push(
                    normalize_path(artifact, "artifacts")
                        .with_context(|| format!("{}: project {name}", path.display()))?,
                );
            }
            dedupe(&mut artifacts);

            projects.push(Project {
                name,
                relative_path,
                path: absolute,
                tag_prefix,
                adapter,
                file,
                artifacts,
                paths,
            });
        }

        projects.sort_by(|left, right| left.name.cmp(&right.name));

        for (index, project) in projects.iter().enumerate() {
            for other in projects.iter().skip(index + 1) {
                if project.tag_prefix.starts_with(&other.tag_prefix)
                    || other.tag_prefix.starts_with(&project.tag_prefix)
                {
                    bail!(
                        "{}: tag prefixes {:?} ({}) and {:?} ({}) overlap",
                        path.display(),
                        project.tag_prefix,
                        project.name,
                        other.tag_prefix,
                        other.name
                    );
                }
            }
        }

        Ok(Self {
            config_path: path.to_path_buf(),
            root: root.to_path_buf(),
            shared_paths,
            projects,
        })
    }

    /// Select projects by name, preserving config order. No names means every
    /// project. Duplicate names are harmless; unknown names are an error.
    pub fn select(&self, names: &[String]) -> Result<Vec<Project>> {
        if names.is_empty() {
            return Ok(self.projects.clone());
        }

        let mut wanted: Vec<&str> = Vec::new();
        for name in names {
            if !self.projects.iter().any(|project| &project.name == name) {
                let available: Vec<&str> = self
                    .projects
                    .iter()
                    .map(|project| project.name.as_str())
                    .collect();
                bail!(
                    "unknown project {:?}; available projects: {}",
                    name,
                    available.join(", ")
                );
            }
            if !wanted.contains(&name.as_str()) {
                wanted.push(name);
            }
        }

        Ok(self
            .projects
            .iter()
            .filter(|project| wanted.contains(&project.name.as_str()))
            .cloned()
            .collect())
    }

    /// Path filters for one project: its own paths plus shared paths.
    pub fn filters_for(&self, project: &Project) -> Vec<String> {
        let mut paths = project.paths.clone();
        paths.extend(self.shared_paths.iter().cloned());
        dedupe(&mut paths);
        paths
    }
}

fn validate_name(name: &str) -> Result<()> {
    if name.is_empty() {
        bail!("project names cannot be empty");
    }
    if let Some(character) = name
        .chars()
        .find(|character| character.is_whitespace() || character.is_control())
    {
        bail!(
            "project name {name:?} cannot contain whitespace or control characters ({character:?})"
        );
    }
    if name.contains('/') || name.contains('\\') {
        bail!("project name {name:?} cannot contain a path separator");
    }
    if name.contains(':') {
        bail!("project name {name:?} cannot contain ':'");
    }
    if name.contains(',') {
        bail!("project name {name:?} cannot contain ','");
    }
    if name.starts_with('-') || name.starts_with('.') {
        bail!("project name {name:?} cannot start with '-' or '.'");
    }
    Ok(())
}

fn normalize_paths(values: &[String], field: &str) -> Result<Vec<String>> {
    values
        .iter()
        .map(|value| normalize_path(value, field))
        .collect()
}

/// Normalize a repository-relative path without allowing escapes or pathspec
/// magic. Glob characters are intentionally left intact for artifact patterns.
fn normalize_path(value: &str, field: &str) -> Result<String> {
    if value.trim() != value {
        bail!("{field} cannot have surrounding whitespace: {value:?}");
    }
    if value.is_empty() {
        bail!("{field} cannot be empty");
    }

    let path = Path::new(value);
    if path.is_absolute() {
        bail!("{field} must be relative to the repository root: {value}");
    }

    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => parts.push(part.to_string_lossy().to_string()),
            Component::CurDir => {}
            Component::ParentDir => {
                bail!("{field} cannot contain '..': {value}");
            }
            Component::RootDir | Component::Prefix(_) => {
                bail!("{field} must be relative to the repository root: {value}");
            }
        }
    }

    if parts.is_empty() {
        return Ok(".".to_string());
    }
    let normalized = parts.join("/");
    if normalized.starts_with(':') || normalized.starts_with('!') {
        bail!("{field} cannot start with pathspec magic ':' or '!': {value}");
    }
    Ok(normalized)
}

fn normalize_file(value: &str, field: &str) -> Result<String> {
    let normalized = normalize_path(value, field)?;
    if normalized == "." {
        bail!("{field} cannot be the project root: {value}");
    }
    Ok(normalized)
}

fn dedupe(values: &mut Vec<String>) {
    let mut seen = std::collections::BTreeSet::new();
    values.retain(|value| seen.insert(value.clone()));
}
