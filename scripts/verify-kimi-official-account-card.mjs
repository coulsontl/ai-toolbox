import { fileURLToPath } from 'node:url';
import { runBrowserFixture } from './lib/browser-fixture.mjs';
import { verifyKimiOfficialAccountCard } from '../web/test/features/coding/kimi/kimiOfficialAccountBrowserChecks.mjs';

const fixtureDirectory = fileURLToPath(new URL(
  '../web/test/features/coding/kimi/fixtures',
  import.meta.url,
));
await runBrowserFixture({
  fixtureDirectory,
  fixtureFilename: 'KimiOfficialAccountFixture.jsx',
  artifactPrefix: 'kimi-official-account-',
  verify: verifyKimiOfficialAccountCard,
  fixtureAliases: [],
});