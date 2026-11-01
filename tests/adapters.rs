use continuous_release::adapters::{get_adapter, read_version, write_version};
use std::fs;
use tempfile::TempDir;

#[test]
fn reads_and_writes_an_npm_package_version() {
    let dir = TempDir::new().expect("temp dir");
    fs::write(
        dir.path().join("package.json"),
        r#"{"name":"x","version":"1.2.3"}"#,
    )
    .expect("write");

    let adapter = get_adapter("npm").expect("adapter");
    assert_eq!(
        read_version(dir.path(), adapter, None).expect("read"),
        "1.2.3"
    );
    write_version(dir.path(), adapter, "2.0.0", None).expect("write");

    let text = fs::read_to_string(dir.path().join("package.json")).expect("read");
    let value: serde_json::Value = serde_json::from_str(&text).expect("json");
    assert_eq!(value["version"], "2.0.0");
    assert_eq!(value["name"], "x");
}

#[test]
fn reads_and_writes_a_cargo_package_version() {
    let dir = TempDir::new().expect("temp dir");
    fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"1.2.3\"\nedition = \"2021\"\n",
    )
    .expect("write");

    let adapter = get_adapter("cargo").expect("adapter");
    assert_eq!(
        read_version(dir.path(), adapter, None).expect("read"),
        "1.2.3"
    );
    write_version(dir.path(), adapter, "2.0.0", None).expect("write");

    let text = fs::read_to_string(dir.path().join("Cargo.toml")).expect("read");
    assert!(text.contains("version = \"2.0.0\""));
    assert!(text.contains("name = \"demo\""));
    assert!(text.contains("edition = \"2021\""));
}

#[test]
fn reads_and_writes_a_text_version_file() {
    let dir = TempDir::new().expect("temp dir");
    fs::write(dir.path().join("version.txt"), "1.2.3\n").expect("write");

    let adapter = get_adapter("text").expect("adapter");
    assert_eq!(
        read_version(dir.path(), adapter, None).expect("read"),
        "1.2.3"
    );
    write_version(dir.path(), adapter, "2.0.0", None).expect("write");
    assert_eq!(
        fs::read_to_string(dir.path().join("version.txt")).expect("read"),
        "2.0.0\n"
    );
}

#[test]
fn supports_a_custom_file() {
    let dir = TempDir::new().expect("temp dir");
    fs::write(dir.path().join("VERSION"), "0.1.0\n").expect("write");

    let adapter = get_adapter("text").expect("adapter");
    write_version(dir.path(), adapter, "0.2.0", Some("VERSION")).expect("write");
    assert_eq!(
        read_version(dir.path(), adapter, Some("VERSION")).expect("read"),
        "0.2.0"
    );
}

#[test]
fn rejects_unknown_adapters() {
    let error = get_adapter("maven").expect_err("unknown adapter");
    assert!(error.to_string().contains("Unknown adapter"));
}
