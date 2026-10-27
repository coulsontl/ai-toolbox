import { fileURLToPath } from 'node:url';
import { runBrowserFixture } from './lib/browser-fixture.mjs';
import { verifyFetchedModelsModal } from '../web/test/features/coding/codex/components/fetchedModelsModalBrowserChecks.mjs';

// The provider form used to own 「导入模型映射」; the catalog moved to the
// provider card in 049e6cc7 and the form's button was removed in 7bb69132, so
// this suite now drives the shared dialog directly with the same provider
// inputs the card passes it.
const fixtureDirectory = fileURLToPath(new URL('../web/test/features/coding/codex/components/fixtures', import.meta.url));
await runBrowserFixture({
  fixtureDirectory,
  fixtureFilename: 'CodexFetchedModelsModalFixture.jsx',
  artifactPrefix: 'codex-fetched-models-modal-',
  verify: verifyFetchedModelsModal,
});
