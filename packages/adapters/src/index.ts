import { readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

export interface VersionAdapter {
  name: string;
  defaultFile: string;
  read(cwd: string, file?: string): string;
  write(cwd: string, version: string, file?: string): void;
}

const PACKAGE_VERSION_RE = /^(\[package\][\s\S]*?^version\s*=\s*")[^"]+(")/m;
const PACKAGE_VERSION_READ_RE = /^\[package\][\s\S]*?^version\s*=\s*"([^"]+)"/m;

const npm: VersionAdapter = {
  name: 'npm',
  defaultFile: 'package.json',
  read(cwd, file = 'package.json') {
    const pkg = JSON.parse(readFileSync(join(cwd, file), 'utf8')) as { version?: string };
    if (!pkg.version) throw new Error(`${file} has no version`);
    return pkg.version;
  },
  write(cwd, version, file = 'package.json') {
    const path = join(cwd, file);
    const pkg = JSON.parse(readFileSync(path, 'utf8')) as Record<string, unknown>;
    pkg.version = version;
    writeFileSync(path, `${JSON.stringify(pkg, null, 2)}\n`);
  },
};

const cargo: VersionAdapter = {
  name: 'cargo',
  defaultFile: 'Cargo.toml',
  read(cwd, file = 'Cargo.toml') {
    const text = readFileSync(join(cwd, file), 'utf8');
    const match = text.match(PACKAGE_VERSION_READ_RE);
    if (!match) throw new Error(`${file} has no [package] version`);
    return match[1];
  },
  write(cwd, version, file = 'Cargo.toml') {
    const path = join(cwd, file);
    const text = readFileSync(path, 'utf8');
    if (!PACKAGE_VERSION_RE.test(text)) throw new Error(`${file} has no [package] version`);
    writeFileSync(path, text.replace(PACKAGE_VERSION_RE, `$1${version}$2`));
  },
};

const text: VersionAdapter = {
  name: 'text',
  defaultFile: 'version.txt',
  read(cwd, file = 'version.txt') {
    return readFileSync(join(cwd, file), 'utf8').trim();
  },
  write(cwd, version, file = 'version.txt') {
    writeFileSync(join(cwd, file), `${version}\n`);
  },
};

export const adapters: Record<string, VersionAdapter> = { npm, cargo, text };

export function getAdapter(name: string): VersionAdapter {
  const adapter = adapters[name];
  if (!adapter) {
    throw new Error(`Unknown adapter ${name}; expected one of ${Object.keys(adapters).join(', ')}`);
  }
  return adapter;
}
