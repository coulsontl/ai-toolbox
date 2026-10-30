import { fileURLToPath } from 'node:url';
import { runBrowserFixture } from './lib/browser-fixture.mjs';
import { verifyCodebuddyPage } from '../web/test/features/coding/codebuddy/pages/codebuddyPageBrowserChecks.mjs';

const fixtureDirectory = fileURLToPath(
  new URL('../web/test/features/coding/codebuddy/pages/fixtures', import.meta.url),
);
await runBrowserFixture({
  fixtureDirectory,
  fixtureFilename: 'CodebuddyPageFixture.jsx',
  artifactPrefix: 'codebuddy-page-',
  verify: verifyCodebuddyPage,
});
