use continuous_release::release::{
    CreateRelease, ReleaseClient, ReleaseClientOptions, TagConflictError, UpdateRelease,
};
use httpmock::prelude::*;
use serde_json::json;
use std::fs;
use tempfile::TempDir;

fn client(server: &MockServer) -> ReleaseClient {
    ReleaseClient::new(ReleaseClientOptions {
        token: "t".to_string(),
        repository: "o/r".to_string(),
        api_url: Some(server.base_url()),
    })
    .expect("client")
}

fn release_json(server: &MockServer) -> serde_json::Value {
    json!({
        "id": 7,
        "tag_name": "v1.2.3",
        "name": "v1.2.3",
        "body": "",
        "draft": false,
        "prerelease": false,
        "html_url": "https://github.com/o/r/releases/tag/v1.2.3",
        "upload_url": format!("{}/upload{{?name,label}}", server.base_url()),
        "assets": []
    })
}

#[test]
fn returns_none_when_the_release_is_missing() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(GET).path("/repos/o/r/releases/tags/v1.2.3");
        then.status(404).body("not found");
    });

    let client = client(&server);
    assert!(client
        .get_release_by_tag("v1.2.3")
        .expect("lookup")
        .is_none());
    mock.assert();
}

#[test]
fn creates_a_release_with_the_expected_payload() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(POST)
            .path("/repos/o/r/releases")
            .body_includes("\"tag_name\":\"v1.2.3\"")
            .body_includes("\"target_commitish\":\"abc\"");
        then.status(201).json_body(release_json(&server));
    });

    let client = client(&server);
    let release = client
        .create_release(&CreateRelease {
            tag: "v1.2.3",
            target: Some("abc"),
            name: Some("v1.2.3"),
            body: "notes",
            prerelease: false,
            draft: true,
        })
        .expect("create");

    assert_eq!(release.id, 7);
    mock.assert();
}

#[test]
fn updates_an_existing_release() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(PATCH)
            .path("/repos/o/r/releases/7")
            .body_includes("\"body\":\"new notes\"")
            .body_includes("\"prerelease\":true");
        then.status(200).json_body(json!({
            "id": 7,
            "tag_name": "v1.2.3",
            "html_url": "https://github.com/o/r/releases/tag/v1.2.3",
            "upload_url": "https://uploads.test/upload{?name,label}",
            "body": "new notes",
            "prerelease": true,
            "draft": false,
            "assets": []
        }));
    });

    let client = client(&server);
    let release = client
        .update_release(
            7,
            &UpdateRelease {
                body: Some("new notes"),
                prerelease: Some(true),
                ..UpdateRelease::default()
            },
        )
        .expect("update");

    assert!(release.prerelease);
    mock.assert();
}

#[test]
fn reserves_a_version_by_creating_its_tag_ref() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(POST)
            .path("/repos/o/r/git/refs")
            .body_includes("\"ref\":\"refs/tags/v1.2.3\"")
            .body_includes("\"sha\":\"abc\"");
        then.status(201).json_body(json!({
            "ref": "refs/tags/v1.2.3",
            "object": { "sha": "abc", "type": "commit" }
        }));
    });

    let client = client(&server);
    assert!(client.reserve_tag("v1.2.3", "abc").expect("reserve"));
    mock.assert();
}

#[test]
fn treats_a_tag_on_the_same_commit_as_idempotent() {
    let server = MockServer::start();
    let create = server.mock(|when, then| {
        when.method(POST).path("/repos/o/r/git/refs");
        then.status(422).body("Reference already exists");
    });
    let lookup = server.mock(|when, then| {
        when.method(GET).path("/repos/o/r/git/ref/tags/v1.2.3");
        then.status(200).json_body(json!({
            "ref": "refs/tags/v1.2.3",
            "object": { "sha": "abc", "type": "commit" }
        }));
    });

    let client = client(&server);
    assert!(!client.reserve_tag("v1.2.3", "abc").expect("reserve"));
    create.assert();
    lookup.assert();
}

#[test]
fn reports_a_conflict_when_the_tag_points_at_another_commit() {
    let server = MockServer::start();
    let create = server.mock(|when, then| {
        when.method(POST).path("/repos/o/r/git/refs");
        then.status(422).body("Reference already exists");
    });
    let lookup = server.mock(|when, then| {
        when.method(GET).path("/repos/o/r/git/ref/tags/v1.2.3");
        then.status(200).json_body(json!({
            "ref": "refs/tags/v1.2.3",
            "object": { "sha": "other", "type": "commit" }
        }));
    });

    let client = client(&server);
    let error = client.reserve_tag("v1.2.3", "abc").expect_err("conflict");
    let conflict = error
        .downcast_ref::<TagConflictError>()
        .expect("tag conflict error");
    assert_eq!(conflict.actual, "other");
    create.assert();
    lookup.assert();
}

#[test]
fn peels_an_annotated_tag_to_its_commit() {
    let server = MockServer::start();
    let reference = server.mock(|when, then| {
        when.method(GET).path("/repos/o/r/git/ref/tags/v1.2.3");
        then.status(200).json_body(json!({
            "ref": "refs/tags/v1.2.3",
            "object": { "sha": "tag-object", "type": "tag" }
        }));
    });
    let annotated = server.mock(|when, then| {
        when.method(GET).path("/repos/o/r/git/tags/tag-object");
        then.status(200).json_body(json!({
            "object": { "sha": "commit-sha", "type": "commit" }
        }));
    });

    let client = client(&server);
    assert_eq!(
        client.resolve_tag_commit("v1.2.3").expect("resolve"),
        Some("commit-sha".to_string())
    );
    reference.assert();
    annotated.assert();
}

#[test]
fn replaces_a_same_named_asset_when_uploading() {
    let server = MockServer::start();
    let dir = TempDir::new().expect("temp dir");
    let asset_path = dir.path().join("app.tgz");
    fs::write(&asset_path, "tarball").expect("write");

    let assets = server.mock(|when, then| {
        when.method(GET).path("/repos/o/r/releases/7/assets");
        then.status(200).json_body(json!([
            {
                "id": 9,
                "name": "app.tgz",
                "size": 7,
                "content_type": "application/gzip",
                "browser_download_url": ""
            }
        ]));
    });
    let delete = server.mock(|when, then| {
        when.method(DELETE).path("/repos/o/r/releases/assets/9");
        then.status(204);
    });
    let upload = server.mock(|when, then| {
        when.method(POST)
            .path("/upload")
            .query_param("name", "app.tgz");
        then.status(201).json_body(json!({
            "id": 10,
            "name": "app.tgz",
            "size": 7,
            "content_type": "application/gzip",
            "browser_download_url": ""
        }));
    });

    let client = client(&server);
    let release: continuous_release::release::Release =
        serde_json::from_value(release_json(&server)).expect("release");
    let asset = client
        .upload_asset(&release, &asset_path, None)
        .expect("upload");

    assert_eq!(asset.id, 10);
    assets.assert();
    delete.assert();
    upload.assert();
}
