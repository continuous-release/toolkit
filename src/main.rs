use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use continuous_release::adapters::{get_adapter, write_version};
use continuous_release::git::fetch_tags;
use continuous_release::notes::{render_notes, NotesOptions};
use continuous_release::release::{
    CreateRelease, ReleaseClient, ReleaseClientOptions, TagConflictError, UpdateRelease,
};
use continuous_release::version::{
    compute_next_version, last_tag_before, list_version_tags, read_commits, validate_prefix,
    ComputeOptions,
};
use serde_json::json;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, Parser)]
#[command(
    name = "cr",
    version = env!("CR_BUILD_VERSION"),
    about = "Continuous releases from conventional commits"
)]
struct Cli {
    /// Repository directory to operate on
    #[arg(long, global = true)]
    cwd: Option<PathBuf>,

    /// Prefix for version tags
    #[arg(long = "tag-prefix", global = true, default_value = "v")]
    tag_prefix: String,

    /// Force a version instead of deriving one
    #[arg(long = "release-as", global = true)]
    release_as: Option<String>,

    /// Repository in owner/name form
    #[arg(long, global = true)]
    repo: Option<String>,

    /// GitHub token
    #[arg(long, global = true)]
    token: Option<String>,

    /// GitHub API base URL
    #[arg(long = "api-url", global = true)]
    api_url: Option<String>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Derive the next version and render its notes
    Plan {
        /// Write the generated notes to this path
        #[arg(long = "notes-out")]
        notes_out: Option<PathBuf>,

        /// Print the plan as JSON
        #[arg(long)]
        json: bool,
    },

    /// Render notes without reserving or releasing
    Notes {
        /// Write the notes to this path instead of stdout
        #[arg(long)]
        out: Option<PathBuf>,
    },

    /// Write a version through an adapter
    Version {
        /// Version file adapter (npm, cargo, text)
        #[arg(long)]
        adapter: String,

        /// Version file path
        #[arg(long)]
        file: Option<String>,

        /// Version to write (defaults to the next version)
        #[arg(long)]
        version: Option<String>,
    },

    /// Reserve the next version by creating its tag
    Reserve {
        /// Commit to tag
        #[arg(long)]
        target: Option<String>,

        /// Write the generated notes to this path
        #[arg(long = "notes-out")]
        notes_out: Option<PathBuf>,
    },

    /// Drop a reservation that never reached publish
    Unreserve {
        /// Tag to remove
        #[arg(long)]
        tag: String,

        /// Commit the tag must still point at
        #[arg(long)]
        target: Option<String>,
    },

    /// Create or update the GitHub Release and attach assets
    Release {
        /// Release tag (defaults to the next version)
        #[arg(long)]
        tag: Option<String>,

        /// Commit the tag must point at
        #[arg(long)]
        target: Option<String>,

        /// Read release notes from this path
        #[arg(long = "notes-file")]
        notes_file: Option<PathBuf>,

        /// Asset path or glob; repeatable
        #[arg(long)]
        asset: Vec<String>,

        /// Mark the release as a prerelease
        #[arg(long)]
        prerelease: bool,
    },

    /// Check whether a release exists
    Verify {
        /// Release tag (defaults to the next version)
        #[arg(long)]
        tag: Option<String>,
    },
}

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    validate_prefix(&cli.tag_prefix)?;
    let cwd = cli
        .cwd
        .clone()
        .unwrap_or(std::env::current_dir().context("reading the current directory")?);

    match &cli.command {
        Command::Plan { notes_out, json } => plan(&cli, &cwd, notes_out.as_deref(), *json),
        Command::Notes { out } => notes(&cli, &cwd, out.as_deref()),
        Command::Version {
            adapter,
            file,
            version,
        } => write_version_command(&cli, &cwd, adapter, file.as_deref(), version.as_deref()),
        Command::Reserve { target, notes_out } => {
            reserve(&cli, &cwd, target.as_deref(), notes_out.as_deref())
        }
        Command::Unreserve { tag, target } => unreserve(&cli, &cwd, tag, target.as_deref()),
        Command::Release {
            tag,
            target,
            notes_file,
            asset,
            prerelease,
        } => release(
            &cli,
            &cwd,
            tag.as_deref(),
            target.as_deref(),
            notes_file.as_deref(),
            asset,
            *prerelease,
        ),
        Command::Verify { tag } => verify(&cli, &cwd, tag.as_deref()),
    }
}

fn compute(cli: &Cli, cwd: &Path) -> Result<continuous_release::version::NextVersion> {
    compute_next_version(&ComputeOptions {
        cwd,
        release_as: cli.release_as.as_deref(),
        prefix: &cli.tag_prefix,
    })
}

fn repository(cli: &Cli) -> Result<String> {
    cli.repo
        .clone()
        .or_else(|| std::env::var("GITHUB_REPOSITORY").ok())
        .filter(|value| !value.is_empty())
        .context("a repository is required (--repo or GITHUB_REPOSITORY)")
}

fn token(cli: &Cli) -> Result<String> {
    cli.token
        .clone()
        .or_else(|| std::env::var("GITHUB_TOKEN").ok())
        .filter(|value| !value.is_empty())
        .context("a GitHub token is required (--token or GITHUB_TOKEN)")
}

fn target_commit(value: Option<&str>) -> Result<String> {
    value
        .map(str::to_string)
        .or_else(|| std::env::var("GITHUB_SHA").ok())
        .filter(|value| !value.is_empty())
        .context("a target commit is required (--target or GITHUB_SHA)")
}

fn client(cli: &Cli) -> Result<ReleaseClient> {
    ReleaseClient::new(ReleaseClientOptions {
        token: token(cli)?,
        repository: repository(cli)?,
        api_url: cli
            .api_url
            .clone()
            .or_else(|| std::env::var("GITHUB_API_URL").ok()),
    })
}

fn set_output(key: &str, value: &str) {
    let Ok(path) = std::env::var("GITHUB_OUTPUT") else {
        return;
    };
    if let Ok(mut file) = fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{key}={value}");
    }
}

fn resolve(cwd: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    }
}

fn render(cli: &Cli, cwd: &Path, version: &str, previous_tag: Option<&str>) -> Result<String> {
    let range = previous_tag
        .map(|tag| format!("{tag}..HEAD"))
        .unwrap_or_else(|| "HEAD".to_string());
    let commits = read_commits(&range, cwd)?;
    let repository = cli
        .repo
        .clone()
        .or_else(|| std::env::var("GITHUB_REPOSITORY").ok())
        .filter(|value| !value.is_empty());

    Ok(render_notes(&NotesOptions {
        commits: &commits,
        previous_tag,
        version,
        repository: repository.as_deref(),
        tag_prefix: &cli.tag_prefix,
    }))
}

fn plan(cli: &Cli, cwd: &Path, notes_out: Option<&Path>, as_json: bool) -> Result<()> {
    let next = compute(cli, cwd)?;
    let body = render(cli, cwd, &next.version, next.previous_tag.as_deref())?;
    let notes_path = notes_out.map(|path| resolve(cwd, path));
    if let Some(path) = &notes_path {
        fs::write(path, &body).with_context(|| format!("writing {}", path.display()))?;
    }

    set_output("version", &next.version);
    set_output("tag", &next.tag);
    set_output("previous_tag", next.previous_tag.as_deref().unwrap_or(""));
    set_output("existing", if next.existing { "true" } else { "false" });
    set_output("bump", next.bump.as_str());
    if let Some(path) = &notes_path {
        set_output("notes_path", &path.display().to_string());
    }

    if as_json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "version": next.version,
                "tag": next.tag,
                "previousTag": next.previous_tag,
                "bump": next.bump.as_str(),
                "existing": next.existing,
                "releaseAs": next.release_as,
                "notes": body,
                "notesPath": notes_path.map(|path| path.display().to_string()),
            }))?
        );
    } else {
        println!("version: {}", next.version);
        println!("tag: {}", next.tag);
        println!(
            "previous tag: {}",
            next.previous_tag.as_deref().unwrap_or("(none)")
        );
        println!("bump: {}", next.bump.as_str());
        println!("existing: {}", next.existing);
    }

    Ok(())
}

fn notes(cli: &Cli, cwd: &Path, out: Option<&Path>) -> Result<()> {
    let next = compute(cli, cwd)?;
    let body = render(cli, cwd, &next.version, next.previous_tag.as_deref())?;
    match out {
        Some(path) => {
            let path = resolve(cwd, path);
            fs::write(&path, body).with_context(|| format!("writing {}", path.display()))?;
        }
        None => print!("{body}"),
    }
    Ok(())
}

fn write_version_command(
    cli: &Cli,
    cwd: &Path,
    adapter_name: &str,
    file: Option<&str>,
    version: Option<&str>,
) -> Result<()> {
    let adapter = get_adapter(adapter_name)?;
    let version = match version {
        Some(version) => version.to_string(),
        None => compute(cli, cwd)?.version,
    };
    write_version(cwd, adapter, &version, file)?;
    println!(
        "set {} version to {version}{}",
        adapter.name(),
        file.map(|file| format!(" in {file}")).unwrap_or_default()
    );
    Ok(())
}

fn reserve(
    cli: &Cli,
    cwd: &Path,
    target_value: Option<&str>,
    notes_out: Option<&Path>,
) -> Result<()> {
    let client = client(cli)?;
    let target = target_commit(target_value)?;

    for _ in 0..5 {
        let next = compute(cli, cwd)?;
        match client.reserve_tag(&next.tag, &target) {
            Ok(created) => {
                let previous_tag = if next.existing {
                    last_tag_before("HEAD", cwd, &cli.tag_prefix)
                } else {
                    next.previous_tag.clone()
                };
                let body = render(cli, cwd, &next.version, previous_tag.as_deref())?;
                let notes_path = notes_out.map(|path| resolve(cwd, path));
                if let Some(path) = &notes_path {
                    fs::write(path, &body)
                        .with_context(|| format!("writing {}", path.display()))?;
                }

                set_output("version", &next.version);
                set_output("tag", &next.tag);
                set_output("previous_tag", previous_tag.as_deref().unwrap_or(""));
                set_output("existing", if next.existing { "true" } else { "false" });
                set_output("superseded", "false");
                if let Some(path) = &notes_path {
                    set_output("notes_path", &path.display().to_string());
                }

                println!(
                    "{} {} at {}",
                    if created { "reserved" } else { "kept" },
                    next.tag,
                    target
                );
                return Ok(());
            }
            Err(error) => {
                let Some(conflict) = error.downcast_ref::<TagConflictError>() else {
                    return Err(error);
                };
                println!("{} was taken by {}; re-planning", next.tag, conflict.actual);
                fetch_tags(cwd)?;

                let replanned = compute(cli, cwd)?;
                if replanned.tag == next.tag {
                    set_output("superseded", "true");
                    set_output("tag", &next.tag);
                    set_output("existing", "false");
                    println!("superseded by {} at {}", next.tag, conflict.actual);
                    return Ok(());
                }
            }
        }
    }

    bail!("Could not reserve a version after 5 attempts")
}

fn unreserve(cli: &Cli, cwd: &Path, tag: &str, target_value: Option<&str>) -> Result<()> {
    let client = client(cli)?;

    if client.get_release_by_tag(tag)?.is_some() {
        println!("keeping {tag}: a release already exists");
        return Ok(());
    }

    let Some(sha) = client.resolve_tag_commit(tag)? else {
        println!("nothing to unreserve for {tag}");
        return Ok(());
    };

    if let Some(target) = target_value
        .map(str::to_string)
        .or_else(|| std::env::var("GITHUB_SHA").ok())
        .filter(|value| !value.is_empty())
    {
        if sha != target {
            println!("keeping {tag}: it points at {sha}, not {target}");
            return Ok(());
        }
    }

    fetch_tags(cwd)?;
    if let Some(highest) = list_version_tags(cwd, &cli.tag_prefix).first() {
        if highest != tag {
            println!("keeping {tag}: {highest} is newer");
            return Ok(());
        }
    }

    client.delete_tag(tag)?;
    println!("unreserved {tag}");
    Ok(())
}

fn expand_assets(cwd: &Path, patterns: &[String]) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();

    for pattern in patterns {
        let literal = resolve(cwd, Path::new(pattern));
        if literal.is_file() {
            files.push(literal);
            continue;
        }

        let pattern_text = literal.display().to_string();
        let mut matches: Vec<PathBuf> = glob::glob(&pattern_text)
            .with_context(|| format!("invalid asset pattern {pattern}"))?
            .filter_map(Result::ok)
            .collect();
        if matches.is_empty() {
            bail!("Asset pattern matched nothing: {pattern}");
        }
        matches.sort();
        files.extend(matches);
    }

    Ok(files)
}

fn release(
    cli: &Cli,
    cwd: &Path,
    tag_value: Option<&str>,
    target_value: Option<&str>,
    notes_file: Option<&Path>,
    assets: &[String],
    prerelease: bool,
) -> Result<()> {
    let client = client(cli)?;
    let tag = match tag_value {
        Some(tag) => tag.to_string(),
        None => compute(cli, cwd)?.tag,
    };
    let target = target_value
        .map(str::to_string)
        .or_else(|| std::env::var("GITHUB_SHA").ok())
        .filter(|value| !value.is_empty());
    let body = match notes_file {
        Some(path) => fs::read_to_string(resolve(cwd, path))
            .with_context(|| format!("reading {}", path.display()))?,
        None => String::new(),
    };
    let prerelease = prerelease || tag.contains('-');

    match client.resolve_tag_commit(&tag)? {
        Some(tagged) => {
            if let Some(target) = &target {
                if &tagged != target {
                    bail!("tag {tag} points at {tagged}, not {target}");
                }
            }
        }
        None => {
            let target = target
                .as_deref()
                .context("a target commit is required to create the tag")?;
            client.reserve_tag(&tag, target)?;
            println!("reserved {tag} at {target}");
        }
    }

    let existing = client.get_release_by_tag(&tag)?;
    let mut current = match &existing {
        Some(release) => client.update_release(
            release.id,
            &UpdateRelease {
                name: Some(&tag),
                body: Some(&body),
                prerelease: Some(prerelease),
                draft: None,
            },
        )?,
        None => client.create_release(&CreateRelease {
            tag: &tag,
            target: target.as_deref(),
            name: Some(&tag),
            body: &body,
            prerelease,
            draft: true,
        })?,
    };
    println!(
        "{} release {tag}{}",
        if existing.is_some() {
            "updated"
        } else {
            "created"
        },
        if current.draft { " (draft)" } else { "" }
    );

    for asset in expand_assets(cwd, assets)? {
        if !asset.is_file() {
            bail!("Asset not found: {}", asset.display());
        }
        client.upload_asset(&current, &asset, None)?;
        println!("uploaded {}", asset.display());
    }

    if current.draft {
        current = client.update_release(
            current.id,
            &UpdateRelease {
                draft: Some(false),
                ..UpdateRelease::default()
            },
        )?;
        println!("published {tag}");
    }

    set_output("release_url", &current.html_url);
    set_output("release_id", &current.id.to_string());
    println!("{}", current.html_url);
    Ok(())
}

fn verify(cli: &Cli, cwd: &Path, tag_value: Option<&str>) -> Result<()> {
    let client = client(cli)?;
    let tag = match tag_value {
        Some(tag) => tag.to_string(),
        None => compute(cli, cwd)?.tag,
    };
    let current = client.get_release_by_tag(&tag)?;
    set_output("exists", if current.is_some() { "true" } else { "false" });
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "tag": tag,
            "exists": current.is_some(),
            "release": current,
        }))?
    );
    Ok(())
}
