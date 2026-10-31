import { execFileSync } from 'node:child_process';
import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import { computeNextVersion, parseCommit } from '../src/index.js';

function makeRepo() {
  const dir = mkdtempSync(join(tmpdir(), 'cr-version-'));
  const git = (...args: string[]) =>
    execFileSync('git', args, {
      cwd: dir,
      encoding: 'utf8',
      stdio: ['ignore', 'pipe', 'ignore'],
    });

  git('init', '-q', '-b', 'main');
  git('config', 'user.email', 'test@example.com');
  git('config', 'user.name', 'Test');

  let count = 0;
  const commit = (message: string, body?: string) => {
    count += 1;
    writeFileSync(join(dir, 'file.txt'), `${count}\n`);
    git('add', '.');
    const args = ['commit', '-q', '-m', message];
    if (body) args.push('-m', body);
    git(...args);
  };

  return { dir, git, commit };
}

describe('parseCommit', () => {
  it('parses type, scope, and breaking bang', () => {
    const commit = parseCommit('abc', 'feat(api)!: drop v1', '');
    expect(commit.type).toBe('feat');
    expect(commit.scope).toBe('api');
    expect(commit.breaking).toBe(true);
    expect(commit.description).toBe('drop v1');
  });

  it('parses a breaking footer', () => {
    const commit = parseCommit('abc', 'fix: adjust', 'BREAKING CHANGE: removed flag');
    expect(commit.type).toBe('fix');
    expect(commit.breaking).toBe(true);
  });

  it('keeps non-conventional subjects', () => {
    const commit = parseCommit('abc', 'Update docs', '');
    expect(commit.type).toBeNull();
    expect(commit.breaking).toBe(false);
  });
});

describe('computeNextVersion', () => {
  it('starts at 0.1.0 without a tag', () => {
    const { dir, commit } = makeRepo();
    commit('feat: first');
    expect(computeNextVersion({ cwd: dir })).toMatchObject({
      version: '0.1.0',
      tag: 'v0.1.0',
      previousTag: null,
      bump: 'minor',
      existing: false,
    });
  });

  it('bumps patch for fixes', () => {
    const { dir, git, commit } = makeRepo();
    commit('feat: first');
    git('tag', 'v1.2.3');
    commit('fix: a bug');
    expect(computeNextVersion({ cwd: dir })).toMatchObject({
      version: '1.2.4',
      bump: 'patch',
      previousTag: 'v1.2.3',
    });
  });

  it('bumps minor for features', () => {
    const { dir, git, commit } = makeRepo();
    commit('feat: first');
    git('tag', 'v1.2.3');
    commit('feat(core): add thing');
    expect(computeNextVersion({ cwd: dir })).toMatchObject({ version: '1.3.0', bump: 'minor' });
  });

  it('bumps major for a bang', () => {
    const { dir, git, commit } = makeRepo();
    commit('feat: first');
    git('tag', 'v1.2.3');
    commit('refactor!: drop old API');
    expect(computeNextVersion({ cwd: dir })).toMatchObject({ version: '2.0.0', bump: 'major' });
  });

  it('bumps major for a breaking footer', () => {
    const { dir, git, commit } = makeRepo();
    commit('feat: first');
    git('tag', 'v1.2.3');
    commit('fix: adjust', 'BREAKING CHANGE: removed flag');
    expect(computeNextVersion({ cwd: dir })).toMatchObject({ version: '2.0.0', bump: 'major' });
  });

  it('releases a patch for chore-only commits', () => {
    const { dir, git, commit } = makeRepo();
    commit('feat: first');
    git('tag', 'v1.2.3');
    commit('chore: tidy');
    expect(computeNextVersion({ cwd: dir })).toMatchObject({ version: '1.2.4', bump: 'patch' });
  });

  it('honors Release-As', () => {
    const { dir, git, commit } = makeRepo();
    commit('feat: first');
    git('tag', 'v1.2.3');
    commit('chore: force', 'Release-As: 3.4.5');
    expect(computeNextVersion({ cwd: dir })).toMatchObject({
      version: '3.4.5',
      tag: 'v3.4.5',
      releaseAs: '3.4.5',
    });
  });

  it('reuses a tag already on HEAD', () => {
    const { dir, git, commit } = makeRepo();
    commit('feat: first');
    git('tag', 'v1.2.3');
    expect(computeNextVersion({ cwd: dir })).toMatchObject({
      version: '1.2.3',
      tag: 'v1.2.3',
      existing: true,
      bump: 'none',
    });
  });

  it('supports a custom tag prefix', () => {
    const { dir, git, commit } = makeRepo();
    commit('feat: first');
    git('tag', 'release-1.0.0');
    commit('fix: bug');
    expect(computeNextVersion({ cwd: dir, prefix: 'release-' })).toMatchObject({
      version: '1.0.1',
      tag: 'release-1.0.1',
      previousTag: 'release-1.0.0',
    });
  });
});
