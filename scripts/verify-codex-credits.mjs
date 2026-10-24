import { fileURLToPath } from 'node:url';
import { runBrowserFixture } from './lib/browser-fixture.mjs';
import { verifyCodexCredits } from '../web/test/features/coding/codex/codexCreditsBrowserChecks.mjs';

await runBrowserFixture({
  fixtureDirectory: fileURLToPath(new URL('../web/test/features/coding/codex/fixtures', import.meta.url)),
  fixtureFilename: 'CodexCardViewFixture.jsx',
  artifactPrefix: 'codex-credits-',
  verify: verifyCodexCredits,
});
