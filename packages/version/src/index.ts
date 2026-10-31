import { execFileSync } from 'node:child_process';
import semver from 'semver';

export interface ConventionalCommit {
  hash: string;
  subject: string;
  body: string;
  type: string | null;
  scope: string | null;
  breaking: boolean;
  description: string;
}

export type Bump = 'major' | 'minor' | 'patch' | 'none';

export interface NextVersion {
  version: string;
  tag: string;
  previousTag: string | null;
  bump: Bump;
  existing: boolean;
  releaseAs: string | null;
}

const TYPE_RE = /^([a-zA-Z]+)(?:\(([^)]*)\))?(!)?:\s*(.+)$/;
const BREAKING_RE = /^BREAKING[ -]CHANGE:\s*(.+)$/m;
const RELEASE_AS_RE = /^Release-As:\s*(\S+)\s*$/im;

export function parseCommit(hash: string, subject: string, body: string): ConventionalCommit {
  const match = subject.match(TYPE_RE);
  return {
    hash,
    subject,
    body,
    type: match ? match[1].toLowerCase() : null,
    scope: match?.[2] ?? null,
    breaking: Boolean(match?.[3]) || BREAKING_RE.test(body),
    description: match ? match[4] : subject,
  };
}

export function readCommits(range: string, cwd = process.cwd()): ConventionalCommit[] {
  const raw = execFileSync('git', ['log', range, '--format=%H%x1f%s%x1f%b%x1e'], {
    cwd,
    encoding: 'utf8',
    maxBuffer: 64 * 1024 * 1024,
  });

  return raw
    .split('\x1e')
    .map((record) => record.trim())
    .filter(Boolean)
    .map((record) => {
      const [hash = '', subject = '', body = ''] = record.split('\x1f');
      return parseCommit(hash, subject, body);
    });
}

export function lastTag(cwd = process.cwd(), prefix = 'v'): string | null {
  try {
    return (
      execFileSync('git', ['describe', '--tags', '--abbrev=0', '--match', `${prefix}*`], {
        cwd,
        encoding: 'utf8',
        stdio: ['ignore', 'pipe', 'ignore'],
      }).trim() || null
    );
  } catch {
    return null;
  }
}

export function lastTagBefore(ref: string, cwd = process.cwd(), prefix = 'v'): string | null {
  try {
    return (
      execFileSync(
        'git',
        ['describe', '--tags', '--abbrev=0', '--match', `${prefix}*`, `${ref}^`],
        { cwd, encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'] },
      ).trim() || null
    );
  } catch {
    return null;
  }
}

export function listVersionTags(cwd = process.cwd(), prefix = 'v'): string[] {
  try {
    return execFileSync('git', ['tag', '--list', `${prefix}*`, '--sort=-v:refname'], {
      cwd,
      encoding: 'utf8',
      stdio: ['ignore', 'pipe', 'ignore'],
    })
      .split('\n')
      .map((tag) => tag.trim())
      .filter(Boolean);
  } catch {
    return [];
  }
}

export function tagsAtHead(cwd = process.cwd(), prefix = 'v'): string[] {
  try {
    return execFileSync('git', ['tag', '--points-at', 'HEAD', '--list', `${prefix}*`], {
      cwd,
      encoding: 'utf8',
      stdio: ['ignore', 'pipe', 'ignore'],
    })
      .split('\n')
      .map((tag) => tag.trim())
      .filter(Boolean);
  } catch {
    return [];
  }
}

export interface ComputeOptions {
  cwd?: string;
  releaseAs?: string;
  prefix?: string;
}

export function computeNextVersion(options: ComputeOptions = {}): NextVersion {
  const cwd = options.cwd ?? process.cwd();
  const prefix = options.prefix ?? 'v';
  const headTags = tagsAtHead(cwd, prefix);
  const previousTag = lastTag(cwd, prefix);

  if (headTags.length > 0) {
    const version = headTags[0].slice(prefix.length);
    return {
      version,
      tag: headTags[0],
      previousTag,
      bump: 'none',
      existing: true,
      releaseAs: null,
    };
  }

  const range = previousTag ? `${previousTag}..HEAD` : 'HEAD';
  const commits = readCommits(range, cwd);

  const releaseAs =
    options.releaseAs ??
    commits.map((commit) => commit.body.match(RELEASE_AS_RE)?.[1]).find(Boolean) ??
    null;

  if (releaseAs) {
    const version = semver.valid(releaseAs.replace(/^v/, ''));
    if (!version) throw new Error(`Release-As ${releaseAs} is not valid semver`);
    return {
      version,
      tag: `${prefix}${version}`,
      previousTag,
      bump: 'none',
      existing: false,
      releaseAs,
    };
  }

  if (!previousTag) {
    return {
      version: '0.1.0',
      tag: `${prefix}0.1.0`,
      previousTag: null,
      bump: 'minor',
      existing: false,
      releaseAs: null,
    };
  }

  const current = semver.valid(previousTag.slice(prefix.length));
  if (!current) throw new Error(`Last tag ${previousTag} is not valid semver`);

  const breaking = commits.some((commit) => commit.breaking);
  const feature = commits.some((commit) => commit.type === 'feat');
  const bump: Bump = breaking ? 'major' : feature ? 'minor' : 'patch';
  const version = semver.inc(current, bump);

  if (!version) throw new Error(`Could not bump ${current} by ${bump}`);

  return {
    version,
    tag: `${prefix}${version}`,
    previousTag,
    bump,
    existing: false,
    releaseAs: null,
  };
}
