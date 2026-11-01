import { readFile } from "node:fs/promises";
import { basename } from "node:path";
// packages/release/src/index.ts
var TagConflictError = class extends Error {
	tag;
	expected;
	actual;
	constructor(tag, expected, actual) {
		super(`tag ${tag} already points at ${actual}, not ${expected}`);
		this.tag = tag;
		this.expected = expected;
		this.actual = actual;
		this.name = "TagConflictError";
	}
};
var ReleaseClient = class {
	options;
	apiUrl;
	constructor(options) {
		this.options = options;
		this.apiUrl = (options.apiUrl ?? "https://api.github.com").replace(/\/$/, "");
	}
	headers(extra = {}) {
		return {
			Accept: "application/vnd.github+json",
			Authorization: `Bearer ${this.options.token}`,
			"X-GitHub-Api-Version": "2022-11-28",
			"User-Agent": "continuous-release-toolkit",
			...extra
		};
	}
	async request(path, init = {}) {
		const response = await fetch(`${this.apiUrl}${path}`, {
			...init,
			headers: this.headers(init.headers)
		});
		if (!response.ok) {
			const text = await response.text();
			const error = /* @__PURE__ */ new Error(`GitHub API ${init.method ?? "GET"} ${path} failed: ${response.status} ${text}`);
			error.status = response.status;
			throw error;
		}
		if (response.status === 204) return void 0;
		return await response.json();
	}
	async getTagRef(tag) {
		try {
			return await this.request(`/repos/${this.options.repository}/git/ref/tags/${encodeURIComponent(tag)}`);
		} catch (error) {
			if (error.status === 404) return null;
			throw error;
		}
	}
	async resolveTagCommit(tag) {
		const ref = await this.getTagRef(tag);
		if (!ref) return null;
		if (ref.object.type === "commit") return ref.object.sha;
		return (await this.request(`/repos/${this.options.repository}/git/tags/${ref.object.sha}`)).object.sha;
	}
	async createTagRef(tag, sha) {
		return this.request(`/repos/${this.options.repository}/git/refs`, {
			method: "POST",
			headers: { "Content-Type": "application/json" },
			body: JSON.stringify({
				ref: `refs/tags/${tag}`,
				sha
			})
		});
	}
	/**
	* Reserve a version by creating its tag. Tag creation is atomic on the
	* server, so the first run to reach it wins and every later run sees either
	* the same commit (a re-run) or a different one (a lost race).
	*/
	async reserveTag(tag, sha) {
		try {
			await this.createTagRef(tag, sha);
			return { created: true };
		} catch (error) {
			if (error.status !== 422) throw error;
		}
		const current = await this.resolveTagCommit(tag);
		if (!current) throw new Error(`tag ${tag} exists but could not be read`);
		if (current !== sha) throw new TagConflictError(tag, sha, current);
		return { created: false };
	}
	async getReleaseByTag(tag) {
		try {
			return await this.request(`/repos/${this.options.repository}/releases/tags/${encodeURIComponent(tag)}`);
		} catch (error) {
			if (error.status === 404) return null;
			throw error;
		}
	}
	async listReleases() {
		return this.request(`/repos/${this.options.repository}/releases?per_page=100`);
	}
	async createRelease(input) {
		return this.request(`/repos/${this.options.repository}/releases`, {
			method: "POST",
			headers: { "Content-Type": "application/json" },
			body: JSON.stringify({
				tag_name: input.tag,
				target_commitish: input.target,
				name: input.name ?? input.tag,
				body: input.body,
				draft: input.draft ?? false,
				prerelease: input.prerelease ?? false
			})
		});
	}
	async updateRelease(id, input) {
		return this.request(`/repos/${this.options.repository}/releases/${id}`, {
			method: "PATCH",
			headers: { "Content-Type": "application/json" },
			body: JSON.stringify(input)
		});
	}
	async listAssets(releaseId) {
		return this.request(`/repos/${this.options.repository}/releases/${releaseId}/assets?per_page=100`);
	}
	async deleteAsset(assetId) {
		await this.request(`/repos/${this.options.repository}/releases/assets/${assetId}`, { method: "DELETE" });
	}
	async uploadAsset(release, filePath, name = basename(filePath)) {
		const existing = await this.listAssets(release.id);
		for (const asset of existing.filter((candidate) => candidate.name === name)) await this.deleteAsset(asset.id);
		const url = new URL(release.upload_url.replace(/\{.*\}$/, ""));
		url.searchParams.set("name", name);
		const body = await readFile(filePath);
		const response = await fetch(url, {
			method: "POST",
			headers: this.headers({
				"Content-Type": "application/octet-stream",
				"Content-Length": String(body.length)
			}),
			body
		});
		if (!response.ok) {
			const text = await response.text();
			throw new Error(`Upload ${name} failed: ${response.status} ${text}`);
		}
		return await response.json();
	}
	async deleteRelease(id) {
		await this.request(`/repos/${this.options.repository}/releases/${id}`, { method: "DELETE" });
	}
	async deleteTag(tag) {
		await this.request(`/repos/${this.options.repository}/git/refs/tags/${encodeURIComponent(tag)}`, { method: "DELETE" });
	}
};
export { ReleaseClient, TagConflictError };

//# sourceMappingURL=index.js.map