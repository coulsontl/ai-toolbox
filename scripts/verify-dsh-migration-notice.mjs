import { fileURLToPath } from 'node:url';
import { runBrowserFixture } from './lib/browser-fixture.mjs';
import { verifyDshMigrationNotice } from '../web/test/features/coding/dsh/components/dshMigrationNoticeBrowserChecks.mjs';

const fixtureDirectory = fileURLToPath(new URL(
  '../web/test/features/coding/dsh/components/fixtures',
  import.meta.url,
));
await runBrowserFixture({
  fixtureDirectory,
  fixtureFilename: 'DshMigrationNoticeFixture.jsx',
  artifactPrefix: 'dsh-migration-notice-',
  verify: verifyDshMigrationNotice,
  fixtureAliases: [],
});
