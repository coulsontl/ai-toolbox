import assert from 'node:assert/strict';
import { execFile as execFileCallback } from 'node:child_process';
import { copyFile, mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { promisify } from 'node:util';

const execFile = promisify(execFileCallback);
const scriptsDirectory = fileURLToPath(new URL('../../../scripts/', import.meta.url));

for (const shouldFail of [false, true]) {
  test(`web test runner exits ${shouldFail ? 'nonzero for a later failing file' : 'zero when all files pass'}`, async (testContext) => {
    const fixtureRoot = await mkdtemp(path.join(tmpdir(), 'ai-toolbox-web-runner-'));
    testContext.after(() => rm(fixtureRoot, { recursive: true, force: true }));
    const fixtureScripts = path.join(fixtureRoot, 'scripts');
    const fixtureTests = path.join(fixtureRoot, 'web', 'test');
    await mkdir(fixtureScripts, { recursive: true });
    await mkdir(fixtureTests, { recursive: true });
    for (const name of ['run-web-tests.mjs', 'register-node-ts-extension-loader.mjs', 'node-ts-extension-loader.mjs']) {
      await copyFile(path.join(scriptsDirectory, name), path.join(fixtureScripts, name));
    }
    // The fixture sits in the OS temp directory, outside this repository, so no
    // ancestor package.json marks it as ESM. Node then loads `.test.ts` as
    // CommonJS and every fixture file dies on its `import` statement. CI passes
    // only because Linux resolves it differently; declare the module type so the
    // check exercises the runner instead of the ambient directory layout.
    await writeFile(path.join(fixtureRoot, 'package.json'), '{"type":"module"}');

    await writeFile(path.join(fixtureTests, 'first.test.ts'), `
import test from 'node:test';
test('first file passes', () => {});
`);
    await writeFile(path.join(fixtureTests, 'second.test.ts'), `
import test from 'node:test';
import { setTimeout } from 'node:timers/promises';
test('second file finishes later', async () => {
  await setTimeout(100);
  ${shouldFail ? "throw new Error('Expected fixture failure');" : ''}
});
`);
    // Exercise the CLI as an independent process, outside the parent runner.
    const environment = { ...process.env };
    delete environment.NODE_TEST_CONTEXT;
    const runFixture = () => execFile(process.execPath, [path.join(fixtureScripts, 'run-web-tests.mjs')], {
      cwd: fixtureRoot,
      env: environment,
    });
    if (shouldFail) {
      await assert.rejects(runFixture, (error: unknown) => {
        const result = error as { code?: number; stdout?: string };
        assert.equal(result.code, 1);
        assert.match(result.stdout ?? '', /Expected fixture failure/);
        return true;
      });
    } else {
      const result = await runFixture();
      assert.match(result.stdout, /pass 2/);
      assert.match(result.stdout, /fail 0/);
    }
  });
}
