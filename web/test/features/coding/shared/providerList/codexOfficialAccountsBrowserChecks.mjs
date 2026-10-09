import assert from 'node:assert/strict';
import { writeFile } from 'node:fs/promises';
import path from 'node:path';

/**
 * Checks the official-account block on the **real** Codex card in a browser.
 *
 * The block is a *section* of the card, so it has to line up with the card's
 * other sections: the account rows' actions are right-aligned, and so is the
 * model list toolbar under them. Rendering the block inside the header row's
 * content column — which is what the component used to do — silently insets it
 * by the width of the header action links, so every row stopped ~160px short of
 * the buttons below it and the card looked like it had a hole on the right.
 * No type or static check can see that; only the measured geometry can.
 *
 * Labels are read off the page rather than hardcoded so re-wording is free.
 */
export async function verifyCodexOfficialAccountAlignment({ send, evaluate, baseUrl, artifactRoot }) {
  const checks = [];
  const delay = milliseconds => new Promise(resolve => setTimeout(resolve, milliseconds));
  const check = (name, actual, expected = true) => {
    assert.deepEqual(actual, expected, name);
    checks.push(name);
    console.log('PASS ' + name);
  };
  // 50s, not 10s — see the note in the Kimi check: a full `pnpm test` run keeps
  // the machine busy enough that a short budget fails on boot time alone.
  const waitFor = async (expression, label) => {
    for (let attempt = 0; attempt < 400; attempt++) {
      if (await evaluate(expression)) return;
      await delay(125);
    }
    throw new Error('Timed out waiting for: ' + (label || expression));
  };

  // `?accounts=2` turns `provider-a` into the official provider, and
  // `data-provider-id` is the card's stable anchor (the same one the locate
  // action queries). Addressing the card by a title string would break the
  // moment the wording changes — and it did.
  const officialCard = '[data-provider-id="provider-a"]';
  await send('Page.navigate', { url: `${baseUrl}/?accounts=2&runId=align-${Date.now()}` });
  // Wait for the *toggle*, not merely the card box: the card element exists
  // before React has drawn the section inside it, and clicking a button that
  // is not there yet throws.
  const toggleQuery =
    `${officialCard} .ant-card button:has(.anticon-right), `
    + `${officialCard} .ant-card button:has(.anticon-down)`;
  await waitFor(
    `Boolean(document.querySelector('${toggleQuery}'))`,
    'the account section\'s collapse toggle',
  );

  // Expanded through a DOM click rather than a synthetic pointer sequence: the
  // block's default state is collapsed, and a real pointer event here leaves
  // the browser unresponsive to the next command. The toggle is the card's only
  // button carrying the section's collapse caret; the model list below is a
  // sibling of the card, not a child, so this cannot pick that one up.
  await evaluate(`document.querySelector('${toggleQuery}').click(), true`);
  await delay(500);

  const geometry = await evaluate(`(() => {
    const card = document.querySelector('${officialCard} .ant-card');
    if (!card) return null;
    const rightEdgeOf = element => element
      ? Math.round(element.getBoundingClientRect().right)
      : null;
    const styleOf = element => element?.getAttribute('style') ?? '';

    // The header row is two columns: content beside the action links. Its last
    // child is therefore the rightmost thing the header can put on screen, and
    // its edge is the card's content edge.
    const headerRow = [...card.querySelectorAll('div')]
      .find(node => styleOf(node).includes('align-items: flex-start'));
    const headerActions = headerRow ? [...headerRow.children].at(-1) : null;

    // The account rows carry the block's own inline layout, so they are
    // addressed by it rather than by a class name.
    const accountRow = [...card.querySelectorAll('div')].find(node =>
      styleOf(node).includes('space-between') && node.textContent.includes('@example.invalid'));
    const accountActions = accountRow ? [...accountRow.children].at(-1) : null;

    const body = card.querySelector('.ant-card-body');
    const bodyStyle = body ? getComputedStyle(body) : null;
    return {
      renderedAccountRows: [...card.querySelectorAll('div')]
        .filter(node => styleOf(node).includes('border-bottom')
          && node.textContent.includes('@example.invalid')).length,
      accountRowRight: rightEdgeOf(accountActions),
      accountBlockRight: rightEdgeOf(accountActions?.parentElement),
      headerActionsRight: rightEdgeOf(headerActions),
      contentRight: body
        ? Math.round(body.getBoundingClientRect().right - parseFloat(bodyStyle.paddingRight || '0'))
        : null,
    };
  })()`);

  assert.ok(geometry, 'the official-account card must render');
  check('both saved accounts render a row', geometry.renderedAccountRows, 2);
  check(
    'the account rows end at the card content edge, not inset by the header actions',
    geometry.accountRowRight,
    geometry.contentRight,
  );
  check(
    'the account rows share the header actions\' right edge',
    geometry.accountRowRight,
    geometry.headerActionsRight,
  );
  check(
    'the whole account block spans the card, not the header row content column',
    geometry.accountBlockRight,
    geometry.contentRight,
  );

  const { data } = await send('Page.captureScreenshot', { format: 'png' });
  await writeFile(path.join(artifactRoot, 'account-alignment.png'), Buffer.from(data, 'base64'));

  return checks;
}