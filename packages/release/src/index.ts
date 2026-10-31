import { readFile } from 'node:fs/promises';
import { basename } from 'node:path';

export interface ReleaseClientOptions {
  token: string;
  repository: string;
  apiUrl?: string;
}

export interface ReleaseAsset {
  id: number;
  name: string;
  size: number;
  content_type: string;
  browser_download_url: string;
}

export interface Release {
  id: number;
  tag_name: string;
  name: string | null;
  body: string | null;
  draft: boolean;
  prerelease: boolean;
  html_url: string;
  upload_url: string;
  assets: ReleaseAsset[];
}

export interface CreateReleaseInput {
  tag: string;
  target?: string;
  name?: string;
  body: string;
  prerelease?: boolean;
  draft?: boolean;
}

export interface UpdateReleaseInput {
  name?: string;
  body?: string;
  prerelease?: boolean;
  draft?: boolean;
}

export interface GitRef {
  ref: string;
  object: { sha: string; type: string };
}

export class TagConflictError extends Error {
  constructor(
    readonly tag: string,
    readonly expected: string,
    readonly actual: string,
  ) {
    super(`tag ${tag} already points at ${actual}, not ${expected}`);
    this.name = 'TagConflictError';
  }
}

export class ReleaseClient {
  private readonly apiUrl: string;

  constructor(private readonly options: ReleaseClientOptions) {
    this.apiUrl = (options.apiUrl ?? 'https://api.github.com').replace(/\/$/, '');
  }

  private headers(extra: Record<string, string> = {}): Record<string, string> {
    return {
      Accept: 'application/vnd.github+json',
      Authorization: `Bearer ${this.options.token}`,
      'X-GitHub-Api-Version': '2022-11-28',
      'User-Agent': 'continuous-release-toolkit',
      ...extra,
    };
  }

  private async request<T>(path: string, init: RequestInit = {}): Promise<T> {
    const response = await fetch(`${this.apiUrl}${path}`, {
      ...init,
      headers: this.headers(init.headers as Record<string, string> | undefined),
    });

    if (!response.ok) {
      const text = await response.text();
      const error = new Error(
        `GitHub API ${init.method ?? 'GET'} ${path} failed: ${response.status} ${text}`,
      ) as Error & { status?: number };
      error.status = response.status;
      throw error;
    }

    if (response.status === 204) return undefined as T;
    return (await response.json()) as T;
  }

  async getTagRef(tag: string): Promise<GitRef | null> {
    try {
      return await this.request<GitRef>(
        `/repos/${this.options.repository}/git/ref/tags/${encodeURIComponent(tag)}`,
      );
    } catch (error) {
      if ((error as { status?: number }).status === 404) return null;
      throw error;
    }
  }

  async resolveTagCommit(tag: string): Promise<string | null> {
    const ref = await this.getTagRef(tag);
    if (!ref) return null;
    if (ref.object.type === 'commit') return ref.object.sha;
    const annotated = await this.request<{ object: { sha: string } }>(
      `/repos/${this.options.repository}/git/tags/${ref.object.sha}`,
    );
    return annotated.object.sha;
  }

  async createTagRef(tag: string, sha: string): Promise<GitRef> {
    return this.request<GitRef>(`/repos/${this.options.repository}/git/refs`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ ref: `refs/tags/${tag}`, sha }),
    });
  }

  /**
   * Reserve a version by creating its tag. Tag creation is atomic on the
   * server, so the first run to reach it wins and every later run sees either
   * the same commit (a re-run) or a different one (a lost race).
   */
  async reserveTag(tag: string, sha: string): Promise<{ created: boolean }> {
    try {
      await this.createTagRef(tag, sha);
      return { created: true };
    } catch (error) {
      if ((error as { status?: number }).status !== 422) throw error;
    }

    const current = await this.resolveTagCommit(tag);
    if (!current) throw new Error(`tag ${tag} exists but could not be read`);
    if (current !== sha) throw new TagConflictError(tag, sha, current);
    return { created: false };
  }

  async getReleaseByTag(tag: string): Promise<Release | null> {
    try {
      return await this.request<Release>(
        `/repos/${this.options.repository}/releases/tags/${encodeURIComponent(tag)}`,
      );
    } catch (error) {
      if ((error as { status?: number }).status === 404) return null;
      throw error;
    }
  }

  async listReleases(): Promise<Release[]> {
    return this.request<Release[]>(`/repos/${this.options.repository}/releases?per_page=100`);
  }

  async createRelease(input: CreateReleaseInput): Promise<Release> {
    return this.request<Release>(`/repos/${this.options.repository}/releases`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        tag_name: input.tag,
        target_commitish: input.target,
        name: input.name ?? input.tag,
        body: input.body,
        draft: input.draft ?? false,
        prerelease: input.prerelease ?? false,
      }),
    });
  }

  async updateRelease(id: number, input: UpdateReleaseInput): Promise<Release> {
    return this.request<Release>(`/repos/${this.options.repository}/releases/${id}`, {
      method: 'PATCH',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(input),
    });
  }

  async listAssets(releaseId: number): Promise<ReleaseAsset[]> {
    return this.request<ReleaseAsset[]>(
      `/repos/${this.options.repository}/releases/${releaseId}/assets?per_page=100`,
    );
  }

  async deleteAsset(assetId: number): Promise<void> {
    await this.request<void>(`/repos/${this.options.repository}/releases/assets/${assetId}`, {
      method: 'DELETE',
    });
  }

  async uploadAsset(release: Release, filePath: string, name = basename(filePath)): Promise<ReleaseAsset> {
    const existing = await this.listAssets(release.id);
    for (const asset of existing.filter((candidate) => candidate.name === name)) {
      await this.deleteAsset(asset.id);
    }

    const url = new URL(release.upload_url.replace(/\{.*\}$/, ''));
    url.searchParams.set('name', name);
    const body = await readFile(filePath);
    const response = await fetch(url, {
      method: 'POST',
      headers: this.headers({
        'Content-Type': 'application/octet-stream',
        'Content-Length': String(body.length),
      }),
      body: body as unknown as BodyInit,
    });

    if (!response.ok) {
      const text = await response.text();
      throw new Error(`Upload ${name} failed: ${response.status} ${text}`);
    }

    return (await response.json()) as ReleaseAsset;
  }

  async deleteRelease(id: number): Promise<void> {
    await this.request<void>(`/repos/${this.options.repository}/releases/${id}`, {
      method: 'DELETE',
    });
  }

  async deleteTag(tag: string): Promise<void> {
    await this.request<void>(
      `/repos/${this.options.repository}/git/refs/tags/${encodeURIComponent(tag)}`,
      { method: 'DELETE' },
    );
  }
}
