import assert from 'node:assert/strict';
import { writeFile } from 'node:fs/promises';
import path from 'node:path';

/**
 * Drives the official channel's account section of the **real** Kimi page in a
 * browser.
 *
 * What this guards, and why the real page rather than the section component:
 *
 *  - **The host.** The account list lives inside the official provider card, and
 *    that card has to stay an ordinary list member — draggable, and counted like
 *    any other card. A section-only fixture cannot see the list at all.
 *  - **The row geometry.** The rows must end at the card's content edge and at
 *    the header actions' edge. A block rendered inside the header's content
 *    column is inset by the action links' width — the "empty space on the right"
 *    defect that neither types nor snapshots can see.
 *  - **The virtual row.** A login the app has never captured shows up as a
 *    virtual row whose only action is save; once saved it must stop being
 *    virtual, and the saved row must offer switch/delete instead.
 *
 * Labels are read off the rendered page rather than hardcoded, so re-wording the
 * card does not require editing this file.
 */
export async function verifyKimiOfficialAccountCard({ send, evaluate, baseUrl, artifactRoot }) {
  const checks = [];
  const delay = milliseconds => new Promise(resolve => setTimeout(resolve, milliseconds));
  const check = (name, actual, expected = true) => {
    assert.deepEqual(actual, expected, name);
    checks.push(name);
    console.log('PASS ' + name);
  };
  // 50s, not 10s: the fixture rebuilds and boots the whole page, and in a full
  // `pnpm test` run the machine is still busy shutting down the previous
  // browser. A short budget turns that into a phantom failure.
  const waitFor = async (expression, label) => {
    for (let attempt = 0; attempt < 400; attempt++) {
      if (await evaluate(expression)) return;
      await delay(125);
    }
    throw new Error('Timed out waiting for: ' + (label || expression));
  };
  const fixture = expression => evaluate('kimiOfficialAccountFixture.' + expression);
  const screenshot = async name => {
    const { data } = await send('Page.captureScreenshot', { format: 'png' });
    await writeFile(path.join(artifactRoot, name + '.png'), Buffer.from(data, 'base64'));
  };

  /** A real pointer drag: dnd-kit's PointerSensor only activates past its
   *  `distance` constraint, so the move must overshoot it. */
  const dragCard = async (fromName, toName) => {
    const from = await fixture(`handleCenter(${JSON.stringify(fromName)})`);
    const to = await fixture(`cardCenter(${JSON.stringify(toName)})`);
    await send('Input.dispatchMouseEvent', { type: 'mousePressed', x: from.x, y: from.y, button: 'left', clickCount: 1 });
    const steps = 6;
    for (let step = 1; step <= steps; step++) {
      await send('Input.dispatchMouseEvent', {
        type: 'mouseMoved',
        x: from.x + ((to.x - from.x) * step) / steps,
        y: from.y + ((to.y - from.y) * step) / steps,
        button: 'left',
      });
      await delay(60);
    }
    await send('Input.dispatchMouseEvent', { type: 'mouseReleased', x: to.x, y: to.y, button: 'left', clickCount: 1 });
    await delay(400);
  };

  const openFixture = async query => {
    // No `&` or `=` in the run id: it is appended as a query value, and an
    // unencoded separator would silently split the rest of the query off.
    const runId = `run-${Date.now()}`;
    await send('Page.navigate', { url: `${baseUrl}/?${query}&runId=${runId}` });
    await waitFor(
      `window.kimiOfficialAccountFixture?.state.runId === ${JSON.stringify(runId)}`,
      `fixture ${query}`,
    );
    await delay(300);
  };

  // --- the account section lives inside the official card -------------------
  await openFixture('providers=all&accounts=2');
  // Read after the first load: the fixture defines its globals when it boots.
  const officialName = await fixture('OFFICIAL_PROVIDER_NAME');
  const accountSectionTitle = await fixture('OFFICIAL_ACCOUNT_TITLE');
  await waitFor('kimiOfficialAccountFixture.renderedMembers().length === 4', 'four list members');

  check(
    'the official channel is an ordinary list member, first in created order',
    await fixture('renderedMembers()'),
    [officialName, 'Provider A', 'Provider B', 'Provider C'],
  );
  check(
    'the official card carries the same drag handle as the provider cards',
    await fixture(`hasHandle(${JSON.stringify(officialName)})`),
  );
  check(
    'both saved accounts render a row inside the official card',
    await fixture(`accountRowCount(${JSON.stringify(officialName)})`),
    2,
  );
  await screenshot('populated-list');

  const cardButtons = await fixture(`cardButtonLabels(${JSON.stringify(officialName)})`);
  check('the sign-in entry lives on the official card', cardButtons.includes('登录'));
  check(
    'the list toolbar carries no official-account entry',
    (await fixture('toolbarLabels()')).filter(label => label === accountSectionTitle),
    [],
  );
  check(
    'every saved account can be deleted',
    cardButtons.filter(label => label === '删除').length,
    2,
  );
  check(
    'the account that is not in use offers a switch',
    cardButtons.filter(label => label === '切换').length,
    1,
  );
  // The applied row carries the "default" badge instead of an inert switch: a
  // disabled "switch" next to "default" says the same thing twice.
  check(
    'the applied account offers no switch at all',
    (await fixture(`cardButtonStates(${JSON.stringify(officialName)})`))
      .filter(button => button.label === '切换' || button.label === '默认').map(button => button.label),
    ['切换'],
  );
  check(
    'the applied account is badged as the default',
    await fixture(`cardText(${JSON.stringify(officialName)})`).then(text => text.includes('默认')),
  );
  check(
    'no sentence is rendered twice on the card',
    await fixture(`cardRepeatedSentences(${JSON.stringify(officialName)})`),
    [],
  );

  // --- the rows span the card, not the header's content column --------------
  const alignment = await fixture(`accountAlignment(${JSON.stringify(officialName)})`);
  check(
    'the account rows end at the card content edge',
    alignment.rowRight,
    alignment.contentRight,
  );
  if (alignment.headerRight !== null) {
    check(
      'the account rows share the header actions\' right edge',
      alignment.rowRight,
      alignment.headerRight,
    );
  }

  // --- a live login with no stored row is offered as save-only --------------
  await openFixture('providers=all&accounts=virtual');
  await waitFor('kimiOfficialAccountFixture.accountRowCount() === 1', 'the virtual row');
  const virtualButtons = await fixture(`cardButtonStates(${JSON.stringify(officialName)})`);
  check(
    'the uncaptured live login is marked as the current login',
    await fixture(`cardText(${JSON.stringify(officialName)})`).then(text => text.includes('当前登录')),
  );
  check(
    'the virtual row offers save instead of switch or delete',
    virtualButtons.filter(button => ['保存当前登录', '切换', '删除'].includes(button.label)).map(button => button.label),
    ['保存当前登录'],
  );
  await screenshot('virtual-row');

  // Saving captures it, which is what makes the virtual row disappear.
  await evaluate(
    `[...document.querySelectorAll('#kimi-providers button')]`
    + `.find(button => button.textContent.includes('保存当前登录'))?.click()`,
  );
  await delay(600);
  await waitFor(
    `kimiOfficialAccountFixture.cardButtonLabels(${JSON.stringify(officialName)}).includes('切换')`,
    'the saved row to offer switch',
  );
  check(
    'saving turns the live login into a stored account with switch and delete',
    (await fixture(`cardButtonLabels(${JSON.stringify(officialName)})`))
      .filter(label => ['保存当前登录', '切换', '删除'].includes(label)).sort(),
    ['删除', '切换'].sort(),
  );

  // --- dragging the official card reorders it among the providers -----------
  await openFixture('providers=all&accounts=2');
  await waitFor('kimiOfficialAccountFixture.renderedMembers().length === 4', 'four list members');
  await dragCard(officialName, 'Provider C');
  check(
    'dragging the official card past every provider moves it to the bottom',
    await fixture('waitForMembers(["Provider A","Provider B","Provider C",' + JSON.stringify(officialName) + '])'),
    ['Provider A', 'Provider B', 'Provider C', officialName],
  );
  check(
    'the reorder payload carries provider ids only — the card is a provider',
    await fixture('lastReorder()'),
    ['provider-a', 'provider-b', 'provider-c', 'provider-official'],
  );
  await screenshot('dragged-to-bottom');

  // --- no official row: the list still works, and there is no card ----------
  await openFixture('providers=custom-only&accounts=0');
  await waitFor('kimiOfficialAccountFixture.renderedMembers().length === 3', 'three provider cards');
  check(
    'an install with no official channel renders no official card',
    await fixture('renderedMembers()'),
    ['Provider A', 'Provider B', 'Provider C'],
  );
  check(
    'no account section is rendered anywhere',
    await fixture('accountRowCount()'),
    0,
  );
  await screenshot('no-official-channel');

  return checks;
}
