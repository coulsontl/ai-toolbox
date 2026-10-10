import { fileURLToPath } from 'node:url';
import { runBrowserFixture } from './lib/browser-fixture.mjs';
import { verifyZcodeOfficialAccountCard } from '../web/test/features/coding/zcode/zcodeOfficialAccountBrowserChecks.mjs';

const fixtureDirectory = fileURLToPath(new URL(
  '../web/test/features/coding/zcode/fixtures',
  import.meta.url,
));
await runBrowserFixture({
  fixtureDirectory,
  fixtureFilename: 'ZcodeOfficialAccountFixture.jsx',
  artifactPrefix: 'zcode-official-account-',
  verify: verifyZcodeOfficialAccountCard,
  fixtureAliases: [],
});
