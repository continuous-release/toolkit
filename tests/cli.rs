use continuous_release::release::{ReleaseClient, ReleaseClientOptions};
use fake_github::FakeGitHub;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use tempfile::TempDir;

fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git runs");
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

struct Repo {
    dir: TempDir,
    _remote: TempDir,
}

impl Repo {
    fn new() -> Self {
        let dir = TempDir::new().expect("repo");
        let remote = TempDir::new().expect("remote");
        git(dir.path(), &["init", "-q", "-b", "main"]);
        git(dir.path(), &["config", "user.email", "test@example.com"]);
        git(dir.path(), &["config", "user.name", "Test"]);
        git(remote.path(), &["init", "-q", "--bare"]);
        git(
            dir.path(),
            &[
                "remote",
                "add",
                "origin",
                remote.path().to_str().expect("path"),
            ],
        );
        Self {
            dir,
            _remote: remote,
        }
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }

    fn write(&self, relative: &str, contents: &str) {
        let path = self.path().join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create dirs");
        }
        fs::write(path, contents).expect("write");
    }

    fn write_config(&self, config: Value) {
        self.write(
            "cr.json",
            &serde_json::to_string_pretty(&config).expect("config json"),
        );
    }

    fn commit(&self, message: &str, body: Option<&str>) {
        git(self.path(), &["add", "-A"]);
        let mut args = vec!["commit", "-q", "-m", message];
        if let Some(body) = body {
            args.push("-m");
            args.push(body);
        }
        git(self.path(), &args);
        self.push();
    }

    fn tag(&self, name: &str) {
        git(self.path(), &["tag", name]);
        self.push();
    }

    fn push(&self) {
        git(self.path(), &["push", "-q", "origin", "main", "--tags"]);
    }

    fn head(&self) -> String {
        git(self.path(), &["rev-parse", "HEAD"])
    }
}

fn fake() -> FakeGitHub {
    let mut fake = FakeGitHub::with_default_repo("o", "r");
    fake.start().expect("start fake GitHub");
    fake
}

fn client(fake: &FakeGitHub) -> ReleaseClient {
    ReleaseClient::new(ReleaseClientOptions {
        token: "t".to_string(),
        repository: "o/r".to_string(),
        api_url: Some(fake.url().to_string()),
    })
    .expect("client")
}

fn run(repo: &Repo, fake: &FakeGitHub, output_path: Option<&Path>, args: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cr"));
    command
        .current_dir(repo.path())
        .env("GITHUB_REPOSITORY", "o/r")
        .env("GITHUB_TOKEN", "t")
        .env("GITHUB_API_URL", fake.url())
        .env("GITHUB_SHA", repo.head())
        .env_remove("GITHUB_OUTPUT")
        .args(args);
    if let Some(path) = output_path {
        command.env("GITHUB_OUTPUT", path);
    }
    command.output().expect("run cr")
}

fn success(output: &Output) {
    assert!(
        output.status.success(),
        "cr failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn failure(output: &Output) -> String {
    assert!(
        !output.status.success(),
        "cr unexpectedly succeeded\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stderr).to_string()
}

fn json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "stdout is not JSON: {error}\n{}",
            String::from_utf8_lossy(&output.stdout)
        )
    })
}

fn outputs(path: &Path) -> HashMap<String, String> {
    fs::read_to_string(path)
        .expect("read outputs")
        .lines()
        .filter_map(|line| {
            line.split_once('=')
                .map(|(key, value)| (key.to_string(), value.to_string()))
        })
        .collect()
}

fn output_file(repo: &Repo) -> std::path::PathBuf {
    let path = repo.path().join("github-output");
    fs::write(&path, "").expect("clear outputs");
    path
}

fn project<'a>(value: &'a Value, name: &str) -> &'a Value {
    value["projects"]
        .as_array()
        .expect("projects array")
        .iter()
        .find(|project| project["name"] == name)
        .unwrap_or_else(|| panic!("project {name} missing from {value}"))
}

fn fake_release(fake: &FakeGitHub, tag: &str) -> fake_github::Release {
    fake.list_releases()
        .into_iter()
        .find(|release| release.tag_name == tag)
        .unwrap_or_else(|| panic!("release {tag} missing"))
}

fn two_projects() -> Repo {
    let repo = Repo::new();
    repo.write_config(json!({
        "projects": {
            "core": {
                "path": "packages/core",
                "adapter": "npm",
                "file": "package.json",
                "artifacts": ["dist/*.tgz"]
            },
            "ui": {
                "path": "packages/ui",
                "adapter": "text",
                "file": "VERSION",
                "artifacts": ["dist/*.tgz"]
            }
        }
    }));
    repo.write(
        "packages/core/package.json",
        "{\n  \"name\": \"core\",\n  \"version\": \"1.0.0\"\n}\n",
    );
    repo.write("packages/core/index.js", "one\n");
    repo.write("packages/ui/VERSION", "1.0.0\n");
    repo.write("packages/ui/index.js", "one\n");
    repo.commit("feat: initial packages", None);
    repo.tag("core-v1.0.0");
    repo.tag("ui-v1.0.0");
    repo
}

#[test]
fn plan_reports_each_project_and_writes_per_project_notes() {
    let repo = two_projects();
    repo.write("packages/core/index.js", "two\n");
    repo.commit("feat(core): add new api", None);
    let mut fake = fake();
    let outputs_path = output_file(&repo);

    let output = run(
        &repo,
        &fake,
        Some(&outputs_path),
        &["plan", "--json", "--notes-out", "notes"],
    );
    success(&output);
    let value = json(&output);
    let core = project(&value, "core");
    let ui = project(&value, "ui");
    assert_eq!(core["version"], "1.1.0");
    assert_eq!(core["tag"], "core-v1.1.0");
    assert_eq!(core["previousTag"], "core-v1.0.0");
    assert_eq!(core["changed"], true);
    assert_eq!(ui["version"], "1.0.0");
    assert_eq!(ui["tag"], "ui-v1.0.0");
    assert_eq!(ui["changed"], false);

    let core_notes = fs::read_to_string(repo.path().join("notes/core.md")).expect("core notes");
    assert!(core_notes.contains("feat(core): add new api"));
    assert!(!repo.path().join("notes/ui.md").exists());

    let outputs = outputs(&outputs_path);
    assert!(outputs["projects"].contains("\"core\""));
    assert_eq!(outputs["changed"], "true");
    assert!(outputs.contains_key("notes_dir"));
    assert!(!outputs.contains_key("version"));

    fake.stop();
}

#[test]
fn selecting_one_project_gives_flat_outputs() {
    let repo = two_projects();
    repo.write("packages/core/index.js", "two\n");
    repo.commit("feat(core): add new api", None);
    let mut fake = fake();
    let outputs_path = output_file(&repo);

    let output = run(
        &repo,
        &fake,
        Some(&outputs_path),
        &["plan", "--json", "--project", "ui"],
    );
    success(&output);
    let value = json(&output);
    assert_eq!(value["name"], "ui");
    assert_eq!(value["version"], "1.0.0");
    assert_eq!(value["changed"], false);

    let outputs = outputs(&outputs_path);
    assert_eq!(outputs["project"], "ui");
    assert_eq!(outputs["version"], "1.0.0");
    assert_eq!(outputs["tag"], "ui-v1.0.0");
    assert_eq!(outputs["changed"], "false");

    fake.stop();
}

#[test]
fn plan_rejects_an_ambiguous_release_as() {
    let repo = two_projects();
    let mut fake = fake();
    let output = run(&repo, &fake, None, &["plan", "--release-as", "9.9.9"]);
    assert!(failure(&output).contains("ambiguous"));
    fake.stop();
}

#[test]
fn shared_path_changes_release_every_project() {
    let repo = Repo::new();
    repo.write_config(json!({
        "sharedPaths": ["shared"],
        "projects": {
            "core": {"path": "packages/core"},
            "ui": {"path": "packages/ui"}
        }
    }));
    repo.write("packages/core/index.js", "one\n");
    repo.write("packages/ui/index.js", "one\n");
    repo.write("shared/schema.json", "{}\n");
    repo.commit("feat: initial", None);
    repo.tag("core-v1.0.0");
    repo.tag("ui-v1.0.0");
    repo.write("shared/schema.json", "{\"v\":2}\n");
    repo.commit("fix: update shared schema", None);

    let mut fake = fake();
    let output = run(&repo, &fake, None, &["plan", "--json"]);
    success(&output);
    let value = json(&output);
    assert_eq!(project(&value, "core")["version"], "1.0.1");
    assert_eq!(project(&value, "ui")["version"], "1.0.1");
    assert_eq!(project(&value, "core")["changed"], true);
    assert_eq!(project(&value, "ui")["changed"], true);

    fake.stop();
}

#[test]
fn version_writes_only_changed_projects_with_their_configured_adapters() {
    let repo = two_projects();
    repo.write("packages/core/index.js", "two\n");
    repo.commit("feat(core): add new api", None);
    let mut fake = fake();

    let output = run(&repo, &fake, None, &["version"]);
    success(&output);

    let package: Value = serde_json::from_str(
        &fs::read_to_string(repo.path().join("packages/core/package.json")).unwrap(),
    )
    .expect("package json");
    assert_eq!(package["version"], "1.1.0");
    assert_eq!(
        fs::read_to_string(repo.path().join("packages/ui/VERSION")).unwrap(),
        "1.0.0\n"
    );

    let output = run(&repo, &fake, None, &["version", "--project", "ui"]);
    success(&output);
    assert!(String::from_utf8_lossy(&output.stdout).contains("unchanged ui"));
    assert_eq!(
        fs::read_to_string(repo.path().join("packages/ui/VERSION")).unwrap(),
        "1.0.0\n"
    );

    fake.stop();
}

#[test]
fn version_supports_cargo_and_rejects_ambiguous_overrides() {
    let repo = Repo::new();
    repo.write_config(json!({
        "projects": {
            "core": {"path": "packages/core", "adapter": "npm", "file": "package.json"},
            "rust": {"path": "packages/rust", "adapter": "cargo", "file": "Cargo.toml"}
        }
    }));
    repo.write(
        "packages/core/package.json",
        "{\n  \"name\": \"core\",\n  \"version\": \"1.0.0\"\n}\n",
    );
    repo.write(
        "packages/rust/Cargo.toml",
        "[package]\nname = \"rust\"\nversion = \"1.0.0\"\nedition = \"2021\"\n",
    );
    repo.write("packages/rust/src/lib.rs", "pub fn one() {}\n");
    repo.commit("feat: initial", None);
    repo.tag("core-v1.0.0");
    repo.tag("rust-v1.0.0");
    repo.write("packages/rust/src/lib.rs", "pub fn two() {}\n");
    repo.commit("feat(rust): add two", None);

    let mut fake = fake();
    let output = run(&repo, &fake, None, &["version"]);
    success(&output);
    let cargo = fs::read_to_string(repo.path().join("packages/rust/Cargo.toml")).unwrap();
    assert!(cargo.contains("version = \"1.1.0\""));

    assert!(
        failure(&run(&repo, &fake, None, &["version", "--version", "9.9.9"])).contains("ambiguous")
    );
    assert!(
        failure(&run(&repo, &fake, None, &["version", "--adapter", "npm"])).contains("ambiguous")
    );
    assert!(failure(&run(
        &repo,
        &fake,
        None,
        &["version", "--file", "package.json"]
    ))
    .contains("ambiguous"));

    fake.stop();
}

#[test]
fn version_requires_an_adapter_when_the_config_has_none() {
    let repo = Repo::new();
    repo.write_config(json!({"projects": {"core": {"path": "packages/core"}}}));
    repo.write("packages/core/index.js", "one\n");
    repo.commit("feat: initial", None);
    let mut fake = fake();

    let output = run(&repo, &fake, None, &["version"]);
    assert!(failure(&output).contains("has no adapter"));
    let output = run(
        &repo,
        &fake,
        None,
        &["version", "--adapter", "text", "--file", "index.js"],
    );
    success(&output);

    fake.stop();
}

#[test]
fn reserve_reserves_each_changed_project_and_reports_unchanged_ones() {
    let repo = two_projects();
    repo.write("packages/core/index.js", "two\n");
    repo.commit("feat(core): add new api", None);
    repo.write("packages/ui/index.js", "two\n");
    repo.commit("fix(ui): correct layout", None);

    let mut fake = fake();
    let outputs_path = output_file(&repo);
    let output = run(
        &repo,
        &fake,
        Some(&outputs_path),
        &["reserve", "--notes-out", "notes"],
    );
    success(&output);

    let tags = fake.list_tags();
    assert!(tags.contains(&"core-v1.1.0".to_string()));
    assert!(tags.contains(&"ui-v1.0.1".to_string()));
    let core_notes = fs::read_to_string(repo.path().join("notes/core.md")).expect("core notes");
    let ui_notes = fs::read_to_string(repo.path().join("notes/ui.md")).expect("ui notes");
    assert!(core_notes.contains("feat(core): add new api"));
    assert!(ui_notes.contains("fix(ui): correct layout"));

    let outputs = outputs(&outputs_path);
    assert_eq!(outputs["superseded"], "false");
    assert_eq!(outputs["changed"], "true");
    assert!(outputs["projects"].contains("\"core\""));
    assert!(outputs["projects"].contains("\"ui\""));

    // Re-running the same commit keeps the same leases instead of failing.
    let output = run(&repo, &fake, None, &["reserve"]);
    success(&output);
    let mut tags = fake.list_tags();
    tags.sort();
    assert_eq!(
        tags,
        vec!["core-v1.1.0".to_string(), "ui-v1.0.1".to_string()]
    );

    fake.stop();
}

#[test]
fn reserve_rolls_back_tags_created_earlier_when_a_later_project_is_superseded() {
    let repo = two_projects();
    repo.write("packages/core/index.js", "two\n");
    repo.write("packages/ui/index.js", "two\n");
    repo.commit("feat: change both", None);

    let mut fake = fake();
    // The UI lease was won by another commit. The local repository does not
    // know about it, so re-planning produces the same tag and is superseded.
    fake.seed_tag("ui-v1.1.0", "deadbeef");
    let outputs_path = output_file(&repo);

    let output = run(&repo, &fake, Some(&outputs_path), &["reserve"]);
    success(&output);
    assert!(String::from_utf8_lossy(&output.stdout).contains("rolled back core-v1.1.0"));
    assert!(!fake.list_tags().contains(&"core-v1.1.0".to_string()));
    assert!(fake.list_tags().contains(&"ui-v1.1.0".to_string()));

    let outputs = outputs(&outputs_path);
    assert_eq!(outputs["superseded"], "true");
    assert_eq!(outputs["changed"], "true");

    fake.stop();
}

#[test]
fn release_creates_and_promotes_a_release_for_every_project() {
    let repo = two_projects();
    repo.write("packages/core/index.js", "two\n");
    repo.commit("feat(core): add new api", None);
    repo.write("packages/ui/index.js", "two\n");
    repo.commit("fix(ui): correct layout", None);
    repo.write("packages/core/dist/core.tgz", "core asset");
    repo.write("packages/ui/dist/ui.tgz", "ui asset");

    let mut fake = fake();
    let outputs_path = output_file(&repo);
    let output = run(&repo, &fake, Some(&outputs_path), &["release"]);
    success(&output);

    let core = fake_release(&fake, "core-v1.1.0");
    let ui = fake_release(&fake, "ui-v1.0.1");
    assert_eq!(
        core.assets
            .iter()
            .map(|asset| asset.name.as_str())
            .collect::<Vec<_>>(),
        vec!["core.tgz"]
    );
    assert_eq!(
        ui.assets
            .iter()
            .map(|asset| asset.name.as_str())
            .collect::<Vec<_>>(),
        vec!["ui.tgz"]
    );
    assert!(core
        .body
        .as_deref()
        .unwrap_or_default()
        .contains("feat(core)"));
    assert!(ui.body.as_deref().unwrap_or_default().contains("fix(ui)"));

    let release_client = client(&fake);
    assert!(
        !release_client
            .get_release_by_tag("core-v1.1.0")
            .expect("lookup")
            .expect("release")
            .draft
    );
    assert!(
        !release_client
            .get_release_by_tag("ui-v1.0.1")
            .expect("lookup")
            .expect("release")
            .draft
    );

    let outputs = outputs(&outputs_path);
    let releases: Value = serde_json::from_str(&outputs["releases"]).expect("releases json");
    assert_eq!(releases.as_array().expect("array").len(), 2);
    assert_eq!(outputs["changed"], "true");
    assert!(!outputs.contains_key("tag"));

    // Re-running updates the existing releases and replaces their assets.
    let output = run(&repo, &fake, None, &["release"]);
    success(&output);
    assert_eq!(fake.list_releases().len(), 2);

    fake.stop();
}

#[test]
fn release_creates_nothing_when_one_project_asset_is_missing() {
    let repo = two_projects();
    repo.write("packages/core/index.js", "two\n");
    repo.commit("feat(core): add new api", None);
    repo.write("packages/ui/index.js", "two\n");
    repo.commit("fix(ui): correct layout", None);
    repo.write("packages/core/dist/core.tgz", "core asset");
    // packages/ui/dist is deliberately missing.

    let mut fake = fake();
    let output = run(&repo, &fake, None, &["release"]);
    assert!(failure(&output).contains("Asset pattern matched nothing"));
    assert!(fake.list_releases().is_empty());
    assert!(fake.list_tags().is_empty());

    fake.stop();
}

#[test]
fn release_rejects_ambiguous_overrides() {
    let repo = two_projects();
    let mut fake = fake();

    assert!(
        failure(&run(&repo, &fake, None, &["release", "--tag", "v1.0.0"])).contains("ambiguous")
    );
    assert!(failure(&run(
        &repo,
        &fake,
        None,
        &["release", "--notes-file", "notes.md"]
    ))
    .contains("ambiguous"));
    assert!(failure(&run(
        &repo,
        &fake,
        None,
        &["release", "--asset", "dist/*.tgz"]
    ))
    .contains("ambiguous"));
    assert!(
        failure(&run(&repo, &fake, None, &["verify", "--tag", "v1.0.0"])).contains("ambiguous")
    );

    fake.stop();
}

#[test]
fn verify_reports_every_project_and_can_show_a_missing_release() {
    let repo = two_projects();
    repo.write("packages/core/index.js", "two\n");
    repo.commit("feat(core): add new api", None);
    repo.write("packages/ui/index.js", "two\n");
    repo.commit("fix(ui): correct layout", None);
    repo.write("packages/core/dist/core.tgz", "core asset");
    repo.write("packages/ui/dist/ui.tgz", "ui asset");

    let mut fake = fake();
    success(&run(&repo, &fake, None, &["release"]));
    let outputs_path = output_file(&repo);
    let output = run(&repo, &fake, Some(&outputs_path), &["verify"]);
    success(&output);
    let value = json(&output);
    assert_eq!(project(&value, "core")["exists"], true);
    assert_eq!(project(&value, "ui")["exists"], true);
    assert_eq!(outputs(&outputs_path)["exists"], "true");

    let release_client = client(&fake);
    let release = release_client
        .get_release_by_tag("ui-v1.0.1")
        .expect("lookup")
        .expect("release");
    release_client.delete_release(release.id).expect("delete");

    let output = run(&repo, &fake, Some(&outputs_path), &["verify"]);
    success(&output);
    let value = json(&output);
    assert_eq!(project(&value, "core")["exists"], true);
    assert_eq!(project(&value, "ui")["exists"], false);
    assert_eq!(outputs(&outputs_path)["exists"], "false");

    fake.stop();
}

#[test]
fn unreserve_without_a_tag_drops_the_next_tag_of_each_project() {
    let repo = two_projects();
    repo.write("packages/core/index.js", "two\n");
    repo.write("packages/ui/index.js", "two\n");
    repo.commit("feat: change both", None);

    let mut fake = fake();
    success(&run(&repo, &fake, None, &["reserve"]));
    assert!(fake.list_tags().contains(&"core-v1.1.0".to_string()));
    assert!(fake.list_tags().contains(&"ui-v1.1.0".to_string()));

    // The workflow fetches reserved tags before unreserving; mirror that here.
    repo.tag("core-v1.1.0");
    repo.tag("ui-v1.1.0");

    let output = run(&repo, &fake, None, &["unreserve"]);
    success(&output);
    assert!(fake.list_tags().is_empty());

    fake.stop();
}

#[test]
fn single_version_mode_still_works_without_a_config() {
    let repo = Repo::new();
    repo.write(
        "package.json",
        "{\n  \"name\": \"solo\",\n  \"version\": \"1.0.0\"\n}\n",
    );
    repo.commit("feat: first", None);
    repo.tag("v1.0.0");
    repo.write("file.txt", "one\n");
    repo.commit("fix: a bug", None);

    let mut fake = fake();
    let output = run(&repo, &fake, None, &["plan", "--json"]);
    success(&output);
    let value = json(&output);
    assert_eq!(value["version"], "1.0.1");
    assert_eq!(value["tag"], "v1.0.1");
    assert_eq!(value["changed"], true);

    success(&run(&repo, &fake, None, &["reserve"]));
    assert!(fake.list_tags().contains(&"v1.0.1".to_string()));

    success(&run(
        &repo,
        &fake,
        None,
        &["version", "--adapter", "npm", "--version", "1.2.3"],
    ));
    let package: Value =
        serde_json::from_str(&fs::read_to_string(repo.path().join("package.json")).unwrap())
            .expect("package json");
    assert_eq!(package["version"], "1.2.3");

    fake.stop();
}

#[test]
fn config_is_discovered_from_a_subdirectory_and_project_is_required_without_one() {
    let repo = two_projects();
    repo.write("packages/core/index.js", "two\n");
    repo.commit("feat(core): add new api", None);
    let mut fake = fake();

    let output = run(
        &repo,
        &fake,
        None,
        &[
            "--cwd",
            "packages/core",
            "plan",
            "--json",
            "--project",
            "core",
        ],
    );
    success(&output);
    let value = json(&output);
    assert_eq!(value["version"], "1.1.0");
    assert_eq!(value["tag"], "core-v1.1.0");

    let solo = Repo::new();
    solo.write("file.txt", "one\n");
    solo.commit("feat: first", None);
    let output = run(&solo, &fake, None, &["plan", "--project", "core"]);
    assert!(failure(&output).contains("--project requires"));

    fake.stop();
}

#[test]
fn release_without_notes_keeps_the_legacy_empty_body_for_single_repos() {
    let repo = Repo::new();
    repo.write("file.txt", "one\n");
    repo.commit("feat: first", None);
    repo.tag("v1.0.0");
    repo.write("file.txt", "two\n");
    repo.commit("fix: a bug", None);

    let mut fake = fake();
    success(&run(&repo, &fake, None, &["release"]));
    let release = fake_release(&fake, "v1.0.1");
    assert_eq!(release.body.as_deref().unwrap_or(""), "");

    fake.stop();
}

#[test]
fn a_single_selected_project_gets_generated_release_notes() {
    let repo = two_projects();
    repo.write("packages/core/index.js", "two\n");
    repo.commit("feat(core): add new api", None);
    repo.write("packages/core/dist/core.tgz", "core asset");

    let mut fake = fake();
    success(&run(&repo, &fake, None, &["release", "--project", "core"]));

    let core = fake_release(&fake, "core-v1.1.0");
    assert!(core
        .body
        .as_deref()
        .unwrap_or_default()
        .contains("feat(core): add new api"));
    assert_eq!(fake.list_releases().len(), 1);

    fake.stop();
}

fn version_repo(ui_adapter: bool) -> Repo {
    let repo = Repo::new();
    let mut ui = json!({"path": "packages/ui"});
    if ui_adapter {
        ui["adapter"] = json!("text");
        ui["file"] = json!("VERSION");
    }
    repo.write_config(json!({
        "projects": {
            "core": {"path": "packages/core", "adapter": "npm", "file": "package.json"},
            "ui": ui
        }
    }));
    repo.write(
        "packages/core/package.json",
        "{\n  \"name\": \"core\",\n  \"version\": \"1.0.0\"\n}\n",
    );
    repo.write("packages/core/index.js", "one\n");
    repo.write("packages/ui/VERSION", "1.0.0\n");
    repo.write("packages/ui/index.js", "one\n");
    repo.commit("feat: initial packages", None);
    repo.tag("core-v1.0.0");
    repo.tag("ui-v1.0.0");
    repo
}

#[test]
fn version_skips_unchanged_projects_that_have_no_adapter() {
    let repo = version_repo(false);
    repo.write("packages/core/index.js", "two\n");
    repo.commit("feat(core): add new api", None);

    let mut fake = fake();
    success(&run(&repo, &fake, None, &["version"]));

    let package: Value = serde_json::from_str(
        &fs::read_to_string(repo.path().join("packages/core/package.json")).unwrap(),
    )
    .expect("package json");
    assert_eq!(package["version"], "1.1.0");
    assert_eq!(
        fs::read_to_string(repo.path().join("packages/ui/VERSION")).unwrap(),
        "1.0.0\n"
    );

    fake.stop();
}

#[test]
fn version_writes_nothing_when_a_changed_project_has_no_adapter() {
    let repo = version_repo(false);
    repo.write("packages/core/index.js", "two\n");
    repo.commit("feat(core): add new api", None);
    repo.write("packages/ui/index.js", "two\n");
    repo.commit("fix(ui): correct layout", None);

    let mut fake = fake();
    let output = run(&repo, &fake, None, &["version"]);
    assert!(failure(&output).contains("project ui has no adapter"));

    let package: Value = serde_json::from_str(
        &fs::read_to_string(repo.path().join("packages/core/package.json")).unwrap(),
    )
    .expect("package json");
    assert_eq!(package["version"], "1.0.0");

    fake.stop();
}

#[test]
fn reserve_rolls_back_earlier_tags_when_a_later_project_cannot_be_planned() {
    let repo = two_projects();
    repo.write("packages/core/index.js", "two\n");
    repo.commit("feat(core): add new api", None);
    repo.write("packages/ui/index.js", "two\n");
    repo.commit("fix(ui): correct layout", Some("Release-As: not-a-version"));

    let mut fake = fake();
    let output = run(&repo, &fake, None, &["reserve"]);
    assert!(failure(&output).contains("not valid semver"));
    assert!(fake.list_tags().is_empty());

    fake.stop();
}

#[test]
fn release_reports_no_changes_when_every_project_is_unchanged() {
    let repo = two_projects();
    repo.write("README.md", "docs only\n");
    repo.commit("docs: explain the packages", None);
    let mut fake = fake();
    let outputs_path = output_file(&repo);

    let output = run(
        &repo,
        &fake,
        Some(&outputs_path),
        &["release", "--target", &repo.head()],
    );
    success(&output);
    assert!(String::from_utf8_lossy(&output.stdout).contains("no changed projects"));
    assert!(fake.list_releases().is_empty());
    assert!(fake.list_tags().is_empty());

    let outputs = outputs(&outputs_path);
    assert_eq!(outputs["changed"], "false");
    assert_eq!(outputs["releases"], "[]");

    fake.stop();
}
