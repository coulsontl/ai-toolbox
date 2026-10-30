import assert from 'node:assert/strict';
import { writeFile } from 'node:fs/promises';
import path from 'node:path';

/**
 * Visual + layout checks for the real CodeBuddy page.
 *
 * Same invariants as the WorkBuddy page, plus the two CodeBuddy-only sections:
 * `CODEBUDDY.md` prompt presets and the read-only `settings.json` preview.
 */
export async function verifyCodebuddyPage({ send, evaluate, baseUrl, artifactRoot }) {
  const theme = process.env.CODEBUDDY_PAGE_THEME === 'dark' ? 'dark' : 'light';
  const checks = [];
  const check = (name, actual, expected = true) => {
    assert.deepEqual(actual, expected, name);
    checks.push(name);
    console.log('PASS ' + name);
  };
  const delay = (milliseconds) => new Promise((resolve) => setTimeout(resolve, milliseconds));
  const waitFor = async (expression) => {
    for (let attempt = 0; attempt < 200; attempt += 1) {
      if (await evaluate(expression)) return;
      await delay(50);
    }
    throw new Error('Timed out waiting for: ' + expression);
  };

  await send('Page.navigate', {
    url: `${baseUrl}/?theme=${theme}&runId=codebuddy-page-${Date.now()}`,
  });
  await waitFor('window.__codebuddyFixture && document.querySelectorAll("[data-provider-id]").length > 0');

  const channelCount = await evaluate('window.__codebuddyFixture.providerCardCount()');
  check('both channels render as cards', channelCount, 2);

  const sidebarTitles = JSON.parse(
    await evaluate('JSON.stringify(window.__codebuddyFixture.sidebarSectionTitles())'),
  );
  check('all four sections are marked for the sidebar', sidebarTitles.length, 4);
  check(
    'every sidebar marker carries a title',
    sidebarTitles.every((title) => typeof title === 'string' && title.trim().length > 0),
  );

  // `availableModels` is non-empty here, so the hint must be the restricted
  // variant — the two states are easy to wire to the wrong branch.
  const restrictedHint = await evaluate(
    'document.body.innerText.includes("availableModels")',
  );
  check('the restricted hint names availableModels', restrictedHint);

  const overflowingCards = await evaluate(`(() => {
    return [...document.querySelectorAll('[data-provider-id]')]
      .filter((card) => card.getBoundingClientRect().right > innerWidth + 1).length;
  })()`);
  check('no card overflows the viewport horizontally', overflowingCards, 0);

  const documentScrollsHorizontally = await evaluate(
    'document.documentElement.scrollWidth > document.documentElement.clientWidth + 1',
  );
  check('the page does not scroll horizontally', documentScrollsHorizontally, false);

  const nestedCards = await evaluate(`(() => {
    return [...document.querySelectorAll('[data-provider-id]')]
      .filter((card) => card.parentElement?.closest('[data-provider-id]')).length;
  })()`);
  check('provider cards are not nested inside each other', nestedCards, 0);

  await delay(600);
  const { data } = await send('Page.captureScreenshot', { format: 'png' });
  await writeFile(
    path.join(artifactRoot, `codebuddy-page-${theme}.png`),
    Buffer.from(data, 'base64'),
  );

  return checks;
}
