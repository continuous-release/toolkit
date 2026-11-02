use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use continuous_release::adapters::{get_adapter, write_version, Adapter};
use continuous_release::git::fetch_tags;
use continuous_release::notes::{render_notes, NotesOptions};
use continuous_release::project::{Project, ProjectSet};
use continuous_release::release::{
    CreateRelease, Release, ReleaseClient, ReleaseClientOptions, TagConflictError, UpdateRelease,
};
use continuous_release::version::{
    compute_next_version, last_tag_before, list_version_tags, read_commits_filtered,
    validate_prefix, ComputeOptions, NextVersion,
};
use serde_json::{json, Value};
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

    /// Prefix for version tags in single-version mode
    #[arg(long = "tag-prefix", global = true, default_value = "v")]
    tag_prefix: String,

    /// Project configuration file (defaults to cr.json at the repository root)
    #[arg(long, global = true)]
    config: Option<PathBuf>,

    /// Select a project from the configuration; repeatable
    #[arg(long = "project", global = true)]
    project: Vec<String>,

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
        /// Write the generated notes to this path (a directory for multi-project runs)
        #[arg(long = "notes-out")]
        notes_out: Option<PathBuf>,

        /// Print the plan as JSON
        #[arg(long)]
        json: bool,
    },

    /// Render notes without reserving or releasing
    Notes {
        /// Write the notes to this path (a directory for multi-project runs)
        #[arg(long)]
        out: Option<PathBuf>,
    },

    /// Write a version through an adapter
    Version {
        /// Version file adapter (npm, cargo, text); defaults to the project adapter
        #[arg(long)]
        adapter: Option<String>,

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

        /// Write the generated notes to this path (a directory for multi-project runs)
        #[arg(long = "notes-out")]
        notes_out: Option<PathBuf>,
    },

    /// Drop a reservation that never reached publish
    Unreserve {
        /// Tag to remove; omit to unreserve the next tag of every selected project
        #[arg(long)]
        tag: Option<String>,

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

struct Run {
    cwd: PathBuf,
    root: PathBuf,
    projects: Option<ProjectSet>,
    targets: Vec<Target>,
}

/// One unit of work: the repository in single-version mode, or one configured
/// project. Commands iterate targets so the single-project path stays small.
#[derive(Clone)]
struct Target {
    project: Option<Project>,
    tag_prefix: String,
    paths: Vec<String>,
}

impl Target {
    fn name(&self) -> Option<&str> {
        self.project.as_ref().map(|project| project.name.as_str())
    }

    fn label(&self) -> &str {
        self.name().unwrap_or("repository")
    }

    fn directory(&self, cwd: &Path) -> PathBuf {
        self.project
            .as_ref()
            .map(|project| project.path.clone())
            .unwrap_or_else(|| cwd.to_path_buf())
    }

    fn artifacts(&self) -> &[String] {
        self.project
            .as_ref()
            .map(|project| project.artifacts.as_slice())
            .unwrap_or(&[])
    }

    fn adapter(&self) -> Option<Adapter> {
        self.project.as_ref().and_then(|project| project.adapter)
    }

    fn file(&self) -> Option<&str> {
        self.project
            .as_ref()
            .and_then(|project| project.file.as_deref())
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let context = context(&cli)?;

    if context.targets.len() > 1 && cli.release_as.is_some() {
        bail!(
            "--release-as is ambiguous with {} selected projects; run once per project",
            context.targets.len()
        );
    }

    match &cli.command {
        Command::Plan { notes_out, json } => plan(&cli, &context, notes_out.as_deref(), *json),
        Command::Notes { out } => notes(&cli, &context, out.as_deref()),
        Command::Version {
            adapter,
            file,
            version,
        } => write_version_command(
            &cli,
            &context,
            adapter.as_deref(),
            file.as_deref(),
            version.as_deref(),
        ),
        Command::Reserve { target, notes_out } => {
            reserve(&cli, &context, target.as_deref(), notes_out.as_deref())
        }
        Command::Unreserve { tag, target } => {
            unreserve(&cli, &context, tag.as_deref(), target.as_deref())
        }
        Command::Release {
            tag,
            target,
            notes_file,
            asset,
            prerelease,
        } => release(
            &cli,
            &context,
            tag.as_deref(),
            target.as_deref(),
            notes_file.as_deref(),
            asset,
            *prerelease,
        ),
        Command::Verify { tag } => verify(&cli, &context, tag.as_deref()),
    }
}

fn context(cli: &Cli) -> Result<Run> {
    validate_prefix(&cli.tag_prefix)?;
    let cwd = cli
        .cwd
        .clone()
        .unwrap_or(std::env::current_dir().context("reading the current directory")?);
    let root = continuous_release::version::repository_root(&cwd).unwrap_or_else(|_| cwd.clone());

    let config_path = match &cli.config {
        Some(path) => {
            let path = resolve(&cwd, path);
            if !path.is_file() {
                bail!("project config {} does not exist", path.display());
            }
            Some(path)
        }
        None => {
            let candidate = root.join("cr.json");
            candidate.is_file().then_some(candidate)
        }
    };

    let projects = match &config_path {
        Some(path) => Some(ProjectSet::load(path, &root)?),
        None => None,
    };
    if !cli.project.is_empty() && projects.is_none() {
        bail!("--project requires cr.json or --config");
    }

    let targets = match &projects {
        Some(set) => set
            .select(&cli.project)?
            .into_iter()
            .map(|project| Target {
                tag_prefix: project.tag_prefix.clone(),
                paths: set.filters_for(&project),
                project: Some(project),
            })
            .collect(),
        None => vec![Target {
            project: None,
            tag_prefix: cli.tag_prefix.clone(),
            paths: Vec::new(),
        }],
    };

    Ok(Run {
        cwd,
        root,
        projects,
        targets,
    })
}

fn compute(cli: &Cli, context: &Run, target: &Target) -> Result<NextVersion> {
    compute_next_version(&ComputeOptions {
        cwd: &context.root,
        release_as: cli.release_as.as_deref(),
        prefix: &target.tag_prefix,
        paths: &target.paths,
    })
}

fn repository(cli: &Cli) -> Option<String> {
    cli.repo
        .clone()
        .or_else(|| std::env::var("GITHUB_REPOSITORY").ok())
        .filter(|value| !value.is_empty())
}

fn require_repository(cli: &Cli) -> Result<String> {
    repository(cli).context("a repository is required (--repo or GITHUB_REPOSITORY)")
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
        repository: require_repository(cli)?,
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

/// The tag the notes should compare against. For a re-run whose tag is already
/// at HEAD, use the tag before it so the notes still describe the release.
fn previous_tag_for(next: &NextVersion, context: &Run, target: &Target) -> Option<String> {
    if next.existing {
        last_tag_before("HEAD", &context.root, &target.tag_prefix)
            .or_else(|| next.previous_tag.clone())
    } else {
        next.previous_tag.clone()
    }
}

fn render(cli: &Cli, context: &Run, target: &Target, next: &NextVersion) -> Result<String> {
    if !next.changed {
        return Ok(String::new());
    }

    let previous_tag = previous_tag_for(next, context, target);
    let range = previous_tag
        .as_ref()
        .map(|tag| format!("{tag}..HEAD"))
        .unwrap_or_else(|| "HEAD".to_string());
    let commits = read_commits_filtered(&range, &context.root, &target.paths)?;
    let repository = repository(cli);

    Ok(render_notes(&NotesOptions {
        commits: &commits,
        previous_tag: previous_tag.as_deref(),
        version: &next.version,
        repository: repository.as_deref(),
        tag_prefix: &target.tag_prefix,
    }))
}

/// Compute notes paths for one target. In a multi-project run `--notes-out` is
/// a directory and each changed project gets `<project>.md`.
fn notes_path_for(
    context: &Run,
    target: &Target,
    next: &NextVersion,
    notes_out: Option<&Path>,
    multi: bool,
) -> Result<Option<PathBuf>> {
    let Some(notes_out) = notes_out else {
        return Ok(None);
    };

    if multi {
        let directory = resolve(&context.cwd, notes_out);
        if directory.exists() && !directory.is_dir() {
            bail!(
                "notes output {} must be a directory in a multi-project run",
                directory.display()
            );
        }
        fs::create_dir_all(&directory)
            .with_context(|| format!("creating {}", directory.display()))?;
        if !next.changed {
            return Ok(None);
        }
        let name = target.name().unwrap_or("repository");
        Ok(Some(directory.join(format!("{name}.md"))))
    } else {
        Ok(Some(resolve(&context.cwd, notes_out)))
    }
}

fn write_notes(path: Option<&Path>, body: &str) -> Result<()> {
    if let Some(path) = path {
        fs::write(path, body).with_context(|| format!("writing {}", path.display()))?;
    }
    Ok(())
}

fn plan(cli: &Cli, context: &Run, notes_out: Option<&Path>, as_json: bool) -> Result<()> {
    let multi = context.targets.len() > 1;
    let mut entries = Vec::new();
    let mut notes_directory: Option<PathBuf> = None;

    for target in &context.targets {
        let next = compute(cli, context, target)?;
        let body = render(cli, context, target, &next)?;
        let notes_path = notes_path_for(context, target, &next, notes_out, multi)?;
        write_notes(notes_path.as_deref(), &body)?;
        if multi {
            if let Some(path) = &notes_path {
                notes_directory = path.parent().map(Path::to_path_buf);
            }
        }

        let previous_tag = previous_tag_for(&next, context, target);
        entries.push(json!({
            "name": target.name(),
            "path": target.project.as_ref().map(|project| project.relative_path.clone()),
            "version": next.version,
            "tag": next.tag,
            "previousTag": previous_tag,
            "bump": next.bump.as_str(),
            "existing": next.existing,
            "changed": next.changed,
            "releaseAs": next.release_as,
            "notes": if next.changed { Value::String(body) } else { Value::Null },
            "notesPath": notes_path.map(|path| path.display().to_string()),
        }));
    }

    let changed = entries
        .iter()
        .any(|entry| entry["changed"].as_bool().unwrap_or(false));
    write_project_output(&entries)?;
    set_output("changed", if changed { "true" } else { "false" });
    if entries.len() == 1 {
        let entry = &entries[0];
        set_output("version", entry["version"].as_str().unwrap_or(""));
        set_output("tag", entry["tag"].as_str().unwrap_or(""));
        set_output("previous_tag", entry["previousTag"].as_str().unwrap_or(""));
        set_output(
            "existing",
            if entry["existing"] == true {
                "true"
            } else {
                "false"
            },
        );
        set_output("bump", entry["bump"].as_str().unwrap_or("none"));
        if let Some(name) = entry["name"].as_str() {
            set_output("project", name);
        }
        if let Some(path) = entry["notesPath"].as_str() {
            set_output("notes_path", path);
        }
    } else if let Some(directory) = &notes_directory {
        set_output("notes_dir", &directory.display().to_string());
    }

    if as_json {
        let value = if entries.len() == 1 {
            entries.remove(0)
        } else {
            json!({ "projects": entries })
        };
        println!("{}", serde_json::to_string_pretty(&value)?);
    } else if entries.len() == 1 {
        print_plan_entry(&entries[0], false);
    } else {
        for entry in &entries {
            print_plan_entry(entry, true);
        }
    }

    Ok(())
}

fn print_plan_entry(entry: &Value, with_name: bool) {
    let name = entry["name"].as_str().unwrap_or("repository");
    let version = entry["version"].as_str().unwrap_or("");
    let tag = entry["tag"].as_str().unwrap_or("");
    let previous = entry["previousTag"].as_str().unwrap_or("(none)");
    let bump = entry["bump"].as_str().unwrap_or("none");
    let existing = entry["existing"].as_bool().unwrap_or(false);
    let changed = entry["changed"].as_bool().unwrap_or(true);

    if with_name {
        println!(
            "{name}: {version} ({tag}, {bump}{}{})",
            if existing { ", existing" } else { "" },
            if changed { "" } else { ", unchanged" }
        );
    } else {
        println!("version: {version}");
        println!("tag: {tag}");
        println!("previous tag: {previous}");
        println!("bump: {bump}");
        println!("existing: {existing}");
        println!("changed: {changed}");
    }
}

fn write_project_output(entries: &[Value]) -> Result<()> {
    let compact: Vec<Value> = entries
        .iter()
        .map(|entry| {
            json!({
                "name": entry["name"],
                "version": entry["version"],
                "tag": entry["tag"],
                "previousTag": entry["previousTag"],
                "bump": entry["bump"],
                "existing": entry["existing"],
                "changed": entry["changed"],
            })
        })
        .collect();
    set_output("projects", &serde_json::to_string(&compact)?);
    Ok(())
}

fn notes(cli: &Cli, context: &Run, out: Option<&Path>) -> Result<()> {
    let multi = context.targets.len() > 1;
    for target in &context.targets {
        let next = compute(cli, context, target)?;
        let body = render(cli, context, target, &next)?;
        match out {
            Some(path) => {
                let notes_path = notes_path_for(context, target, &next, Some(path), multi)?;
                write_notes(notes_path.as_deref(), &body)?;
            }
            None if multi => {
                if next.changed {
                    println!("# {}", target.label());
                    println!();
                    print!("{body}");
                    println!();
                }
            }
            None => print!("{body}"),
        }
    }
    Ok(())
}

fn write_version_command(
    cli: &Cli,
    context: &Run,
    adapter_name: Option<&str>,
    file: Option<&str>,
    version: Option<&str>,
) -> Result<()> {
    let multi = context.targets.len() > 1;
    if multi {
        if adapter_name.is_some() {
            bail!(
                "--adapter is ambiguous with {} selected projects",
                context.targets.len()
            );
        }
        if file.is_some() {
            bail!(
                "--file is ambiguous with {} selected projects",
                context.targets.len()
            );
        }
        if version.is_some() {
            bail!(
                "--version is ambiguous with {} selected projects",
                context.targets.len()
            );
        }
    }

    let cli_adapter = adapter_name.map(get_adapter).transpose()?;

    // Compute every version before touching a file so a planning or adapter
    // error cannot leave a partial bump behind.
    let mut planned: Vec<(Target, NextVersion)> = Vec::new();
    for target in &context.targets {
        let next = compute(cli, context, target)?;
        if version.is_none() && !next.changed {
            println!("unchanged {} ({})", target.label(), next.tag);
            continue;
        }
        planned.push((target.clone(), next));
    }

    let mut writes: Vec<(Target, Adapter, Option<String>, String)> = Vec::new();
    for (target, next) in planned {
        let adapter =
            cli_adapter
                .or_else(|| target.adapter())
                .with_context(|| match target.name() {
                    Some(name) => format!(
                        "project {name} has no adapter; pass --adapter or set adapter in cr.json"
                    ),
                    None => "an adapter is required (--adapter)".to_string(),
                })?;
        let file = file.or_else(|| target.file()).map(str::to_string);
        let value = match version {
            Some(version) => version.to_string(),
            None => next.version,
        };
        writes.push((target, adapter, file, value));
    }

    for (target, adapter, file, version) in writes {
        let directory = target.directory(&context.cwd);
        write_version(&directory, adapter, &version, file.as_deref())?;
        println!(
            "set {} version to {version}{} for {}",
            adapter.name(),
            file.map(|file| format!(" in {file}")).unwrap_or_default(),
            target.label()
        );
    }
    Ok(())
}

struct ReserveResult {
    target: Target,
    next: NextVersion,
    previous_tag: Option<String>,
    notes_path: Option<PathBuf>,
    superseded: bool,
}

enum ReserveOutcome {
    Unchanged(ReserveResult),
    Reserved(ReserveResult),
    Superseded(ReserveResult),
}

fn reserve(
    cli: &Cli,
    context: &Run,
    target_value: Option<&str>,
    notes_out: Option<&Path>,
) -> Result<()> {
    let client = client(cli)?;
    let target_sha = target_commit(target_value)?;
    let multi = context.targets.len() > 1;
    let mut results: Vec<ReserveResult> = Vec::new();
    let mut created_tags: Vec<String> = Vec::new();

    for target in &context.targets {
        let outcome = match reserve_target(
            cli,
            context,
            target,
            &client,
            &target_sha,
            notes_out,
            multi,
            &mut created_tags,
        ) {
            Ok(outcome) => outcome,
            Err(error) => {
                // A later project failed, so drop the leases this run created.
                rollback_tags(&client, &created_tags);
                return Err(error);
            }
        };

        match outcome {
            ReserveOutcome::Unchanged(result) | ReserveOutcome::Reserved(result) => {
                results.push(result);
            }
            ReserveOutcome::Superseded(result) => {
                println!(
                    "superseded by {} for {}",
                    result.next.tag,
                    result.target.label()
                );
                results.push(result);
                rollback_tags(&client, &created_tags);
                finish_superseded(cli, context, results)?;
                return Ok(());
            }
        }
    }

    write_reserve_outputs(&results)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn reserve_target(
    cli: &Cli,
    context: &Run,
    target: &Target,
    client: &ReleaseClient,
    target_sha: &str,
    notes_out: Option<&Path>,
    multi: bool,
    created_tags: &mut Vec<String>,
) -> Result<ReserveOutcome> {
    let next = compute(cli, context, target)?;
    if !next.changed {
        println!("unchanged {} ({})", target.label(), next.tag);
        return Ok(ReserveOutcome::Unchanged(ReserveResult {
            target: target.clone(),
            previous_tag: next.previous_tag.clone(),
            next,
            notes_path: None,
            superseded: false,
        }));
    }

    let mut attempts = 0;
    let next = loop {
        let next = compute(cli, context, target)?;
        if !next.changed {
            break next;
        }

        match client.reserve_tag(&next.tag, target_sha) {
            Ok(created) => {
                if created {
                    created_tags.push(next.tag.clone());
                }
                println!(
                    "{} {} at {} for {}",
                    if created { "reserved" } else { "kept" },
                    next.tag,
                    target_sha,
                    target.label()
                );
                break next;
            }
            Err(error) => {
                let Some(conflict) = error.downcast_ref::<TagConflictError>() else {
                    return Err(error);
                };
                println!(
                    "{} was taken by {}; re-planning {}",
                    next.tag,
                    conflict.actual,
                    target.label()
                );
                fetch_tags(&context.root)?;
                let replanned = compute(cli, context, target)?;
                if replanned.tag == next.tag {
                    return Ok(ReserveOutcome::Superseded(ReserveResult {
                        target: target.clone(),
                        previous_tag: next.previous_tag.clone(),
                        next,
                        notes_path: None,
                        superseded: true,
                    }));
                }
                attempts += 1;
                if attempts >= 5 {
                    bail!(
                        "Could not reserve a version for {} after 5 attempts",
                        target.label()
                    );
                }
            }
        }
    };

    let previous_tag = previous_tag_for(&next, context, target);
    let body = render(cli, context, target, &next)?;
    let notes_path = notes_path_for(context, target, &next, notes_out, multi)?;
    write_notes(notes_path.as_deref(), &body)?;
    Ok(ReserveOutcome::Reserved(ReserveResult {
        target: target.clone(),
        previous_tag,
        next,
        notes_path,
        superseded: false,
    }))
}

fn finish_superseded(cli: &Cli, context: &Run, mut results: Vec<ReserveResult>) -> Result<()> {
    // Include the remaining targets in the report so a workflow can see every
    // project, but do not reserve anything else.
    let reported: Vec<String> = results
        .iter()
        .map(|result| result.target.label().to_string())
        .collect();
    for target in &context.targets {
        if reported.contains(&target.label().to_string()) {
            continue;
        }
        let next = compute(cli, context, target)?;
        results.push(ReserveResult {
            target: target.clone(),
            previous_tag: next.previous_tag.clone(),
            next,
            notes_path: None,
            superseded: false,
        });
    }

    write_reserve_outputs(&results)?;
    set_output("superseded", "true");
    if results.len() == 1 {
        println!("superseded");
    }
    Ok(())
}

fn write_reserve_outputs(results: &[ReserveResult]) -> Result<()> {
    let entries: Vec<Value> = results
        .iter()
        .map(|result| {
            json!({
                "name": result.target.name(),
                "version": result.next.version,
                "tag": result.next.tag,
                "previousTag": result.previous_tag,
                "bump": result.next.bump.as_str(),
                "existing": result.next.existing,
                "changed": result.next.changed,
                "superseded": result.superseded,
            })
        })
        .collect();

    set_output("projects", &serde_json::to_string(&entries)?);
    let changed = results.iter().any(|result| result.next.changed);
    set_output("changed", if changed { "true" } else { "false" });
    set_output(
        "superseded",
        if results.iter().any(|result| result.superseded) {
            "true"
        } else {
            "false"
        },
    );

    if results.len() == 1 {
        let result = &results[0];
        set_output("version", &result.next.version);
        set_output("tag", &result.next.tag);
        set_output("previous_tag", result.previous_tag.as_deref().unwrap_or(""));
        set_output(
            "existing",
            if result.next.existing {
                "true"
            } else {
                "false"
            },
        );
        set_output("bump", result.next.bump.as_str());
        if let Some(name) = result.target.name() {
            set_output("project", name);
        }
        if let Some(path) = &result.notes_path {
            set_output("notes_path", &path.display().to_string());
        }
    } else {
        if let Some(directory) = results
            .iter()
            .find_map(|result| result.notes_path.as_ref())
            .and_then(|path| path.parent())
        {
            set_output("notes_dir", &directory.display().to_string());
        }
    }

    Ok(())
}

fn rollback_tags(client: &ReleaseClient, tags: &[String]) {
    for tag in tags.iter().rev() {
        match client.delete_tag(tag) {
            Ok(()) => println!("rolled back {tag}"),
            Err(error) => eprintln!("warning: could not roll back {tag}: {error:#}"),
        }
    }
}

fn unreserve(
    cli: &Cli,
    context: &Run,
    tag: Option<&str>,
    target_value: Option<&str>,
) -> Result<()> {
    let client = client(cli)?;

    if let Some(tag) = tag {
        let prefix = context
            .projects
            .as_ref()
            .and_then(|set| {
                set.projects
                    .iter()
                    .find(|project| tag.starts_with(&project.tag_prefix))
                    .map(|project| project.tag_prefix.clone())
            })
            .or_else(|| {
                tag.starts_with(&cli.tag_prefix)
                    .then(|| cli.tag_prefix.clone())
            });
        return unreserve_tag(&client, context, tag, target_value, prefix.as_deref());
    }

    for target in &context.targets {
        let next = compute(cli, context, target)?;
        if !next.changed {
            println!("unchanged {} ({})", target.label(), next.tag);
            continue;
        }
        unreserve_tag(
            &client,
            context,
            &next.tag,
            target_value,
            Some(&target.tag_prefix),
        )?;
    }
    Ok(())
}

fn unreserve_tag(
    client: &ReleaseClient,
    context: &Run,
    tag: &str,
    target_value: Option<&str>,
    prefix: Option<&str>,
) -> Result<()> {
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

    fetch_tags(&context.root)?;
    if let Some(prefix) = prefix {
        if let Some(highest) = list_version_tags(&context.root, prefix).first() {
            if highest != tag {
                println!("keeping {tag}: {highest} is newer");
                return Ok(());
            }
        }
    }

    client.delete_tag(tag)?;
    println!("unreserved {tag}");
    Ok(())
}

fn expand_assets(base: &Path, patterns: &[String]) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();

    for pattern in patterns {
        let literal = resolve(base, Path::new(pattern));
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

fn target_assets(context: &Run, target: &Target, explicit: &[String]) -> Result<Vec<PathBuf>> {
    let patterns: Vec<String> = if explicit.is_empty() {
        target.artifacts().to_vec()
    } else {
        explicit.to_vec()
    };
    if patterns.is_empty() {
        return Ok(Vec::new());
    }
    let base = target.directory(&context.cwd);
    expand_assets(&base, &patterns)
}

struct PreparedRelease {
    target: Target,
    tag: String,
    body: String,
    assets: Vec<PathBuf>,
    prerelease: bool,
    release: Option<Release>,
}

fn release(
    cli: &Cli,
    context: &Run,
    tag_value: Option<&str>,
    target_value: Option<&str>,
    notes_file: Option<&Path>,
    assets: &[String],
    prerelease: bool,
) -> Result<()> {
    let multi = context.targets.len() > 1;
    if multi {
        if tag_value.is_some() {
            bail!(
                "--tag is ambiguous with {} selected projects",
                context.targets.len()
            );
        }
        if notes_file.is_some() {
            bail!(
                "--notes-file is ambiguous with {} selected projects",
                context.targets.len()
            );
        }
        if !assets.is_empty() {
            bail!(
                "--asset is ambiguous with {} selected projects",
                context.targets.len()
            );
        }
    }

    let client = client(cli)?;
    let project_mode = context.projects.is_some();
    let target = target_value
        .map(str::to_string)
        .or_else(|| std::env::var("GITHUB_SHA").ok())
        .filter(|value| !value.is_empty());

    let explicit_notes = match notes_file {
        Some(path) => Some(
            fs::read_to_string(resolve(&context.cwd, path))
                .with_context(|| format!("reading {}", path.display()))?,
        ),
        None => None,
    };

    let mut prepared = Vec::new();
    for work in &context.targets {
        let next = match tag_value {
            Some(_) => None,
            None => Some(compute(cli, context, work)?),
        };
        if let Some(next) = &next {
            if !next.changed {
                println!("unchanged {} ({})", work.label(), next.tag);
                continue;
            }
        }

        let tag = tag_value
            .map(str::to_string)
            .or_else(|| next.as_ref().map(|next| next.tag.clone()))
            .expect("a tag was computed or supplied");
        let body = match &explicit_notes {
            Some(body) => body.clone(),
            // In project mode, a release without an explicit notes file gets
            // the notes derived from its own commits. The legacy single-repo
            // behaviour of publishing an empty body stays unchanged.
            None if project_mode && tag_value.is_none() => match &next {
                Some(next) if next.changed => render(cli, context, work, next)?,
                _ => String::new(),
            },
            None => String::new(),
        };
        let assets = target_assets(context, work, assets)?;
        prepared.push(PreparedRelease {
            target: work.clone(),
            prerelease: prerelease || tag.contains('-'),
            tag,
            body,
            assets,
            release: None,
        });
    }

    if prepared.is_empty() {
        println!("no changed projects");
        set_output("changed", "false");
        set_output("releases", "[]");
        return Ok(());
    }

    // Ensure every tag exists before publishing anything.
    for item in &prepared {
        match client.resolve_tag_commit(&item.tag)? {
            Some(tagged) => {
                if let Some(target) = &target {
                    if &tagged != target {
                        bail!("tag {} points at {tagged}, not {target}", item.tag);
                    }
                }
            }
            None => {
                let target = target
                    .as_deref()
                    .context("a target commit is required to create the tag")?;
                client.reserve_tag(&item.tag, target)?;
                println!("reserved {} at {target}", item.tag);
            }
        }
    }

    // Draft (or update) every release before uploading any asset.
    for item in &mut prepared {
        let existing = client.get_release_by_tag(&item.tag)?;
        item.release = Some(match &existing {
            Some(release) => client.update_release(
                release.id,
                &UpdateRelease {
                    name: Some(&item.tag),
                    body: Some(&item.body),
                    prerelease: Some(item.prerelease),
                    draft: None,
                },
            )?,
            None => client.create_release(&CreateRelease {
                tag: &item.tag,
                target: target.as_deref(),
                name: Some(&item.tag),
                body: &item.body,
                prerelease: item.prerelease,
                draft: true,
            })?,
        });
        println!(
            "{} release {} for {}",
            if existing.is_some() {
                "updated"
            } else {
                "created"
            },
            item.tag,
            item.target.label()
        );
    }

    // Upload every asset to every draft before promoting any of them.
    for item in &prepared {
        let release = item.release.as_ref().expect("release was created");
        for asset in &item.assets {
            if !asset.is_file() {
                bail!("Asset not found: {}", asset.display());
            }
            client.upload_asset(release, asset, None)?;
            println!("uploaded {} for {}", asset.display(), item.target.label());
        }
    }

    // Only now make the new releases visible.
    for item in &mut prepared {
        let release = item.release.as_ref().expect("release was created");
        if release.draft {
            item.release = Some(client.update_release(
                release.id,
                &UpdateRelease {
                    draft: Some(false),
                    ..UpdateRelease::default()
                },
            )?);
            println!("published {} for {}", item.tag, item.target.label());
        }
    }

    let entries: Vec<Value> = prepared
        .iter()
        .map(|item| {
            let release = item.release.as_ref().expect("release was created");
            json!({
                "name": item.target.name(),
                "tag": item.tag,
                "id": release.id,
                "url": release.html_url,
                "draft": release.draft,
                "prerelease": release.prerelease,
            })
        })
        .collect();
    set_output("changed", "true");
    set_output("releases", &serde_json::to_string(&entries)?);

    if !multi {
        let release = prepared[0].release.as_ref().expect("release was created");
        set_output("release_url", &release.html_url);
        set_output("release_id", &release.id.to_string());
        set_output("tag", &prepared[0].tag);
        println!("{}", release.html_url);
    } else {
        for item in &prepared {
            let release = item.release.as_ref().expect("release was created");
            println!("{}: {}", item.target.label(), release.html_url);
        }
    }

    Ok(())
}

fn verify(cli: &Cli, context: &Run, tag_value: Option<&str>) -> Result<()> {
    if context.targets.len() > 1 && tag_value.is_some() {
        bail!(
            "--tag is ambiguous with {} selected projects",
            context.targets.len()
        );
    }

    let client = client(cli)?;
    let mut entries = Vec::new();
    for target in &context.targets {
        let tag = match tag_value {
            Some(tag) => tag.to_string(),
            None => compute(cli, context, target)?.tag,
        };
        let release = client.get_release_by_tag(&tag)?;
        entries.push(json!({
            "name": target.name(),
            "tag": tag,
            "exists": release.is_some(),
            "release": release,
        }));
    }

    let all_exist = entries
        .iter()
        .all(|entry| entry["exists"].as_bool().unwrap_or(false));
    set_output("exists", if all_exist { "true" } else { "false" });
    set_output("projects", &serde_json::to_string(&entries)?);

    let value = if entries.len() == 1 {
        let entry = entries.remove(0);
        json!({
            "tag": entry["tag"],
            "exists": entry["exists"],
            "release": entry["release"],
        })
    } else {
        json!({ "projects": entries })
    };
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}
