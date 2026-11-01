// packages/notes/src/index.ts
const SECTIONS = [
	{
		type: "feat",
		title: "Features"
	},
	{
		type: "fix",
		title: "Bug Fixes"
	},
	{
		type: "perf",
		title: "Performance Improvements"
	},
	{
		type: "refactor",
		title: "Code Refactoring"
	},
	{
		type: "docs",
		title: "Documentation"
	},
	{
		type: "build",
		title: "Build System"
	},
	{
		type: "ci",
		title: "Continuous Integration"
	},
	{
		type: "test",
		title: "Tests"
	},
	{
		type: "chore",
		title: "Chores"
	},
	{
		type: "revert",
		title: "Reverts"
	}
];
const HIDDEN_TYPES = /* @__PURE__ */ new Set(["release"]);
function shortHash(hash) {
	return hash.slice(0, 7);
}
function isNoise(commit) {
	if (commit.subject.startsWith("Merge ")) return true;
	if (commit.type === "chore" && commit.scope === "release") return true;
	return false;
}
function renderNotes(options) {
	const prefix = options.tagPrefix ?? "v";
	const commits = options.commits.filter((commit) => !isNoise(commit));
	const lines = ["## What's Changed", ""];
	const breaking = commits.filter((commit) => commit.breaking);
	if (breaking.length > 0) {
		lines.push("### ⚠ BREAKING CHANGES", "");
		for (const commit of breaking) lines.push(`- ${commit.subject} (${shortHash(commit.hash)})`);
		lines.push("");
	}
	for (const section of SECTIONS) {
		const matches = commits.filter((commit) => commit.type === section.type);
		if (matches.length === 0) continue;
		lines.push(`### ${section.title}`, "");
		for (const commit of matches) lines.push(`- ${commit.subject} (${shortHash(commit.hash)})`);
		lines.push("");
	}
	const known = new Set(SECTIONS.map((section) => section.type));
	const other = commits.filter((commit) => !commit.type || !known.has(commit.type) && !HIDDEN_TYPES.has(commit.type));
	if (other.length > 0) {
		lines.push("### Other Changes", "");
		for (const commit of other) lines.push(`- ${commit.subject} (${shortHash(commit.hash)})`);
		lines.push("");
	}
	if (options.repository) {
		const tag = `${prefix}${options.version}`;
		const link = options.previousTag ? `https://github.com/${options.repository}/compare/${options.previousTag}...${tag}` : `https://github.com/${options.repository}/commits/${tag}`;
		lines.push(`**Full Changelog**: ${link}`);
	}
	return `${lines.join("\n").trim()}\n`;
}
export { renderNotes };

//# sourceMappingURL=index.js.map