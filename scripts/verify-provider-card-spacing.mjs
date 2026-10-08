import { fileURLToPath } from 'node:url';
import { runBrowserFixture } from './lib/browser-fixture.mjs';
import { verifyProviderCardSpacing } from '../web/test/features/coding/shared/providerCardVariants/providerCardSpacingBrowserChecks.mjs';

const fixtureDirectory = fileURLToPath(new URL(
  '../web/test/features/coding/shared/providerCardVariants/fixtures',
  import.meta.url,
));
await runBrowserFixture({
  fixtureDirectory,
  fixtureFilename: 'ProviderCardSpacingFixture.jsx',
  artifactPrefix: 'provider-card-spacing-',
  verify: verifyProviderCardSpacing,
  fixtureAliases: [],
});