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
    count: usize,
}

impl Repo {
    fn new() -> Self {
        let dir = TempDir::new().expect("temp dir");
        git(dir.path(), &["init", "-q", "-b", "main"]);
        git(dir.path(), &["config", "user.email", "test@example.com"]);
        git(dir.path(), &["config", "user.name", "Test"]);
        Self { dir, count: 0 }
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }

    fn commit(&mut self, message: &str, body: Option<&str>) {
        self.count += 1;
        fs::write(self.path().join("file.txt"), format!("{}\n", self.count)).expect("write");
        git(self.path(), &["add", "."]);
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
    let mut repo = Repo::new();
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
    let mut repo = Repo::new();
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
    let mut repo = Repo::new();
    repo.commit("feat: first", None);
    repo.tag("v1.2.3");
    repo.commit("feat(core): add thing", None);
    let next = compute(&repo);
    assert_eq!(next.version, "1.3.0");
    assert_eq!(next.bump.as_str(), "minor");
}

#[test]
fn bumps_major_for_a_bang() {
    let mut repo = Repo::new();
    repo.commit("feat: first", None);
    repo.tag("v1.2.3");
    repo.commit("refactor!: drop old API", None);
    let next = compute(&repo);
    assert_eq!(next.version, "2.0.0");
    assert_eq!(next.bump.as_str(), "major");
}

#[test]
fn bumps_major_for_a_breaking_footer() {
    let mut repo = Repo::new();
    repo.commit("feat: first", None);
    repo.tag("v1.2.3");
    repo.commit("fix: adjust", Some("BREAKING CHANGE: removed flag"));
    let next = compute(&repo);
    assert_eq!(next.version, "2.0.0");
    assert_eq!(next.bump.as_str(), "major");
}

#[test]
fn releases_a_patch_for_chore_only_commits() {
    let mut repo = Repo::new();
    repo.commit("feat: first", None);
    repo.tag("v1.2.3");
    repo.commit("chore: tidy", None);
    let next = compute(&repo);
    assert_eq!(next.version, "1.2.4");
    assert_eq!(next.bump.as_str(), "patch");
}

#[test]
fn honors_release_as() {
    let mut repo = Repo::new();
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
    let mut repo = Repo::new();
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
    let mut repo = Repo::new();
    repo.commit("feat: first", None);
    repo.tag("release-1.0.0");
    repo.commit("fix: bug", None);
    let next = compute_next_version(&ComputeOptions {
        cwd: repo.path(),
        release_as: None,
        prefix: "release-",
    })
    .expect("version");
    assert_eq!(next.version, "1.0.1");
    assert_eq!(next.tag, "release-1.0.1");
    assert_eq!(next.previous_tag.as_deref(), Some("release-1.0.0"));
}
