import { fileURLToPath } from 'node:url';
import { runBrowserFixture } from './lib/browser-fixture.mjs';
import { verifyProviderListDrag } from '../web/test/features/coding/shared/providerList/providerListDragBrowserChecks.mjs';

const fixtureDirectory = fileURLToPath(new URL(
  '../web/test/features/coding/shared/providerList/fixtures',
  import.meta.url,
));
await runBrowserFixture({
  fixtureDirectory,
  fixtureFilename: 'ProviderListDragFixture.jsx',
  artifactPrefix: 'provider-list-drag-',
  verify: verifyProviderListDrag,
  fixtureAliases: [],
});
