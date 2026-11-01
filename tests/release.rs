use continuous_release::release::{
    CreateRelease, ReleaseClient, ReleaseClientOptions, TagConflictError, UpdateRelease,
};
use fake_github::FakeGitHub;
use std::fs;
use tempfile::TempDir;

fn fake() -> FakeGitHub {
    let mut fake = FakeGitHub::with_default_repo("o", "r");
    fake.start().expect("fake GitHub");
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

fn create_release(client: &ReleaseClient) -> continuous_release::release::Release {
    client
        .create_release(&CreateRelease {
            tag: "v1.2.3",
            target: Some("abc"),
            name: Some("v1.2.3"),
            body: "notes",
            prerelease: false,
            draft: true,
        })
        .expect("create")
}

#[test]
fn returns_none_when_the_release_is_missing() {
    let mut fake = fake();
    let client = client(&fake);

    assert!(client
        .get_release_by_tag("v1.2.3")
        .expect("lookup")
        .is_none());

    fake.stop();
}

#[test]
fn creates_a_release_with_the_expected_payload() {
    let mut fake = fake();
    let client = client(&fake);

    let release = create_release(&client);

    assert_eq!(release.tag_name, "v1.2.3");
    assert_eq!(release.name.as_deref(), Some("v1.2.3"));
    assert_eq!(release.body.as_deref(), Some("notes"));
    assert!(release.draft);
    assert!(!release.prerelease);

    let stored = client
        .get_release_by_tag("v1.2.3")
        .expect("lookup")
        .expect("release");
    assert_eq!(stored.id, release.id);
    assert_eq!(stored.body.as_deref(), Some("notes"));
    assert!(stored.draft);

    fake.stop();
}

#[test]
fn updates_an_existing_release() {
    let mut fake = fake();
    let client = client(&fake);
    let created = create_release(&client);

    let release = client
        .update_release(
            created.id,
            &UpdateRelease {
                body: Some("new notes"),
                prerelease: Some(true),
                ..UpdateRelease::default()
            },
        )
        .expect("update");

    assert_eq!(release.body.as_deref(), Some("new notes"));
    assert!(release.prerelease);

    let stored = client
        .get_release_by_tag("v1.2.3")
        .expect("lookup")
        .expect("release");
    assert_eq!(stored.body.as_deref(), Some("new notes"));
    assert!(stored.prerelease);

    fake.stop();
}

#[test]
fn reserves_a_version_by_creating_its_tag_ref() {
    let mut fake = fake();
    let client = client(&fake);

    assert!(client.reserve_tag("v1.2.3", "abc").expect("reserve"));
    assert!(fake.list_tags().contains(&"v1.2.3".to_string()));

    fake.stop();
}

#[test]
fn treats_a_tag_on_the_same_commit_as_idempotent() {
    let mut fake = fake();
    let client = client(&fake);

    assert!(client.reserve_tag("v1.2.3", "abc").expect("first reserve"));
    assert!(!client.reserve_tag("v1.2.3", "abc").expect("second reserve"));
    assert_eq!(
        client.resolve_tag_commit("v1.2.3").expect("resolve"),
        Some("abc".to_string())
    );

    fake.stop();
}

#[test]
fn reports_a_conflict_when_the_tag_points_at_another_commit() {
    let mut fake = fake();
    let client = client(&fake);

    assert!(client.reserve_tag("v1.2.3", "abc").expect("first reserve"));
    let error = client.reserve_tag("v1.2.3", "def").expect_err("conflict");
    let conflict = error
        .downcast_ref::<TagConflictError>()
        .expect("tag conflict error");
    assert_eq!(conflict.actual, "abc");

    fake.stop();
}

#[test]
fn peels_an_annotated_tag_to_its_commit() {
    let mut fake = fake();
    fake.seed_annotated_tag("v1.2.3", "tag-object", "commit-sha");
    let client = client(&fake);

    assert_eq!(
        client.resolve_tag_commit("v1.2.3").expect("resolve"),
        Some("commit-sha".to_string())
    );

    fake.stop();
}

#[test]
fn replaces_a_same_named_asset_when_uploading() {
    let mut fake = fake();
    let client = client(&fake);
    let dir = TempDir::new().expect("temp dir");
    let asset_path = dir.path().join("app.tgz");
    fs::write(&asset_path, "tarball").expect("write");
    let release = create_release(&client);

    let first = client
        .upload_asset(&release, &asset_path, None)
        .expect("first upload");
    assert_eq!(first.name, "app.tgz");
    assert_eq!(first.size, 7);

    let second = client
        .upload_asset(&release, &asset_path, None)
        .expect("second upload");
    assert_eq!(second.name, "app.tgz");
    assert_ne!(first.id, second.id);

    let assets = client.list_assets(release.id).expect("list assets");
    assert_eq!(assets.len(), 1);
    assert_eq!(assets[0].id, second.id);
    assert_eq!(assets[0].size, 7);

    fake.stop();
}
