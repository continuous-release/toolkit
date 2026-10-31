import { execFileSync } from 'node:child_process';
import { appendFileSync, existsSync, globSync, readFileSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { parseArgs } from 'node:util';
import { getAdapter } from '@continuous-release/adapters';
import { renderNotes } from '@continuous-release/notes';
import { ReleaseClient, TagConflictError } from '@continuous-release/release';
import {
  computeNextVersion,
  lastTagBefore,
  listVersionTags,
  readCommits,
} from '@continuous-release/version';

const USAGE = `continuous-release toolkit

Usage:
  cr plan [--cwd <dir>] [--tag-prefix v] [--release-as <version>] [--notes-out <file>] [--json]
  cr notes [--cwd <dir>] [--tag-prefix v] [--out <file>]
  cr version [--cwd <dir>] --adapter <npm|cargo|text> [--file <path>] [--version <version>]
  cr reserve --repo <owner/repo> [--token <token>] [--target <sha>] [--notes-out <file>] [--release-as <version>] [--api-url <url>]
  cr unreserve --repo <owner/repo> --tag <tag> [--token <token>] [--target <sha>] [--api-url <url>]
  cr release --repo <owner/repo> [--token <token>] [--tag <tag>] [--target <sha>] [--notes-file <file>] [--asset <path>]... [--prerelease] [--api-url <url>]
  cr verify --repo <owner/repo> [--token <token>] [--tag <tag>] [--api-url <url>]

Environment:
  GITHUB_TOKEN, GITHUB_REPOSITORY, GITHUB_SHA, GITHUB_OUTPUT
`;

const parsed = parseArgs({
  allowPositionals: true,
  options: {
    cwd: { type: 'string' },
    'tag-prefix': { type: 'string' },
    'release-as': { type: 'string' },
    json: { type: 'boolean', default: false },
    'notes-out': { type: 'string' },
    out: { type: 'string' },
    adapter: { type: 'string' },
    file: { type: 'string' },
    version: { type: 'string' },
    tag: { type: 'string' },
    target: { type: 'string' },
    'notes-file': { type: 'string' },
    asset: { type: 'string', multiple: true },
    token: { type: 'string' },
    repo: { type: 'string' },
    'api-url': { type: 'string' },
    prerelease: { type: 'boolean', default: false },
    help: { type: 'boolean', default: false },
  },
});

const { values, positionals } = parsed;
const command = positionals[0] ?? 'help';
const cwd = values.cwd ? resolve(values.cwd) : process.cwd();
const prefix = values['tag-prefix'] ?? 'v';
const repository = values.repo ?? process.env.GITHUB_REPOSITORY ?? '';
const token = values.token ?? process.env.GITHUB_TOKEN ?? '';

function setOutput(key: string, value: string): void {
  if (!process.env.GITHUB_OUTPUT) return;
  appendFileSync(process.env.GITHUB_OUTPUT, `${key}=${value}\n`);
}

function resolveInput(path: string): string {
  return resolve(cwd, path);
}

function expandAssets(patterns: string[]): string[] {
  const files: string[] = [];
  for (const pattern of patterns) {
    const matches = globSync(pattern, { cwd });
    if (matches.length === 0) throw new Error(`Asset pattern matched nothing: ${pattern}`);
    for (const match of matches) files.push(resolve(cwd, match));
  }
  return files;
}

function requireRepository(): string {
  if (!repository) throw new Error('A repository is required (--repo or GITHUB_REPOSITORY)');
  return repository;
}

function requireToken(): string {
  if (!token) throw new Error('A GitHub token is required (--token or GITHUB_TOKEN)');
  return token;
}

function plan() {
  const next = computeNextVersion({ cwd, prefix, releaseAs: values['release-as'] });
  const range = next.previousTag ? `${next.previousTag}..HEAD` : 'HEAD';
  const commits = readCommits(range, cwd);
  const notes = renderNotes({
    commits,
    previousTag: next.previousTag,
    version: next.version,
    repository,
    tagPrefix: prefix,
  });
  const notesPath = values['notes-out'] ? resolveInput(values['notes-out']) : undefined;
  if (notesPath) writeFileSync(notesPath, notes);

  setOutput('version', next.version);
  setOutput('tag', next.tag);
  setOutput('previous_tag', next.previousTag ?? '');
  setOutput('existing', String(next.existing));
  setOutput('bump', next.bump);
  if (notesPath) setOutput('notes_path', notesPath);

  if (values.json) {
    console.log(JSON.stringify({ ...next, notes, notesPath }, null, 2));
    return;
  }

  console.log(`version: ${next.version}`);
  console.log(`tag: ${next.tag}`);
  console.log(`previous tag: ${next.previousTag ?? '(none)'}`);
  console.log(`bump: ${next.bump}`);
  console.log(`existing: ${next.existing}`);
}

function notes() {
  const next = computeNextVersion({ cwd, prefix, releaseAs: values['release-as'] });
  const range = next.previousTag ? `${next.previousTag}..HEAD` : 'HEAD';
  const commits = readCommits(range, cwd);
  const body = renderNotes({
    commits,
    previousTag: next.previousTag,
    version: next.version,
    repository,
    tagPrefix: prefix,
  });
  if (values.out) writeFileSync(resolveInput(values.out), body);
  else process.stdout.write(body);
}

function version() {
  if (!values.adapter) throw new Error('--adapter is required (npm, cargo, or text)');
  const adapter = getAdapter(values.adapter);
  const next = values.version ?? computeNextVersion({ cwd, prefix, releaseAs: values['release-as'] }).version;
  adapter.write(cwd, next, values.file);
  console.log(`set ${adapter.name} version to ${next}${values.file ? ` in ${values.file}` : ''}`);
}

function releaseClient(): ReleaseClient {
  return new ReleaseClient({
    token: requireToken(),
    repository: requireRepository(),
    apiUrl: values['api-url'] ?? process.env.GITHUB_API_URL,
  });
}

function targetCommit(): string {
  const target = values.target ?? process.env.GITHUB_SHA;
  if (!target) throw new Error('A target commit is required (--target or GITHUB_SHA)');
  return target;
}

function renderPlanNotes(previousTag: string | null, version: string): string {
  const range = previousTag ? `${previousTag}..HEAD` : 'HEAD';
  return renderNotes({
    commits: readCommits(range, cwd),
    previousTag,
    version,
    repository,
    tagPrefix: prefix,
  });
}

/**
 * Reserve a version by creating its tag before anything is built or
 * published. The tag is the lock: a re-run finds it on the same commit and
 * continues, while a concurrent run that computed the same version either
 * re-plans from the winning tag or is superseded by it.
 */
async function reserve() {
  const client = releaseClient();
  const target = targetCommit();

  for (let attempt = 0; attempt < 5; attempt += 1) {
    const next = computeNextVersion({ cwd, prefix, releaseAs: values['release-as'] });
    try {
      const reservation = await client.reserveTag(next.tag, target);
      const previousTag = next.existing
        ? lastTagBefore('HEAD', cwd, prefix)
        : next.previousTag;
      const notes = renderPlanNotes(previousTag, next.version);
      const notesPath = values['notes-out'] ? resolveInput(values['notes-out']) : undefined;
      if (notesPath) writeFileSync(notesPath, notes);

      setOutput('version', next.version);
      setOutput('tag', next.tag);
      setOutput('previous_tag', previousTag ?? '');
      setOutput('existing', String(next.existing));
      setOutput('superseded', 'false');
      if (notesPath) setOutput('notes_path', notesPath);

      console.log(
        `${reservation.created ? 'reserved' : 'kept'} ${next.tag} at ${target}`,
      );
      return;
    } catch (error) {
      if (!(error instanceof TagConflictError)) throw error;

      console.log(`${next.tag} was taken by ${error.actual}; re-planning`);
      execFileSync('git', ['fetch', '--tags', '--force', 'origin'], {
        cwd,
        stdio: 'inherit',
      });

      const replanned = computeNextVersion({ cwd, prefix, releaseAs: values['release-as'] });
      if (replanned.tag === next.tag) {
        // The winning tag is not in our history, so this commit is already
        // contained in that release. Nothing is left for this run to publish.
        setOutput('superseded', 'true');
        setOutput('tag', next.tag);
        setOutput('existing', 'false');
        console.log(`superseded by ${next.tag} at ${error.actual}`);
        return;
      }
    }
  }

  throw new Error('Could not reserve a version after 5 attempts');
}

/**
 * Drop a reservation that never reached publish. The tag is only removed when
 * no release exists, it still points at this commit, and no newer version has
 * been tagged, so a failed build cannot strand a version.
 */
async function unreserve() {
  const client = releaseClient();
  const tag = values.tag;
  if (!tag) throw new Error('--tag is required');

  const release = await client.getReleaseByTag(tag);
  if (release) {
    console.log(`keeping ${tag}: a release already exists`);
    return;
  }

  const sha = await client.resolveTagCommit(tag);
  if (!sha) {
    console.log(`nothing to unreserve for ${tag}`);
    return;
  }

  const target = values.target ?? process.env.GITHUB_SHA;
  if (target && sha !== target) {
    console.log(`keeping ${tag}: it points at ${sha}, not ${target}`);
    return;
  }

  const highest = listVersionTags(cwd, prefix)[0];
  if (highest && highest !== tag) {
    console.log(`keeping ${tag}: ${highest} is newer`);
    return;
  }

  await client.deleteTag(tag);
  console.log(`unreserved ${tag}`);
}

async function release() {
  const client = releaseClient();
  const target = values.target ?? process.env.GITHUB_SHA;
  const tag = values.tag ?? computeNextVersion({ cwd, prefix, releaseAs: values['release-as'] }).tag;
  const body = values['notes-file'] ? readFileSync(resolveInput(values['notes-file']), 'utf8') : '';
  const prerelease = values.prerelease || tag.includes('-');

  const taggedCommit = await client.resolveTagCommit(tag);
  if (!taggedCommit) {
    if (!target) throw new Error('A target commit is required to create the tag');
    await client.reserveTag(tag, target);
    console.log(`reserved ${tag} at ${target}`);
  } else if (target && taggedCommit !== target) {
    throw new Error(`tag ${tag} points at ${taggedCommit}, not ${target}`);
  }

  const existing = await client.getReleaseByTag(tag);
  let current = existing
    ? await client.updateRelease(existing.id, { name: tag, body, prerelease })
    : await client.createRelease({
        tag,
        target,
        name: tag,
        body,
        prerelease,
        draft: true,
      });
  console.log(`${existing ? 'updated' : 'created'} release ${tag}${current.draft ? ' (draft)' : ''}`);

  for (const asset of expandAssets(values.asset ?? [])) {
    if (!existsSync(asset)) throw new Error(`Asset not found: ${asset}`);
    await client.uploadAsset(current, asset);
    console.log(`uploaded ${asset}`);
  }

  if (current.draft) {
    current = await client.updateRelease(current.id, { draft: false });
    console.log(`published ${tag}`);
  }

  setOutput('release_url', current.html_url);
  setOutput('release_id', String(current.id));
  console.log(current.html_url);
}

async function verify() {
  const client = new ReleaseClient({
    token: requireToken(),
    repository: requireRepository(),
    apiUrl: values['api-url'] ?? process.env.GITHUB_API_URL,
  });
  const tag = values.tag ?? computeNextVersion({ cwd, prefix }).tag;
  const current = await client.getReleaseByTag(tag);
  setOutput('exists', String(Boolean(current)));
  console.log(JSON.stringify({ tag, exists: Boolean(current), release: current }, null, 2));
}

async function main() {
  if (values.help || command === 'help') {
    process.stdout.write(USAGE);
    return;
  }

  switch (command) {
    case 'plan':
      plan();
      return;
    case 'notes':
      notes();
      return;
    case 'version':
      version();
      return;
    case 'reserve':
      await reserve();
      return;
    case 'unreserve':
      await unreserve();
      return;
    case 'release':
      await release();
      return;
    case 'verify':
      await verify();
      return;
    default:
      throw new Error(`Unknown command ${command}\n\n${USAGE}`);
  }
}

main().catch((error: unknown) => {
  const message = error instanceof Error ? error.message : String(error);
  console.error(`error: ${message}`);
  process.exit(1);
});
