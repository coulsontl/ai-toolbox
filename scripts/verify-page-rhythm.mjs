import { fileURLToPath } from 'node:url';
import { runBrowserFixture } from './lib/browser-fixture.mjs';
import { verifyPageRhythm } from '../web/test/features/coding/shared/pageRhythm/pageRhythmBrowserChecks.mjs';

const fixtureDirectory = fileURLToPath(new URL(
  '../web/test/features/coding/shared/pageRhythm/fixtures',
  import.meta.url,
));
await runBrowserFixture({
  fixtureDirectory,
  fixtureFilename: 'PageRhythmFixture.jsx',
  artifactPrefix: 'page-rhythm-',
  verify: verifyPageRhythm,
  fixtureAliases: [],
});
