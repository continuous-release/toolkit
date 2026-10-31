import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ReleaseClient, type Release } from '../src/index.js';

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

const baseRelease: Release = {
  id: 7,
  tag_name: 'v1.2.3',
  name: 'v1.2.3',
  body: '',
  draft: false,
  prerelease: false,
  html_url: 'https://github.com/o/r/releases/tag/v1.2.3',
  upload_url: 'https://uploads.test/repos/o/r/releases/7/assets{?name,label}',
  assets: [],
};

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('ReleaseClient', () => {
  it('returns null when the release is missing', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => new Response('not found', { status: 404 })));
    const client = new ReleaseClient({ token: 't', repository: 'o/r', apiUrl: 'https://api.test' });
    await expect(client.getReleaseByTag('v1.2.3')).resolves.toBeNull();
  });

  it('creates a release with the expected payload', async () => {
    const fetchMock = vi.fn(async (_input: string | URL | Request, init?: RequestInit) => {
      expect(init?.method).toBe('POST');
      expect(JSON.parse(String(init?.body))).toMatchObject({
        tag_name: 'v1.2.3',
        target_commitish: 'abc',
        prerelease: false,
      });
      return jsonResponse(baseRelease);
    });
    vi.stubGlobal('fetch', fetchMock);

    const client = new ReleaseClient({ token: 't', repository: 'o/r', apiUrl: 'https://api.test' });
    const release = await client.createRelease({ tag: 'v1.2.3', target: 'abc', body: 'notes' });

    expect(release.id).toBe(7);
    expect(String(fetchMock.mock.calls[0][0])).toBe('https://api.test/repos/o/r/releases');
  });

  it('updates an existing release', async () => {
    const fetchMock = vi.fn(async (_input: string | URL | Request, init?: RequestInit) => {
      expect(init?.method).toBe('PATCH');
      expect(JSON.parse(String(init?.body))).toMatchObject({ body: 'new notes', prerelease: true });
      return jsonResponse({ ...baseRelease, body: 'new notes', prerelease: true });
    });
    vi.stubGlobal('fetch', fetchMock);

    const client = new ReleaseClient({ token: 't', repository: 'o/r', apiUrl: 'https://api.test' });
    const release = await client.updateRelease(7, { body: 'new notes', prerelease: true });

    expect(release.prerelease).toBe(true);
    expect(String(fetchMock.mock.calls[0][0])).toBe('https://api.test/repos/o/r/releases/7');
  });

  it('replaces a same-named asset when uploading', async () => {
    const cwd = mkdtempSync(join(tmpdir(), 'cr-release-'));
    const assetPath = join(cwd, 'app.tgz');
    writeFileSync(assetPath, 'tarball');

    const fetchMock = vi.fn(async (input: string | URL | Request, init?: RequestInit) => {
      const url = String(input);
      if (url.startsWith('https://uploads.test/')) {
        expect(url).toContain('name=app.tgz');
        expect(init?.method).toBe('POST');
        return jsonResponse({ id: 10, name: 'app.tgz', size: 7, content_type: 'application/gzip', browser_download_url: '' });
      }
      if (url.includes('/releases/7/assets?per_page=100')) {
        return jsonResponse([{ id: 9, name: 'app.tgz', size: 7, content_type: 'application/gzip', browser_download_url: '' }]);
      }
      if (url.endsWith('/releases/assets/9') && init?.method === 'DELETE') {
        return new Response(null, { status: 204 });
      }
      throw new Error(`unexpected fetch ${url}`);
    });
    vi.stubGlobal('fetch', fetchMock);

    const client = new ReleaseClient({ token: 't', repository: 'o/r', apiUrl: 'https://api.test' });
    const asset = await client.uploadAsset(baseRelease, assetPath);

    expect(asset.id).toBe(10);
    expect(fetchMock).toHaveBeenCalledTimes(3);
  });

  it('encodes tag names when deleting a tag', async () => {
    const fetchMock = vi.fn(
      async (_input: string | URL | Request, _init?: RequestInit) =>
        new Response(null, { status: 204 }),
    );
    vi.stubGlobal('fetch', fetchMock);

    const client = new ReleaseClient({ token: 't', repository: 'o/r', apiUrl: 'https://api.test' });
    await client.deleteTag('release/1.0.0');

    expect(String(fetchMock.mock.calls[0][0])).toBe(
      'https://api.test/repos/o/r/git/refs/tags/release%2F1.0.0',
    );
  });
});
