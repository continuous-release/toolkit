use continuous_release::notes::{render_notes, NotesOptions};
use continuous_release::version::parse_commit;

fn commit(
    hash: &str,
    subject: &str,
    body: &str,
) -> continuous_release::version::ConventionalCommit {
    parse_commit(hash, subject, body)
}

#[test]
fn groups_commits_and_links_the_compare_range() {
    let commits = vec![
        commit("aaaaaaaa1", "feat(api): add endpoint", ""),
        commit("bbbbbbbb2", "fix: correct timeout", ""),
        commit("cccccccc3", "docs: explain usage", ""),
    ];
    let notes = render_notes(&NotesOptions {
        commits: &commits,
        previous_tag: Some("v1.2.3"),
        version: "1.3.0",
        repository: Some("continuous-release/nx"),
        tag_prefix: "v",
    });

    assert!(notes.contains("### Features"));
    assert!(notes.contains("### Bug Fixes"));
    assert!(notes.contains("### Documentation"));
    assert!(notes.contains("feat(api): add endpoint (aaaaaaa)"));
    assert!(notes.contains(
        "**Full Changelog**: https://github.com/continuous-release/nx/compare/v1.2.3...v1.3.0"
    ));
}

#[test]
fn adds_a_breaking_changes_section() {
    let commits = vec![
        commit("aaaaaaaa1", "feat!: drop v1", ""),
        commit("bbbbbbbb2", "fix: adjust", "BREAKING CHANGE: removed flag"),
    ];
    let notes = render_notes(&NotesOptions {
        commits: &commits,
        previous_tag: Some("v1.0.0"),
        version: "2.0.0",
        repository: Some("continuous-release/bazel"),
        tag_prefix: "v",
    });

    assert!(notes.contains("### ⚠ BREAKING CHANGES"));
    assert!(notes.contains("feat!: drop v1"));
    assert!(notes.contains("fix: adjust"));
}

#[test]
fn uses_a_commits_link_for_the_first_release() {
    let commits = vec![commit("aaaaaaaa1", "feat: first", "")];
    let notes = render_notes(&NotesOptions {
        commits: &commits,
        previous_tag: None,
        version: "0.1.0",
        repository: Some("continuous-release/nix"),
        tag_prefix: "v",
    });

    assert!(notes
        .contains("**Full Changelog**: https://github.com/continuous-release/nix/commits/v0.1.0"));
}

#[test]
fn skips_merge_and_release_commits() {
    let commits = vec![
        commit("aaaaaaaa1", "Merge branch main", ""),
        commit("bbbbbbbb2", "chore(release): v1.0.0", ""),
        commit("cccccccc3", "feat: real change", ""),
    ];
    let notes = render_notes(&NotesOptions {
        commits: &commits,
        previous_tag: Some("v1.0.0"),
        version: "1.1.0",
        repository: None,
        tag_prefix: "v",
    });

    assert!(!notes.contains("Merge branch main"));
    assert!(!notes.contains("chore(release)"));
    assert!(notes.contains("feat: real change"));
}

#[test]
fn omits_the_changelog_link_without_a_repository() {
    let commits = vec![commit("aaaaaaaa1", "feat: first", "")];
    let notes = render_notes(&NotesOptions {
        commits: &commits,
        previous_tag: None,
        version: "0.1.0",
        repository: None,
        tag_prefix: "v",
    });

    assert!(!notes.contains("Full Changelog"));
}
