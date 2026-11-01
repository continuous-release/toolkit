use anyhow::{bail, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Adapter {
    Npm,
    Cargo,
    Text,
}

impl Adapter {
    pub fn name(self) -> &'static str {
        match self {
            Adapter::Npm => "npm",
            Adapter::Cargo => "cargo",
            Adapter::Text => "text",
        }
    }

    pub fn default_file(self) -> &'static str {
        match self {
            Adapter::Npm => "package.json",
            Adapter::Cargo => "Cargo.toml",
            Adapter::Text => "version.txt",
        }
    }
}

pub fn get_adapter(name: &str) -> Result<Adapter> {
    match name {
        "npm" => Ok(Adapter::Npm),
        "cargo" => Ok(Adapter::Cargo),
        "text" => Ok(Adapter::Text),
        other => bail!("Unknown adapter {other}; expected one of npm, cargo, text"),
    }
}

fn target(cwd: &Path, adapter: Adapter, file: Option<&str>) -> PathBuf {
    cwd.join(file.unwrap_or_else(|| adapter.default_file()))
}

pub fn read_version(cwd: &Path, adapter: Adapter, file: Option<&str>) -> Result<String> {
    let path = target(cwd, adapter, file);
    let text = fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;

    match adapter {
        Adapter::Npm => {
            let value: serde_json::Value = serde_json::from_str(&text)
                .with_context(|| format!("parsing {}", path.display()))?;
            value
                .get("version")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
                .with_context(|| format!("{} has no version", path.display()))
        }
        Adapter::Cargo => {
            let document: toml_edit::DocumentMut = text
                .parse()
                .with_context(|| format!("parsing {}", path.display()))?;
            document["package"]["version"]
                .as_str()
                .map(str::to_string)
                .with_context(|| format!("{} has no [package] version", path.display()))
        }
        Adapter::Text => Ok(text.trim().to_string()),
    }
}

pub fn write_version(
    cwd: &Path,
    adapter: Adapter,
    version: &str,
    file: Option<&str>,
) -> Result<()> {
    let path = target(cwd, adapter, file);
    let text = fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;

    let output = match adapter {
        Adapter::Npm => {
            let mut value: serde_json::Value = serde_json::from_str(&text)
                .with_context(|| format!("parsing {}", path.display()))?;
            let object = value
                .as_object_mut()
                .with_context(|| format!("{} is not a JSON object", path.display()))?;
            object.insert(
                "version".to_string(),
                serde_json::Value::String(version.to_string()),
            );
            format!("{}\n", serde_json::to_string_pretty(&value)?)
        }
        Adapter::Cargo => {
            let mut document: toml_edit::DocumentMut = text
                .parse()
                .with_context(|| format!("parsing {}", path.display()))?;
            if document.get("package").is_none() {
                bail!("{} has no [package] version", path.display());
            }
            document["package"]["version"] = toml_edit::value(version);
            document.to_string()
        }
        Adapter::Text => format!("{version}\n"),
    };

    fs::write(&path, output).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}
