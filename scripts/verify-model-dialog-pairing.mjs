import { fileURLToPath } from 'node:url';
import { runBrowserFixture } from './lib/browser-fixture.mjs';
import { verifyModelDialogPairing } from '../web/test/features/coding/shared/modelFormLayout/modelDialogPairingChecks.mjs';

const fixtureDirectory = fileURLToPath(
  new URL('../web/test/features/coding/shared/modelFormLayout/fixtures', import.meta.url),
);

await runBrowserFixture({
  fixtureDirectory,
  fixtureFilename: 'ModelDialogPairingFixture.jsx',
  artifactPrefix: 'model-dialog-pairing-',
  verify: verifyModelDialogPairing,
});
