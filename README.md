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
| `project` | Load and validate independent per-project versioning config |
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

## Independent project versions

Put `cr.json` at the repository root (or pass `--config`). Without it, `cr`
keeps the single-version behaviour above.

```json
{
  "independent": true,
  "sharedPaths": ["shared/schema.json"],
  "projects": {
    "core": {
      "path": "packages/core",
      "tagPrefix": "core-v",
      "adapter": "npm",
      "file": "package.json",
      "artifacts": ["dist/*.tgz"],
      "paths": ["packages/core", "shared/schema.json"]
    },
    "ui": {
      "path": "packages/ui",
      "adapter": "cargo",
      "file": "Cargo.toml",
      "artifacts": ["target/release/ui"]
    }
  }
}
```

- `path` is required and is relative to the repository root.
- `tagPrefix` defaults to `<project>-v`; prefixes must be unique and must not
  overlap, so `core-v` and `core-v2` are rejected.
- `paths` defaults to `[path]`. A project only releases when a commit since its
  last tag touched one of its paths. `sharedPaths` is added to every project.
- `adapter` and `file` are optional. `file` and artifact globs are relative to
  the project directory; `paths` and `sharedPaths` are relative to the
  repository root.
- Projects whose files did not change are no-ops (`changed: false`). A tag that
  already points at HEAD remains a rerunnable release (`existing: true`).
- `--project <name>` selects a project and is repeatable. With one project
  selected, the flat `version`, `tag`, and `previous_tag` outputs are written as
  before; with several, use the `projects`, `notes_dir`, and `releases` outputs.
- `--release-as`, `--tag`, `--notes-file`, `--asset`, `--adapter`, `--file`, and
  `--version` are rejected when several projects are selected, because they
  would be ambiguous.
- Multi-project `reserve` rolls back only the tags it created in that run if a
  later project fails or is superseded. Multi-project `release` drafts and
  uploads every release before promoting any of them.
- For a multi-project run, `--notes-out` (and `notes --out`) is a directory and
  each changed project gets `<project>.md`.

## CLI

```sh
cr plan --notes-out release-notes.md
cr reserve --notes-out release-notes.md
cr version --adapter npm --version 1.2.3
cr release --repo owner/repo --tag v1.2.3 --notes-file release-notes.md --asset dist/*.tgz
cr verify --repo owner/repo --tag v1.2.3
```

With `cr.json`:

```sh
cr plan --project core --json
cr version                 # uses each project's configured adapter
cr reserve --notes-out notes
cr release                 # uses each project's configured artifacts
cr unreserve               # drops the next tag of every selected project
```

`plan` writes `version`, `tag`, `previous_tag`, `existing`, `bump`, `changed`,
and `notes_path` to `$GITHUB_OUTPUT` when it is set. `reserve` also writes
`superseded`. Multi-project runs write `projects` (a compact JSON array),
`changed`, and `notes_dir`, and `release` writes `releases`.

## Action

```yaml
- id: plan
  uses: continuous-release/toolkit@v0.3.3
  with:
    command: plan
    notes-out: ${{ runner.temp }}/release-notes.md

- run: ./build.sh
  env:
    VERSION: ${{ steps.plan.outputs.version }}

- uses: continuous-release/toolkit@v0.3.3
  with:
    command: release
    tag: ${{ steps.plan.outputs.tag }}
    notes-file: ${{ steps.plan.outputs.notes_path }}
    asset: dist/*.tgz
```

For a monorepo, pass `config` and optionally a comma-separated `project` list.
The action surface is the same as the CLI.

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
