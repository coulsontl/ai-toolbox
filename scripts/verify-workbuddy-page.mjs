import { fileURLToPath } from 'node:url';
import { runBrowserFixture } from './lib/browser-fixture.mjs';
import { verifyWorkbuddyPage } from '../web/test/features/coding/workbuddy/pages/workbuddyPageBrowserChecks.mjs';

const fixtureDirectory = fileURLToPath(
  new URL('../web/test/features/coding/workbuddy/pages/fixtures', import.meta.url),
);
await runBrowserFixture({
  fixtureDirectory,
  fixtureFilename: 'WorkbuddyPageFixture.jsx',
  artifactPrefix: 'workbuddy-page-',
  verify: verifyWorkbuddyPage,
});
