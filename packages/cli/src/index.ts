import { appendFileSync, existsSync, globSync, readFileSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { parseArgs } from 'node:util';
import { getAdapter } from '@continuous-release/adapters';
import { renderNotes } from '@continuous-release/notes';
import { ReleaseClient } from '@continuous-release/release';
import { computeNextVersion, readCommits } from '@continuous-release/version';

const USAGE = `continuous-release toolkit

Usage:
  cr plan [--cwd <dir>] [--tag-prefix v] [--release-as <version>] [--notes-out <file>] [--json]
  cr notes [--cwd <dir>] [--tag-prefix v] [--out <file>]
  cr version [--cwd <dir>] --adapter <npm|cargo|text> [--file <path>] [--version <version>]
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

async function release() {
  const client = new ReleaseClient({
    token: requireToken(),
    repository: requireRepository(),
    apiUrl: values['api-url'] ?? process.env.GITHUB_API_URL,
  });
  const next = values.tag
    ? null
    : computeNextVersion({ cwd, prefix, releaseAs: values['release-as'] });
  const tag = values.tag ?? next!.tag;
  const body = values['notes-file'] ? readFileSync(resolveInput(values['notes-file']), 'utf8') : '';
  const prerelease = values.prerelease || tag.includes('-');
  const existing = await client.getReleaseByTag(tag);
  const current = existing
    ? await client.updateRelease(existing.id, { name: tag, body, prerelease })
    : await client.createRelease({
        tag,
        target: values.target ?? process.env.GITHUB_SHA,
        name: tag,
        body,
        prerelease,
      });
  console.log(`${existing ? 'updated' : 'created'} release ${tag}`);

  for (const asset of expandAssets(values.asset ?? [])) {
    if (!existsSync(asset)) throw new Error(`Asset not found: ${asset}`);
    await client.uploadAsset(current, asset);
    console.log(`uploaded ${asset}`);
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
