import assert from 'node:assert/strict';
import { writeFile } from 'node:fs/promises';
import path from 'node:path';

/**
 * Pins the page-level rhythm of the four "config directory" pages.
 *
 * The number these checks protect is 16: `CodingPageHeader` brings its own 16px
 * bottom margin, and every section owns a 16px bottom margin of its own. A page
 * container that *also* declares `gap: 16px` turns both into 32 without failing
 * anything — `tsc` sees a stylesheet, the i18n check sees keys, and a
 * component-level fixture never renders the page container. That happened twice
 * now (OpenCode 2026-10-19, pi/oh_my_pi/dsh/hermes 2026-10-22), so the geometry
 * is asserted here instead.
 *
 * The tolerance is ±1px for sub-pixel rounding.
 */
const EXPECTED_GAP = 16;

const near = (actual, expected) => typeof actual === 'number' && Math.abs(actual - expected) <= 1;

export async function verifyPageRhythm({ send, evaluate, baseUrl, artifactRoot }) {
  const checks = [];
  const delay = milliseconds => new Promise(resolve => setTimeout(resolve, milliseconds));
  const check = (name, actual, expected = true) => {
    assert.deepEqual(actual, expected, name);
    checks.push(name);
    console.log('PASS ' + name);
  };

  await send('Page.navigate', { url: `${baseUrl}/?runId=page-rhythm-${Date.now()}` });
  for (let attempt = 0; attempt < 400; attempt++) {
    if (await evaluate('window.pageRhythmFixture && document.querySelectorAll("[data-page-shell]").length === 4')) break;
    await delay(125);
  }
  await delay(300);

  for (const [pageKey, pageLabel] of [['pi', 'Pi'], ['omp', 'Oh My Pi'], ['dsh', 'DSH'], ['hermes', 'Hermes']]) {
    const headerGap = await evaluate(
      `pageRhythmFixture.headerGapToFirstSection(${JSON.stringify(pageKey)})`,
    );
    check(
      `${pageLabel}: the header sits ${EXPECTED_GAP}px from the first section (not 32)`,
      near(headerGap, EXPECTED_GAP),
      true,
    );

    const sectionGap = await evaluate(`pageRhythmFixture.sectionGap(${JSON.stringify(pageKey)})`);
    check(
      `${pageLabel}: consecutive sections keep ${EXPECTED_GAP}px between them`,
      near(sectionGap, EXPECTED_GAP),
      true,
    );
  }

  const { data } = await send('Page.captureScreenshot', { format: 'png' });
  await writeFile(path.join(artifactRoot, 'page-rhythm.png'), Buffer.from(data, 'base64'));

  return checks;
}
