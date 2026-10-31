import { describe, expect, it } from 'vitest';
import { parseCommit } from '../../version/src/index.js';
import { renderNotes } from '../src/index.js';

function commit(hash: string, subject: string, body = '') {
  return parseCommit(hash, subject, body);
}

describe('renderNotes', () => {
  it('groups commits into sections and links the compare range', () => {
    const notes = renderNotes({
      commits: [
        commit('aaaaaaaa1', 'feat(api): add endpoint'),
        commit('bbbbbbbb2', 'fix: correct timeout'),
        commit('cccccccc3', 'docs: explain usage'),
      ],
      previousTag: 'v1.2.3',
      version: '1.3.0',
      repository: 'continuous-release/nx',
    });

    expect(notes).toContain('### Features');
    expect(notes).toContain('### Bug Fixes');
    expect(notes).toContain('### Documentation');
    expect(notes).toContain('feat(api): add endpoint (aaaaaaa)');
    expect(notes).toContain(
      '**Full Changelog**: https://github.com/continuous-release/nx/compare/v1.2.3...v1.3.0',
    );
  });

  it('adds a breaking changes section', () => {
    const notes = renderNotes({
      commits: [
        commit('aaaaaaaa1', 'feat!: drop v1'),
        commit('bbbbbbbb2', 'fix: adjust', 'BREAKING CHANGE: removed flag'),
      ],
      previousTag: 'v1.0.0',
      version: '2.0.0',
      repository: 'continuous-release/bazel',
    });

    expect(notes).toContain('### ⚠ BREAKING CHANGES');
    expect(notes).toContain('feat!: drop v1');
    expect(notes).toContain('fix: adjust');
  });

  it('uses a commits link for the first release', () => {
    const notes = renderNotes({
      commits: [commit('aaaaaaaa1', 'feat: first')],
      previousTag: null,
      version: '0.1.0',
      repository: 'continuous-release/nix',
    });

    expect(notes).toContain(
      '**Full Changelog**: https://github.com/continuous-release/nix/commits/v0.1.0',
    );
  });

  it('skips merge and release commits', () => {
    const notes = renderNotes({
      commits: [
        commit('aaaaaaaa1', 'Merge branch main'),
        commit('bbbbbbbb2', 'chore(release): v1.0.0'),
        commit('cccccccc3', 'feat: real change'),
      ],
      previousTag: 'v1.0.0',
      version: '1.1.0',
    });

    expect(notes).not.toContain('Merge branch main');
    expect(notes).not.toContain('chore(release)');
    expect(notes).toContain('feat: real change');
  });

  it('omits the changelog link without a repository', () => {
    const notes = renderNotes({
      commits: [commit('aaaaaaaa1', 'feat: first')],
      previousTag: null,
      version: '0.1.0',
    });

    expect(notes).not.toContain('Full Changelog');
  });
});
