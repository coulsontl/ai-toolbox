import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { runBrowserFixture } from './lib/browser-fixture.mjs';
import { verifyCodexFetchedModelPicker } from '../web/test/features/coding/codex/components/codexFetchedModelPickerBrowserChecks.mjs';

const fixtureDirectory = fileURLToPath(new URL('../web/test/features/coding/codex/components/fixtures', import.meta.url));
await runBrowserFixture({
  fixtureDirectory,
  fixtureFilename: 'CodexFetchedModelPickerFixture.jsx',
  artifactPrefix: 'codex-fetched-model-picker-',
  verify: verifyCodexFetchedModelPicker,
  fixtureAliases: [
    { find: '@/components/common/TomlEditor', replacement: path.join(fixtureDirectory, 'TomlEditorStub.jsx') },
  ],
});
