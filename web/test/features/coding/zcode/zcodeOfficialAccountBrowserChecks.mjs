import assert from 'node:assert/strict';
import { writeFile } from 'node:fs/promises';
import path from 'node:path';

/**
 * Drives the official-account card of the **real** ZCode page in a browser.
 *
 * This host is the odd one of the three: ZCode has no provider row for its
 * official login, so the card is its own list member (inserted at
 * `officialAccountIndex` among the provider cards) and the shared section draws
 * the whole block, heading included. Codex and Kimi render the same block inside
 * a provider card, where the heading is the card's name row.
 *
 * What only a browser can check here, and why each one exists:
 *
 *  - **The hint's colour.** It is the one sentence this host draws through a
 *    different code path than the other two, and it silently came out a shade
 *    darker (`--color-text-secondary` alpha 0.65 vs antd's secondary = alpha
 *    0.45). Types and source guards see nothing; only `getComputedStyle` does.
 *  - **The rows' left edge.** The card used to keep the drag handle inside the
 *    heading line, so the heading was indented while everything under it sat
 *    flush with the card edge — the misalignment the user circled.
 *  - **The collapse.** The ability is shared by all three cards now, and a
 *    caret that renders but cannot be clicked is the failure this catches.
 *
 * Labels are read off the fixture rather than hardcoded, so re-wording is free.
 */
export async function verifyZcodeOfficialAccountCard({ send, evaluate, baseUrl, artifactRoot }) {
  const checks = [];
  const delay = milliseconds => new Promise(resolve => setTimeout(resolve, milliseconds));
  const check = (name, actual, expected = true) => {
    assert.deepEqual(actual, expected, name);
    checks.push(name);
    console.log('PASS ' + name);
  };
  const waitFor = async (expression, label) => {
    for (let attempt = 0; attempt < 400; attempt++) {
      if (await evaluate(expression)) return;
      await delay(125);
    }
    throw new Error('Timed out waiting for: ' + (label || expression));
  };
  const fixture = expression => evaluate('zcodeOfficialAccountFixture.' + expression);
  const screenshot = async name => {
    const { data } = await send('Page.captureScreenshot', { format: 'png' });
    await writeFile(path.join(artifactRoot, name + '.png'), Buffer.from(data, 'base64'));
  };

  const runId = `run-${Date.now()}`;
  await send('Page.navigate', { url: `${baseUrl}/?accounts=2&runId=${runId}` });
  await waitFor(
    `window.zcodeOfficialAccountFixture?.state.runId === ${JSON.stringify(runId)}`,
    'the ZCode page',
  );
  await delay(300);

  const cardId = await fixture('OFFICIAL_ACCOUNT_CARD_ID');
  const headingLabel = await fixture('OFFICIAL_CARD_HEADING');
  const listTitle = await fixture('ACCOUNT_LIST_TITLE');
  await waitFor('zcodeOfficialAccountFixture.renderedMemberIds().length === 4', 'four list members');

  // --- the card is an ordinary list member ---------------------------------
  check(
    'the official card is an ordinary list member, at its saved position',
    await fixture('renderedMemberIds()'),
    ['provider-a', cardId, 'provider-b', 'provider-c'],
  );
  check(
    'it carries the same drag handle as the provider cards',
    await fixture(`hasHandle(${JSON.stringify(cardId)})`),
  );

  // --- the unified account-block shape --------------------------------------
  const cardText = await fixture(`cardText(${JSON.stringify(cardId)})`);
  check(
    'the heading is the unified account label, with the block\'s link glyph',
    await fixture(`hasHeadingGlyph(${JSON.stringify(cardId)})`) && cardText.includes(headingLabel),
  );
  check(
    'the heading carries the account count',
    cardText.includes('(2)'),
  );
  check(
    'the list keeps its own title and a collapse toggle',
    cardText.includes(listTitle) && await fixture(`hasListToggle(${JSON.stringify(cardId)})`),
  );
  check(
    'both saved accounts render a row',
    await fixture(`accountRowCount(${JSON.stringify(cardId)})`),
    2,
  );

  // --- the hint's typography, measured off the rendered page ----------------
  const metrics = await fixture(`lineMetrics(${JSON.stringify(cardId)})`);
  check(
    'the explanation renders under the heading and above the list title',
    Boolean(metrics.hint && metrics.heading && metrics.listTitle)
      && metrics.hint.top >= metrics.heading.bottom
      && metrics.hint.bottom <= metrics.listTitle.top,
  );
  // antd's `secondary` is `colorTextDescription` (alpha 0.45); the app's
  // `--color-text-secondary` variable is alpha 0.65. The hint must use antd's,
  // because that is what Codex and Kimi render the same sentence with.
  check(
    'the explanation uses antd\'s secondary colour, like the other two cards',
    metrics.hint?.color,
    'rgba(0, 0, 0, 0.45)',
  );
  check(
    'the explanation keeps the shared 12px size',
    metrics.hint?.fontSize,
    '12px',
  );

  // --- the rows line up with the title they belong to -----------------------
  // The heading is indented past the drag handle's column; the rows must be
  // indented the same way, or the block reads as two different columns — which
  // is the defect this card actually had (the rows sat flush with the card edge
  // while everything above them was indented).
  //
  // A few pixels of tolerance, deliberately: the rows' indent is the section's
  // fixed 18px, while the title's text starts after the caret, whose rendered
  // width comes from the icon font. The two agree to within ~3px on all three
  // CLIs; pinning them to the pixel would be pinning the font, not the layout.
  check(
    'every account row starts at the list title\'s left edge',
    metrics.rowLefts.every(left => Math.abs(left - metrics.listTitleLeft) <= 4),
  );

  // --- the collapse is real -------------------------------------------------
  const toggleQuery = `zcodeOfficialAccountFixture.listToggle(${JSON.stringify(cardId)})`;
  await evaluate(`${toggleQuery}.click(), true`);
  await delay(400);
  check(
    'collapsing the list from its title hides every row',
    await fixture(`accountRowCount(${JSON.stringify(cardId)})`),
    0,
  );
  await evaluate(`${toggleQuery}.click(), true`);
  await delay(400);
  check(
    'expanding it brings the rows back',
    await fixture(`accountRowCount(${JSON.stringify(cardId)})`),
    2,
  );
  await screenshot('official-card');

  return checks;
}
