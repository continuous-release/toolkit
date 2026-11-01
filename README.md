# continuous-release toolkit

A single static binary, a library, and a GitHub Action for continuous
releases: every push to `main` produces a release, and tags are the source of
truth. There is no version-bump commit back to `main`, so a release cannot
trigger itself.

The toolkit is written in Rust so a release job downloads one binary instead of
installing a runtime, and so its own build is a normal `cargo build --locked`
artifact.

## Modules

| Module | Purpose |
| --- | --- |
| `version` | Parse conventional commits, find the last tag, and derive the next semver version |
| `notes` | Render grouped release notes with breaking changes and a compare link |
| `adapters` | Read and write versions in `package.json`, `Cargo.toml`, and plain text files |
| `release` | Reserve tags and create, update, and upload assets to GitHub Releases |
| `cr` | The CLI that ties the modules together |

## Contract

- A release is derived from conventional commits since the last `v*` tag.
- `feat` bumps minor, `fix` and other commits bump patch, and a breaking change
  bumps major. The first release is `0.1.0`.
- `Release-As: 1.2.3` in a commit body forces a version.
- The tag is reserved before build and publish. A re-run continues the same
  release, a concurrent run re-plans from the winning tag or reports that it was
  superseded, and a failure before the build drops the reservation.
- Releases are created as drafts, get their assets, then are promoted, so a
  half-uploaded release is never visible.
- PRs build and test only. Only a push to `main` releases.

## CLI

```sh
cr plan --notes-out release-notes.md
cr reserve --notes-out release-notes.md
cr version --adapter npm --version 1.2.3
cr release --repo owner/repo --tag v1.2.3 --notes-file release-notes.md --asset dist/*.tgz
cr verify --repo owner/repo --tag v1.2.3
```

`plan` writes `version`, `tag`, `previous_tag`, `existing`, `bump`, and
`notes_path` to `$GITHUB_OUTPUT` when it is set. `reserve` also writes
`superseded`, and `unreserve` drops a tag that never reached publish.

## Action

```yaml
- id: plan
  uses: continuous-release/toolkit@v0.3.0
  with:
    command: plan
    notes-out: ${{ runner.temp }}/release-notes.md

- run: ./build.sh
  env:
    VERSION: ${{ steps.plan.outputs.version }}

- uses: continuous-release/toolkit@v0.3.0
  with:
    command: release
    tag: ${{ steps.plan.outputs.tag }}
    notes-file: ${{ steps.plan.outputs.notes_path }}
    asset: dist/*.tgz
```

The action downloads the prebuilt binary for the runner. The reusable workflow
in [`continuous-release/.github`](https://github.com/continuous-release/.github)
wraps the same contract for the org's testbeds.

## Development

```sh
cargo test --locked
cargo build --release --locked
```

Release assets are `cr-linux-x86_64` and its `.sha256`, built by the release
workflow with `CR_VERSION` set to the reserved version so `cr --version`
reports it.
