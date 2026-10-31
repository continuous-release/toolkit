import { build } from 'rolldown';
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';

const order = ['version', 'notes', 'adapters', 'release', 'cli'];

for (const name of order) {
  const dir = `packages/${name}`;
  const outfile = name === 'cli' ? `${dir}/dist/cli.js` : `${dir}/dist/index.js`;
  mkdirSync(`${dir}/dist`, { recursive: true });

  await build({
    input: `${dir}/src/index.ts`,
    platform: 'node',
    output: {
      file: outfile,
      format: 'esm',
      sourcemap: true,
    },
  });

  if (name === 'cli') {
    const code = readFileSync(outfile, 'utf8');
    if (!code.startsWith('#!')) {
      writeFileSync(outfile, `#!/usr/bin/env node\n${code}`);
    }
  }

  console.log(`built ${outfile}`);
}
