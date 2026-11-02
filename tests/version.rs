use continuous_release::version::{compute_next_version, parse_commit, ComputeOptions};
use std::fs;
use std::path::Path;
use std::process::Command;
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
    count: std::cell::Cell<usize>,
}

impl Repo {
    fn new() -> Self {
        let dir = TempDir::new().expect("temp dir");
        git(dir.path(), &["init", "-q", "-b", "main"]);
        git(dir.path(), &["config", "user.email", "test@example.com"]);
        git(dir.path(), &["config", "user.name", "Test"]);
        Self {
            dir,
            count: std::cell::Cell::new(0),
        }
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }

    fn commit(&self, message: &str, body: Option<&str>) {
        self.count.set(self.count.get() + 1);
        fs::write(
            self.path().join("file.txt"),
            format!("{}\n", self.count.get()),
        )
        .expect("write");
        self.commit_staged(message, body);
    }

    fn write(&self, relative: &str, contents: &str) {
        let path = self.path().join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create dirs");
        }
        fs::write(path, contents).expect("write");
    }

    fn commit_staged(&self, message: &str, body: Option<&str>) {
        git(self.path(), &["add", "-A"]);
        let mut args = vec!["commit", "-q", "-m", message];
        if let Some(body) = body {
            args.push("-m");
            args.push(body);
        }
        git(self.path(), &args);
    }

    fn tag(&self, name: &str) {
        git(self.path(), &["tag", name]);
    }
}

fn compute(repo: &Repo) -> continuous_release::version::NextVersion {
    compute_next_version(&ComputeOptions::new(repo.path())).expect("version")
}

#[test]
fn parses_type_scope_and_breaking_bang() {
    let commit = parse_commit("abc", "feat(api)!: drop v1", "");
    assert_eq!(commit.kind.as_deref(), Some("feat"));
    assert_eq!(commit.scope.as_deref(), Some("api"));
    assert!(commit.breaking);
    assert_eq!(commit.description, "drop v1");
}

#[test]
fn parses_a_breaking_footer() {
    let commit = parse_commit("abc", "fix: adjust", "BREAKING CHANGE: removed flag");
    assert_eq!(commit.kind.as_deref(), Some("fix"));
    assert!(commit.breaking);
}

#[test]
fn keeps_non_conventional_subjects() {
    let commit = parse_commit("abc", "Update docs", "");
    assert_eq!(commit.kind, None);
    assert!(!commit.breaking);
}

#[test]
fn starts_at_zero_one_zero_without_a_tag() {
    let repo = Repo::new();
    repo.commit("feat: first", None);
    let next = compute(&repo);
    assert_eq!(next.version, "0.1.0");
    assert_eq!(next.tag, "v0.1.0");
    assert_eq!(next.previous_tag, None);
    assert_eq!(next.bump.as_str(), "minor");
    assert!(!next.existing);
}

#[test]
fn bumps_patch_for_fixes() {
    let repo = Repo::new();
    repo.commit("feat: first", None);
    repo.tag("v1.2.3");
    repo.commit("fix: a bug", None);
    let next = compute(&repo);
    assert_eq!(next.version, "1.2.4");
    assert_eq!(next.bump.as_str(), "patch");
    assert_eq!(next.previous_tag.as_deref(), Some("v1.2.3"));
}

#[test]
fn bumps_minor_for_features() {
    let repo = Repo::new();
    repo.commit("feat: first", None);
    repo.tag("v1.2.3");
    repo.commit("feat(core): add thing", None);
    let next = compute(&repo);
    assert_eq!(next.version, "1.3.0");
    assert_eq!(next.bump.as_str(), "minor");
}

#[test]
fn bumps_major_for_a_bang() {
    let repo = Repo::new();
    repo.commit("feat: first", None);
    repo.tag("v1.2.3");
    repo.commit("refactor!: drop old API", None);
    let next = compute(&repo);
    assert_eq!(next.version, "2.0.0");
    assert_eq!(next.bump.as_str(), "major");
}

#[test]
fn bumps_major_for_a_breaking_footer() {
    let repo = Repo::new();
    repo.commit("feat: first", None);
    repo.tag("v1.2.3");
    repo.commit("fix: adjust", Some("BREAKING CHANGE: removed flag"));
    let next = compute(&repo);
    assert_eq!(next.version, "2.0.0");
    assert_eq!(next.bump.as_str(), "major");
}

#[test]
fn releases_a_patch_for_chore_only_commits() {
    let repo = Repo::new();
    repo.commit("feat: first", None);
    repo.tag("v1.2.3");
    repo.commit("chore: tidy", None);
    let next = compute(&repo);
    assert_eq!(next.version, "1.2.4");
    assert_eq!(next.bump.as_str(), "patch");
}

#[test]
fn honors_release_as() {
    let repo = Repo::new();
    repo.commit("feat: first", None);
    repo.tag("v1.2.3");
    repo.commit("chore: force", Some("Release-As: 3.4.5"));
    let next = compute(&repo);
    assert_eq!(next.version, "3.4.5");
    assert_eq!(next.tag, "v3.4.5");
    assert_eq!(next.release_as.as_deref(), Some("3.4.5"));
}

#[test]
fn reuses_a_tag_already_on_head() {
    let repo = Repo::new();
    repo.commit("feat: first", None);
    repo.tag("v1.2.3");
    let next = compute(&repo);
    assert_eq!(next.version, "1.2.3");
    assert_eq!(next.tag, "v1.2.3");
    assert!(next.existing);
    assert_eq!(next.bump.as_str(), "none");
}

#[test]
fn supports_a_custom_tag_prefix() {
    let repo = Repo::new();
    repo.commit("feat: first", None);
    repo.tag("release-1.0.0");
    repo.commit("fix: bug", None);
    let next = compute_next_version(&ComputeOptions {
        cwd: repo.path(),
        release_as: None,
        prefix: "release-",
        paths: &[],
    })
    .expect("version");
    assert_eq!(next.version, "1.0.1");
    assert_eq!(next.tag, "release-1.0.1");
    assert_eq!(next.previous_tag.as_deref(), Some("release-1.0.0"));
}

fn compute_paths(
    repo: &Repo,
    prefix: &str,
    paths: &[&str],
) -> continuous_release::version::NextVersion {
    let paths: Vec<String> = paths.iter().map(|path| path.to_string()).collect();
    compute_next_version(&ComputeOptions {
        cwd: repo.path(),
        release_as: None,
        prefix,
        paths: &paths,
    })
    .expect("version")
}

#[test]
fn a_path_filtered_project_is_unchanged_without_matching_commits() {
    let repo = Repo::new();
    repo.write("packages/core/index.js", "one\n");
    repo.commit_staged("feat: add core", None);
    repo.tag("core-v1.0.0");

    repo.write("packages/ui/index.js", "one\n");
    repo.commit_staged("feat: add ui", None);

    let next = compute_paths(&repo, "core-v", &["packages/core"]);
    assert!(!next.changed);
    assert_eq!(next.version, "1.0.0");
    assert_eq!(next.tag, "core-v1.0.0");
    assert_eq!(next.bump.as_str(), "none");
    assert!(!next.existing);
}

#[test]
fn a_path_filtered_project_bumps_only_for_its_own_changes() {
    let repo = Repo::new();
    repo.write("packages/core/index.js", "one\n");
    repo.write("packages/ui/index.js", "one\n");
    repo.commit_staged("feat: add packages", None);
    repo.tag("core-v1.0.0");
    repo.tag("ui-v1.0.0");

    repo.write("packages/core/index.js", "two\n");
    repo.commit_staged("feat(core): new api", None);

    let core = compute_paths(&repo, "core-v", &["packages/core"]);
    assert!(core.changed);
    assert_eq!(core.version, "1.1.0");
    assert_eq!(core.tag, "core-v1.1.0");

    let ui = compute_paths(&repo, "ui-v", &["packages/ui"]);
    assert!(!ui.changed);
    assert_eq!(ui.version, "1.0.0");
    assert_eq!(ui.tag, "ui-v1.0.0");
}

#[test]
fn a_shared_path_bumps_every_filtered_project() {
    let repo = Repo::new();
    repo.write("packages/core/index.js", "one\n");
    repo.write("packages/ui/index.js", "one\n");
    repo.commit_staged("feat: add packages", None);
    repo.tag("core-v1.0.0");
    repo.tag("ui-v1.0.0");

    repo.write("shared/schema.json", "{}\n");
    repo.commit_staged("fix: update shared schema", None);

    let core = compute_paths(&repo, "core-v", &["packages/core", "shared"]);
    let ui = compute_paths(&repo, "ui-v", &["packages/ui", "shared"]);
    assert!(core.changed);
    assert!(ui.changed);
    assert_eq!(core.version, "1.0.1");
    assert_eq!(ui.version, "1.0.1");
}

#[test]
fn release_as_only_forces_a_project_whose_commit_matches() {
    let repo = Repo::new();
    repo.write("packages/core/index.js", "one\n");
    repo.write("packages/ui/index.js", "one\n");
    repo.commit_staged("feat: add packages", None);
    repo.tag("core-v1.0.0");
    repo.tag("ui-v1.0.0");

    repo.write("packages/core/index.js", "two\n");
    repo.commit_staged("chore: force core", Some("Release-As: 2.3.4"));

    let core = compute_paths(&repo, "core-v", &["packages/core"]);
    assert_eq!(core.version, "2.3.4");
    assert_eq!(core.release_as.as_deref(), Some("2.3.4"));
    assert!(core.changed);

    let ui = compute_paths(&repo, "ui-v", &["packages/ui"]);
    assert!(!ui.changed);
    assert_eq!(ui.version, "1.0.0");
    assert_eq!(ui.release_as, None);
}

#[test]
fn a_forced_version_releases_an_otherwise_unchanged_project() {
    let repo = Repo::new();
    repo.write("packages/core/index.js", "one\n");
    repo.commit_staged("feat: add core", None);
    repo.tag("core-v1.0.0");
    repo.write("packages/ui/index.js", "one\n");
    repo.commit_staged("feat: add ui", None);

    let paths = vec!["packages/core".to_string()];
    let next = compute_next_version(&ComputeOptions {
        cwd: repo.path(),
        release_as: Some("5.0.0"),
        prefix: "core-v",
        paths: &paths,
    })
    .expect("version");
    assert!(next.changed);
    assert_eq!(next.version, "5.0.0");
    assert_eq!(next.tag, "core-v5.0.0");
    assert!(next.release_as.is_some());
}

#[test]
fn a_path_filtered_project_without_a_matching_first_commit_is_unchanged() {
    let repo = Repo::new();
    repo.write("packages/ui/index.js", "one\n");
    repo.commit_staged("feat: add ui", None);

    let next = compute_paths(&repo, "core-v", &["packages/core"]);
    assert!(!next.changed);
    assert_eq!(next.version, "0.0.0");
    assert_eq!(next.tag, "core-v0.0.0");
    assert_eq!(next.previous_tag, None);
}

#[test]
fn a_path_filtered_first_release_starts_at_zero_one_zero() {
    let repo = Repo::new();
    repo.write("packages/core/index.js", "one\n");
    repo.commit_staged("feat: add core", None);

    let next = compute_paths(&repo, "core-v", &["packages/core"]);
    assert!(next.changed);
    assert_eq!(next.version, "0.1.0");
    assert_eq!(next.tag, "core-v0.1.0");
}

#[test]
fn a_tag_at_head_is_existing_and_changed_for_a_path_filtered_project() {
    let repo = Repo::new();
    repo.write("packages/core/index.js", "one\n");
    repo.commit_staged("feat: add core", None);
    repo.tag("core-v1.0.0");

    let next = compute_paths(&repo, "core-v", &["packages/core"]);
    assert!(next.existing);
    assert!(next.changed);
    assert_eq!(next.version, "1.0.0");
    assert_eq!(next.tag, "core-v1.0.0");
}

#[test]
fn non_conventional_commits_still_bump_a_changed_project() {
    let repo = Repo::new();
    repo.write("packages/core/index.js", "one\n");
    repo.commit_staged("feat: add core", None);
    repo.tag("core-v1.0.0");

    repo.write("packages/core/index.js", "two\n");
    repo.commit_staged("Update the core", None);

    let next = compute_paths(&repo, "core-v", &["packages/core"]);
    assert!(next.changed);
    assert_eq!(next.version, "1.0.1");
    assert_eq!(next.bump.as_str(), "patch");
}

#[test]
fn single_version_mode_is_always_changed() {
    let repo = Repo::new();
    repo.commit("feat: first", None);
    repo.tag("v1.0.0");
    let next = compute(&repo);
    assert!(next.changed);
}

#[test]
fn an_existing_release_uses_the_highest_tag_at_head() {
    let repo = Repo::new();
    repo.commit("feat: first", None);
    repo.tag("v1.0.0");
    repo.tag("v1.0.1");

    let next = compute(&repo);
    assert!(next.existing);
    assert_eq!(next.version, "1.0.1");
    assert_eq!(next.tag, "v1.0.1");
}
