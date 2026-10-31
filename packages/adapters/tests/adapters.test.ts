import { mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import { getAdapter } from '../src/index.js';

function dir() {
  return mkdtempSync(join(tmpdir(), 'cr-adapters-'));
}

describe('adapters', () => {
  it('reads and writes an npm package version', () => {
    const cwd = dir();
    writeFileSync(join(cwd, 'package.json'), JSON.stringify({ name: 'x', version: '1.2.3' }, null, 2));

    const adapter = getAdapter('npm');
    expect(adapter.read(cwd)).toBe('1.2.3');
    adapter.write(cwd, '2.0.0');

    const pkg = JSON.parse(readFileSync(join(cwd, 'package.json'), 'utf8'));
    expect(pkg.version).toBe('2.0.0');
    expect(pkg.name).toBe('x');
  });

  it('reads and writes a Cargo package version', () => {
    const cwd = dir();
    writeFileSync(
      join(cwd, 'Cargo.toml'),
      '[package]\nname = "demo"\nversion = "1.2.3"\nedition = "2021"\n',
    );

    const adapter = getAdapter('cargo');
    expect(adapter.read(cwd)).toBe('1.2.3');
    adapter.write(cwd, '2.0.0');

    const text = readFileSync(join(cwd, 'Cargo.toml'), 'utf8');
    expect(text).toContain('version = "2.0.0"');
    expect(text).toContain('name = "demo"');
  });

  it('reads and writes a text version file', () => {
    const cwd = dir();
    writeFileSync(join(cwd, 'version.txt'), '1.2.3\n');

    const adapter = getAdapter('text');
    expect(adapter.read(cwd)).toBe('1.2.3');
    adapter.write(cwd, '2.0.0');
    expect(readFileSync(join(cwd, 'version.txt'), 'utf8')).toBe('2.0.0\n');
  });

  it('supports a custom file', () => {
    const cwd = dir();
    writeFileSync(join(cwd, 'VERSION'), '0.1.0\n');

    const adapter = getAdapter('text');
    adapter.write(cwd, '0.2.0', 'VERSION');
    expect(adapter.read(cwd, 'VERSION')).toBe('0.2.0');
  });

  it('rejects unknown adapters', () => {
    expect(() => getAdapter('maven')).toThrow('Unknown adapter');
  });
});
