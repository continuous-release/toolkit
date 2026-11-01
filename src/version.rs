use crate::git::{run_git, try_git};
use anyhow::{bail, Result};
use semver::{BuildMetadata, Prerelease, Version};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConventionalCommit {
    pub hash: String,
    pub subject: String,
    pub body: String,
    pub kind: Option<String>,
    pub scope: Option<String>,
    pub breaking: bool,
    pub description: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bump {
    Major,
    Minor,
    Patch,
    None,
}

impl Bump {
    pub fn as_str(self) -> &'static str {
        match self {
            Bump::Major => "major",
            Bump::Minor => "minor",
            Bump::Patch => "patch",
            Bump::None => "none",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NextVersion {
    pub version: String,
    pub tag: String,
    pub previous_tag: Option<String>,
    pub bump: Bump,
    pub existing: bool,
    pub release_as: Option<String>,
}

/// Parse one commit message. Non-conventional subjects are kept as ordinary
/// changes rather than rejected, because every commit on main releases.
pub fn parse_commit(hash: &str, subject: &str, body: &str) -> ConventionalCommit {
    let message = if body.is_empty() {
        subject.to_string()
    } else {
        format!("{subject}\n\n{body}")
    };

    match git_conventional::Commit::parse(&message) {
        Ok(commit) => ConventionalCommit {
            hash: hash.to_string(),
            subject: subject.to_string(),
            body: body.to_string(),
            kind: Some(commit.type_().to_string().to_lowercase()),
            scope: commit.scope().map(|scope| scope.to_string()),
            breaking: commit.breaking(),
            description: commit.description().to_string(),
        },
        Err(_) => ConventionalCommit {
            hash: hash.to_string(),
            subject: subject.to_string(),
            body: body.to_string(),
            kind: None,
            scope: None,
            breaking: false,
            description: subject.to_string(),
        },
    }
}

pub fn read_commits(range: &str, cwd: &Path) -> Result<Vec<ConventionalCommit>> {
    let raw = run_git(&["log", range, "--format=%H%x1f%s%x1f%b%x1e"], cwd)?;

    Ok(raw
        .split('\x1e')
        .map(str::trim)
        .filter(|record| !record.is_empty())
        .map(|record| {
            let mut parts = record.splitn(3, '\x1f');
            let hash = parts.next().unwrap_or_default();
            let subject = parts.next().unwrap_or_default();
            let body = parts.next().unwrap_or_default();
            parse_commit(hash, subject, body)
        })
        .collect())
}

pub fn last_tag(cwd: &Path, prefix: &str) -> Option<String> {
    try_git(
        &[
            "describe",
            "--tags",
            "--abbrev=0",
            "--match",
            &format!("{prefix}*"),
        ],
        cwd,
    )
}

pub fn last_tag_before(ref_name: &str, cwd: &Path, prefix: &str) -> Option<String> {
    try_git(
        &[
            "describe",
            "--tags",
            "--abbrev=0",
            "--match",
            &format!("{prefix}*"),
            &format!("{ref_name}^"),
        ],
        cwd,
    )
}

pub fn tags_at_head(cwd: &Path, prefix: &str) -> Vec<String> {
    try_git(
        &[
            "tag",
            "--points-at",
            "HEAD",
            "--list",
            &format!("{prefix}*"),
        ],
        cwd,
    )
    .map(|text| {
        text.lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_string)
            .collect()
    })
    .unwrap_or_default()
}

pub fn list_version_tags(cwd: &Path, prefix: &str) -> Vec<String> {
    try_git(
        &["tag", "--list", &format!("{prefix}*"), "--sort=-v:refname"],
        cwd,
    )
    .map(|text| {
        text.lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_string)
            .collect()
    })
    .unwrap_or_default()
}

fn release_as_from(body: &str) -> Option<String> {
    body.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        if key.trim().eq_ignore_ascii_case("release-as") {
            let value = value.trim();
            if value.is_empty() {
                None
            } else {
                Some(value.to_string())
            }
        } else {
            None
        }
    })
}

fn bump_version(current: &Version, bump: Bump) -> Version {
    let mut next = current.clone();
    next.pre = Prerelease::EMPTY;
    next.build = BuildMetadata::EMPTY;

    match bump {
        Bump::Major => {
            next.major += 1;
            next.minor = 0;
            next.patch = 0;
        }
        Bump::Minor => {
            next.minor += 1;
            next.patch = 0;
        }
        Bump::Patch => {
            // A prerelease becoming stable keeps its patch number.
            if current.pre.is_empty() {
                next.patch += 1;
            }
        }
        Bump::None => {}
    }

    next
}

#[derive(Debug, Clone)]
pub struct ComputeOptions<'a> {
    pub cwd: &'a Path,
    pub release_as: Option<&'a str>,
    pub prefix: &'a str,
}

impl<'a> ComputeOptions<'a> {
    pub fn new(cwd: &'a Path) -> Self {
        Self {
            cwd,
            release_as: None,
            prefix: "v",
        }
    }
}

pub fn compute_next_version(options: &ComputeOptions<'_>) -> Result<NextVersion> {
    let head_tags = tags_at_head(options.cwd, options.prefix);
    let previous_tag = last_tag(options.cwd, options.prefix);

    if let Some(tag) = head_tags.first() {
        let version = tag.strip_prefix(options.prefix).unwrap_or(tag).to_string();
        return Ok(NextVersion {
            version,
            tag: tag.clone(),
            previous_tag,
            bump: Bump::None,
            existing: true,
            release_as: None,
        });
    }

    let range = previous_tag
        .as_ref()
        .map(|tag| format!("{tag}..HEAD"))
        .unwrap_or_else(|| "HEAD".to_string());
    let commits = read_commits(&range, options.cwd)?;

    let release_as = options.release_as.map(str::to_string).or_else(|| {
        commits
            .iter()
            .find_map(|commit| release_as_from(&commit.body))
    });

    if let Some(release_as) = release_as {
        let raw = release_as.trim().trim_start_matches('v');
        let version = Version::parse(raw)
            .map_err(|_| anyhow::anyhow!("Release-As {release_as} is not valid semver"))?;
        return Ok(NextVersion {
            version: version.to_string(),
            tag: format!("{}{}", options.prefix, version),
            previous_tag,
            bump: Bump::None,
            existing: false,
            release_as: Some(release_as),
        });
    }

    let Some(previous_tag) = previous_tag else {
        return Ok(NextVersion {
            version: "0.1.0".to_string(),
            tag: format!("{}0.1.0", options.prefix),
            previous_tag: None,
            bump: Bump::Minor,
            existing: false,
            release_as: None,
        });
    };

    let current = Version::parse(previous_tag.trim_start_matches(options.prefix))
        .map_err(|_| anyhow::anyhow!("Last tag {previous_tag} is not valid semver"))?;

    let breaking = commits.iter().any(|commit| commit.breaking);
    let feature = commits
        .iter()
        .any(|commit| commit.kind.as_deref() == Some("feat"));
    let bump = if breaking {
        Bump::Major
    } else if feature {
        Bump::Minor
    } else {
        Bump::Patch
    };

    let version = bump_version(&current, bump);

    Ok(NextVersion {
        version: version.to_string(),
        tag: format!("{}{}", options.prefix, version),
        previous_tag: Some(previous_tag),
        bump,
        existing: false,
        release_as: None,
    })
}

/// Convenience for callers that only need the repository root.
pub fn repository_root(cwd: &Path) -> Result<PathBuf> {
    let root = run_git(&["rev-parse", "--show-toplevel"], cwd)?;
    Ok(PathBuf::from(root))
}

/// Validate a tag prefix early so an empty prefix cannot match every tag.
pub fn validate_prefix(prefix: &str) -> Result<()> {
    if prefix.is_empty() {
        bail!("tag prefix cannot be empty");
    }
    Ok(())
}
