import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
// packages/adapters/src/index.ts
const PACKAGE_VERSION_RE = /^(\[package\][\s\S]*?^version\s*=\s*")[^"]+(")/m;
const PACKAGE_VERSION_READ_RE = /^\[package\][\s\S]*?^version\s*=\s*"([^"]+)"/m;
const adapters = {
	npm: {
		name: "npm",
		defaultFile: "package.json",
		read(cwd, file = "package.json") {
			const pkg = JSON.parse(readFileSync(join(cwd, file), "utf8"));
			if (!pkg.version) throw new Error(`${file} has no version`);
			return pkg.version;
		},
		write(cwd, version, file = "package.json") {
			const path = join(cwd, file);
			const pkg = JSON.parse(readFileSync(path, "utf8"));
			pkg.version = version;
			writeFileSync(path, `${JSON.stringify(pkg, null, 2)}\n`);
		}
	},
	cargo: {
		name: "cargo",
		defaultFile: "Cargo.toml",
		read(cwd, file = "Cargo.toml") {
			const match = readFileSync(join(cwd, file), "utf8").match(PACKAGE_VERSION_READ_RE);
			if (!match) throw new Error(`${file} has no [package] version`);
			return match[1];
		},
		write(cwd, version, file = "Cargo.toml") {
			const path = join(cwd, file);
			const text = readFileSync(path, "utf8");
			if (!PACKAGE_VERSION_RE.test(text)) throw new Error(`${file} has no [package] version`);
			writeFileSync(path, text.replace(PACKAGE_VERSION_RE, `$1${version}$2`));
		}
	},
	text: {
		name: "text",
		defaultFile: "version.txt",
		read(cwd, file = "version.txt") {
			return readFileSync(join(cwd, file), "utf8").trim();
		},
		write(cwd, version, file = "version.txt") {
			writeFileSync(join(cwd, file), `${version}\n`);
		}
	}
};
function getAdapter(name) {
	const adapter = adapters[name];
	if (!adapter) throw new Error(`Unknown adapter ${name}; expected one of ${Object.keys(adapters).join(", ")}`);
	return adapter;
}
export { adapters, getAdapter };

//# sourceMappingURL=index.js.map