import assert from 'node:assert/strict';
import { mkdtemp, mkdir, readFile, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';
import { test } from 'node:test';

test('a failing gzip makes the demo build fail', async () => {
  const root = await mkdtemp(join(tmpdir(), 'hdfour-build-'));
  try {
    await mkdir(join(root, 'demo/pkg'), { recursive: true });
    await mkdir(join(root, 'bin'));
    await writeFile(join(root, 'demo/build.sh'), await readFile(new URL('./build.sh', import.meta.url)));
    await writeFile(join(root, 'demo/pkg/hdfour_wasm_bg.wasm'), 'module');
    for (const [name, script] of Object.entries({
      cargo: 'exit 0', gzip: 'exit 17',
      'wasm-bindgen': 'if [ "$1" = --version ]; then echo "wasm-bindgen 0.2.130"; fi',
    })) await writeFile(join(root, 'bin', name), `#!/bin/sh\n${script}\n`, { mode: 0o755 });
    const result = spawnSync('sh', ['demo/build.sh'], { cwd: root, env: { ...process.env, PATH: `${root}/bin:${process.env.PATH}` } });
    assert.equal(result.status, 17, result.stderr.toString());
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});
