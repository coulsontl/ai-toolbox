import { fileURLToPath } from 'node:url';
import { runBrowserFixture } from './lib/browser-fixture.mjs';
import { verifyCodexOfficialAccountAlignment } from '../web/test/features/coding/shared/providerList/codexOfficialAccountsBrowserChecks.mjs';

const fixtureDirectory = fileURLToPath(new URL(
  '../web/test/features/coding/shared/providerList/fixtures',
  import.meta.url,
));
await runBrowserFixture({
  fixtureDirectory,
  fixtureFilename: 'ProviderListDragFixture.jsx',
  artifactPrefix: 'codex-official-accounts-',
  verify: verifyCodexOfficialAccountAlignment,
  fixtureAliases: [],
});