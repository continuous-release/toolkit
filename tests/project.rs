use continuous_release::project::ProjectSet;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

fn root() -> TempDir {
    let dir = TempDir::new().expect("temp dir");
    for path in [
        "packages/core",
        "packages/ui",
        "packages/rust",
        "shared",
        "nested",
    ] {
        fs::create_dir_all(dir.path().join(path)).expect("create project dir");
    }
    dir
}

fn load(root: &TempDir, config: Value) -> anyhow::Result<ProjectSet> {
    let path = root.path().join("cr.json");
    fs::write(&path, serde_json::to_string_pretty(&config).expect("json")).expect("write config");
    ProjectSet::load(&path, root.path())
}

fn err(root: &TempDir, config: Value) -> String {
    format!("{:#}", load(root, config).expect_err("config should fail"))
}

#[test]
fn resolves_defaults_for_a_minimal_project() {
    let root = root();
    let set = load(
        &root,
        json!({"projects": {"core": {"path": "packages/core"}}}),
    )
    .expect("load");

    assert_eq!(set.shared_paths, Vec::<String>::new());
    assert_eq!(set.projects.len(), 1);
    let project = &set.projects[0];
    assert_eq!(project.name, "core");
    assert_eq!(project.relative_path, "packages/core");
    assert_eq!(project.path, root.path().join("packages/core"));
    assert_eq!(project.tag_prefix, "core-v");
    assert_eq!(project.paths, vec!["packages/core".to_string()]);
    assert_eq!(project.adapter, None);
    assert_eq!(project.file, None);
    assert!(project.artifacts.is_empty());
    assert_eq!(set.config_path, root.path().join("cr.json"));
}

#[test]
fn resolves_explicit_fields_and_shared_paths() {
    let root = root();
    let set = load(
        &root,
        json!({
            "independent": true,
            "sharedPaths": ["shared", "./shared"],
            "projects": {
                "core": {
                    "path": "packages/core",
                    "tagPrefix": "core/v",
                    "adapter": "npm",
                    "file": "package.json",
                    "artifacts": ["dist/*.tgz"],
                    "paths": ["packages/core", "shared"]
                }
            }
        }),
    )
    .expect("load");

    assert_eq!(set.shared_paths, vec!["shared".to_string()]);
    let project = &set.projects[0];
    assert_eq!(project.tag_prefix, "core/v");
    assert_eq!(project.adapter.expect("adapter").name(), "npm");
    assert_eq!(project.file.as_deref(), Some("package.json"));
    assert_eq!(project.artifacts, vec!["dist/*.tgz".to_string()]);
    assert_eq!(
        set.filters_for(project),
        vec!["packages/core".to_string(), "shared".to_string()]
    );
}

#[test]
fn permits_a_project_at_the_repository_root() {
    let root = root();
    let set = load(&root, json!({"projects": {"root": {"path": "."}}})).expect("load");
    let project = &set.projects[0];
    assert_eq!(project.relative_path, ".");
    assert_eq!(project.path, root.path());
    assert_eq!(project.paths, vec![".".to_string()]);
}

#[test]
fn sorts_projects_by_name() {
    let root = root();
    let set = load(
        &root,
        json!({
            "projects": {
                "ui": {"path": "packages/ui"},
                "core": {"path": "packages/core"}
            }
        }),
    )
    .expect("load");
    let names: Vec<&str> = set.projects.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, vec!["core", "ui"]);
}

#[test]
fn rejects_fixed_versions_and_empty_projects() {
    let root = root();
    assert!(err(
        &root,
        json!({"independent": false, "projects": {"core": {"path": "packages/core"}}})
    )
    .contains("independent versioning is required"));
    assert!(err(&root, json!({"projects": {}})).contains("projects cannot be empty"));
}

#[test]
fn rejects_unknown_fields() {
    let root = root();
    assert!(err(
        &root,
        json!({"projects": {"core": {"path": "packages/core", "adapterr": "npm"}}})
    )
    .contains("unknown field"));
    assert!(err(
        &root,
        json!({"project": {"core": {"path": "packages/core"}}})
    )
    .contains("unknown field"));
}

#[test]
fn rejects_invalid_project_names() {
    let root = root();
    for name in ["", "a/b", "a b", "a:b", ".hidden", "-core", "a,b"] {
        let config = json!({"projects": {(name): {"path": "packages/core"}}});
        assert!(err(&root, config).contains("project name"), "name {name:?}");
    }
}

#[test]
fn rejects_duplicate_and_overlapping_tag_prefixes() {
    let root = root();
    let duplicate = json!({
        "projects": {
            "core": {"path": "packages/core", "tagPrefix": "release-v"},
            "ui": {"path": "packages/ui", "tagPrefix": "release-v"}
        }
    });
    assert!(err(&root, duplicate).contains("overlap"));

    let nested = json!({
        "projects": {
            "core": {"path": "packages/core", "tagPrefix": "core-v"},
            "core2": {"path": "packages/ui", "tagPrefix": "core-v2"}
        }
    });
    assert!(err(&root, nested).contains("overlap"));

    let shared = json!({
        "projects": {
            "core": {"path": "packages/core", "tagPrefix": "v"},
            "ui": {"path": "packages/ui", "tagPrefix": "v2"}
        }
    });
    assert!(err(&root, shared).contains("overlap"));
}

#[test]
fn rejects_invalid_tag_prefixes() {
    let root = root();
    for prefix in [
        "",
        "bad prefix",
        "core..v",
        "-core-v",
        "core/v/",
        "a:b",
        "a~b",
        "a[",
        "x.lock",
        ".core-v",
        "core/.hidden",
        "core.v.",
        "@",
    ] {
        let config = json!({
            "projects": {"core": {"path": "packages/core", "tagPrefix": prefix}}
        });
        let message = err(&root, config);
        assert!(
            message.contains("tag prefix"),
            "prefix {prefix:?}: {message}"
        );
    }
}

#[test]
fn rejects_unsafe_or_missing_project_paths() {
    let root = root();
    assert!(
        err(&root, json!({"projects": {"core": {"path": "/tmp/core"}}}))
            .contains("must be relative")
    );
    assert!(
        err(&root, json!({"projects": {"core": {"path": "../core"}}})).contains("cannot contain")
    );
    assert!(err(
        &root,
        json!({"projects": {"core": {"path": "packages/missing"}}})
    )
    .contains("does not exist"));
}

#[test]
fn rejects_bad_adapters_and_files() {
    let root = root();
    assert!(err(
        &root,
        json!({"projects": {"core": {"path": "packages/core", "adapter": "pypi"}}})
    )
    .contains("Unknown adapter"));
    assert!(err(
        &root,
        json!({"projects": {"core": {"path": "packages/core", "file": "package.json"}}})
    )
    .contains("file without an adapter"));
    assert!(err(
        &root,
        json!({
            "projects": {
                "core": {"path": "packages/core", "adapter": "npm", "file": "../package.json"}
            }
        })
    )
    .contains("cannot contain"));
    assert!(err(
        &root,
        json!({
            "projects": {
                "core": {"path": "packages/core", "adapter": "npm", "file": "."}
            }
        })
    )
    .contains("project root"));
}

#[test]
fn rejects_pathspec_magic_and_escaping_paths() {
    let root = root();
    assert!(err(
        &root,
        json!({"projects": {"core": {"path": "packages/core", "paths": [":(exclude)x"]}}})
    )
    .contains("pathspec magic"));
    assert!(err(
        &root,
        json!({"projects": {"core": {"path": "packages/core", "paths": ["!x"]}}})
    )
    .contains("pathspec magic"));
    assert!(err(
        &root,
        json!({"projects": {"core": {"path": "packages/core", "artifacts": ["../x.tgz"]}}})
    )
    .contains("cannot contain"));
    assert!(err(
        &root,
        json!({"projects": {"core": {"path": "packages/core", "artifacts": ["/tmp/x.tgz"]}}})
    )
    .contains("must be relative"));
}

#[test]
fn selects_all_subset_and_deduplicates_names() {
    let root = root();
    let set = load(
        &root,
        json!({
            "projects": {
                "core": {"path": "packages/core"},
                "ui": {"path": "packages/ui"},
                "rust": {"path": "packages/rust"}
            }
        }),
    )
    .expect("load");

    let all = set.select(&[]).expect("all");
    assert_eq!(all.len(), 3);

    let subset = set
        .select(&["ui".to_string(), "core".to_string(), "ui".to_string()])
        .expect("subset");
    let names: Vec<&str> = subset.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, vec!["core", "ui"]);

    let message = format!(
        "{:#}",
        set.select(&["missing".to_string()]).expect_err("unknown")
    );
    assert!(message.contains("unknown project"));
    assert!(message.contains("core, rust, ui"));
}

#[test]
fn reads_the_config_file_from_disk() {
    let root = root();
    let path = root.path().join("cr.json");
    fs::write(&path, r#"{"projects":{"core":{"path":"packages/core"}}}"#).expect("write");
    let set = ProjectSet::load(&path, root.path()).expect("load");
    assert_eq!(set.projects[0].name, "core");
}

#[test]
fn missing_config_file_is_reported() {
    let root = root();
    let path: PathBuf = root.path().join("missing.json");
    let message = format!(
        "{:#}",
        ProjectSet::load(&path, root.path()).expect_err("missing")
    );
    assert!(message.contains("reading project config"));
}
