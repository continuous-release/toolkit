# continuous-release toolkit

Libraries, a CLI, and a GitHub Action for continuous releases: every push to
`main` produces a release, and tags are the source of truth. There is no
version-bump commit back to `main`, so a release cannot trigger itself.

## Packages

| Package | Purpose |
| --- | --- |
| `@continuous-release/version` | Parse conventional commits, find the last tag, and derive the next semver version |
| `@continuous-release/notes` | Render grouped release notes with breaking changes and a compare link |
| `@continuous-release/adapters` | Read and write versions in `package.json`, `Cargo.toml`, and plain text files |
| `@continuous-release/release` | Create, update, and upload assets to GitHub Releases, idempotently |
| `@continuous-release/cli` | The `cr` command that ties the packages together |

## Contract

- A release is derived from conventional commits since the last `v*` tag.
- `feat` bumps minor, `fix` and other commits bump patch, and a breaking change
  bumps major. The first release is `0.1.0`.
- `Release-As: 1.2.3` in a commit body forces a version.
- If the tag already points at `HEAD`, the release is updated rather than
  recreated, which makes re-runs safe.
- PRs build and test only. Only a push to `main` releases.

## CLI

```sh
cr plan --notes-out release-notes.md
cr version --adapter npm --version 1.2.3
cr release --repo owner/repo --tag v1.2.3 --notes-file release-notes.md --asset dist/*.tgz
cr verify --repo owner/repo --tag v1.2.3
```

`plan` writes `version`, `tag`, `previous_tag`, `existing`, `bump`, and
`notes_path` to `$GITHUB_OUTPUT` when it is set.

## Action

```yaml
- id: plan
  uses: continuous-release/toolkit@v0.1.0
  with:
    command: plan
    notes-out: ${{ runner.temp }}/release-notes.md

- run: ./build.sh
  env:
    VERSION: ${{ steps.plan.outputs.version }}

- uses: continuous-release/toolkit@v0.1.0
  with:
    command: release
    tag: ${{ steps.plan.outputs.tag }}
    notes-file: ${{ steps.plan.outputs.notes_path }}
    asset: dist/*.tgz
```

The reusable workflow in
[`continuous-release/.github`](https://github.com/continuous-release/.github)
wraps this contract for the org's testbeds.

## Development

```sh
pnpm install
pnpm test
pnpm typecheck
pnpm build
```

`pnpm build` writes the self-contained CLI bundle to
`packages/cli/dist/cli.js`. That file is committed so the reusable workflow and
the action can run from a fresh checkout without installing dependencies.
