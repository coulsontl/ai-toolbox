import { fileURLToPath } from 'node:url';
import { runBrowserFixture } from './lib/browser-fixture.mjs';
import { verifyLinuxRendering } from '../web/test/features/settings/linuxRenderingBrowserChecks.mjs';
await runBrowserFixture({
  fixtureDirectory: fileURLToPath(new URL('../web/test/features/settings/fixtures', import.meta.url)),
  fixtureFilename: 'LinuxRenderingSettingFixture.jsx',
  artifactPrefix: 'linux-rendering-',
  verify: verifyLinuxRendering,
});
