import assert from 'node:assert/strict';
import { writeFile } from 'node:fs/promises';
import path from 'node:path';

/**
 * Pins the title-to-subtitle distance of the shared card styles.
 *
 * These numbers are not arbitrary: the Claude style stacks its rows on 4px and
 * the meta row carries its own 4px offset, so the gap is 8; the Codex style
 * stacks on 4 alone. Both values existed before the shared cards did — the
 * Claude Code card and the still-bespoke Claude Desktop card both used the
 * 4+4 shape. A migration flattened them onto one value and the Claude Code
 * card silently lost half its gap (2026-10-08 report), which no type or static
 * check can see.
 */
export async function verifyProviderCardSpacing({ send, evaluate, baseUrl, artifactRoot }) {
  const checks = [];
  const delay = milliseconds => new Promise(resolve => setTimeout(resolve, milliseconds));
  const check = (name, actual, expected = true) => {
    assert.deepEqual(actual, expected, name);
    checks.push(name);
    console.log('PASS ' + name);
  };

  await send('Page.navigate', { url: `${baseUrl}/?runId=spacing-${Date.now()}` });
  for (let attempt = 0; attempt < 400; attempt++) {
    if (await evaluate('window.providerCardSpacingFixture && document.querySelectorAll(".ant-card").length === 2')) break;
    await delay(125);
  }
  await delay(300);

  const gap = label => evaluate(
    `providerCardSpacingFixture.titleToMetaGap(${JSON.stringify(label)})`,
  );

  check('the Claude style keeps its 8px title-to-subtitle gap', await gap('AxonHub-Claude'), 8);
  check('the Codex style keeps its 4px title-to-subtitle gap', await gap('AxonHub-Codex'), 4);

  const { data } = await send('Page.captureScreenshot', { format: 'png' });
  await writeFile(path.join(artifactRoot, 'card-spacing.png'), Buffer.from(data, 'base64'));

  return checks;
}