import { fileURLToPath } from 'node:url';
import { runBrowserFixture } from './lib/browser-fixture.mjs';
import { verifyProviderListLocate } from '../web/test/features/coding/shared/providerList/providerListLocateBrowserChecks.mjs';

const fixtureDirectory = fileURLToPath(new URL(
  '../web/test/features/coding/shared/providerList/fixtures',
  import.meta.url,
));
await runBrowserFixture({
  fixtureDirectory,
  fixtureFilename: 'ProviderListDragFixture.jsx',
  artifactPrefix: 'provider-list-locate-',
  verify: verifyProviderListLocate,
  fixtureAliases: [],
});