use crate::version::ConventionalCommit;

const SECTIONS: &[(&str, &str)] = &[
    ("feat", "Features"),
    ("fix", "Bug Fixes"),
    ("perf", "Performance Improvements"),
    ("refactor", "Code Refactoring"),
    ("docs", "Documentation"),
    ("build", "Build System"),
    ("ci", "Continuous Integration"),
    ("test", "Tests"),
    ("chore", "Chores"),
    ("revert", "Reverts"),
];

const HIDDEN_TYPES: &[&str] = &["release"];

fn short_hash(hash: &str) -> &str {
    hash.get(..7).unwrap_or(hash)
}

fn is_noise(commit: &ConventionalCommit) -> bool {
    commit.subject.starts_with("Merge ")
        || (commit.kind.as_deref() == Some("chore") && commit.scope.as_deref() == Some("release"))
}

pub struct NotesOptions<'a> {
    pub commits: &'a [ConventionalCommit],
    pub previous_tag: Option<&'a str>,
    pub version: &'a str,
    pub repository: Option<&'a str>,
    pub tag_prefix: &'a str,
}

pub fn render_notes(options: &NotesOptions<'_>) -> String {
    let prefix = options.tag_prefix;
    let commits: Vec<&ConventionalCommit> = options
        .commits
        .iter()
        .filter(|commit| !is_noise(commit))
        .collect();

    let mut lines: Vec<String> = vec!["## What's Changed".to_string(), String::new()];

    let breaking: Vec<&&ConventionalCommit> =
        commits.iter().filter(|commit| commit.breaking).collect();
    if !breaking.is_empty() {
        lines.push("### ⚠ BREAKING CHANGES".to_string());
        lines.push(String::new());
        for commit in breaking {
            lines.push(format!(
                "- {} ({})",
                commit.subject,
                short_hash(&commit.hash)
            ));
        }
        lines.push(String::new());
    }

    for (kind, title) in SECTIONS {
        let matches: Vec<&&ConventionalCommit> = commits
            .iter()
            .filter(|commit| commit.kind.as_deref() == Some(*kind))
            .collect();
        if matches.is_empty() {
            continue;
        }
        lines.push(format!("### {title}"));
        lines.push(String::new());
        for commit in matches {
            lines.push(format!(
                "- {} ({})",
                commit.subject,
                short_hash(&commit.hash)
            ));
        }
        lines.push(String::new());
    }

    let known: Vec<&str> = SECTIONS.iter().map(|(kind, _)| *kind).collect();
    let other: Vec<&&ConventionalCommit> = commits
        .iter()
        .filter(|commit| match commit.kind.as_deref() {
            None => true,
            Some(kind) => !known.contains(&kind) && !HIDDEN_TYPES.contains(&kind),
        })
        .collect();
    if !other.is_empty() {
        lines.push("### Other Changes".to_string());
        lines.push(String::new());
        for commit in other {
            lines.push(format!(
                "- {} ({})",
                commit.subject,
                short_hash(&commit.hash)
            ));
        }
        lines.push(String::new());
    }

    if let Some(repository) = options.repository {
        let tag = format!("{prefix}{}", options.version);
        let link = match options.previous_tag {
            Some(previous) => format!("https://github.com/{repository}/compare/{previous}...{tag}"),
            None => format!("https://github.com/{repository}/commits/{tag}"),
        };
        lines.push(format!("**Full Changelog**: {link}"));
    }

    let mut text = lines.join("\n");
    while text.ends_with('\n') {
        text.pop();
    }
    text.push('\n');
    text
}
