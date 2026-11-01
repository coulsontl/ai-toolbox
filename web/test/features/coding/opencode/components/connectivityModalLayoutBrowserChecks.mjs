import assert from 'node:assert/strict';
import { writeFile } from 'node:fs/promises';
import path from 'node:path';

/**
 * Pins the collapsed-config layout of the shared connectivity test modal, plus
 * the rule that model removal only appears when the caller declares which ids
 * it owns (`removableModelIds`) — being testable is not being deletable.
 *
 * The failure this catches is invisible to types and to static checks: the
 * modal still renders, the form still works, the tests still pass — but the
 * model list the user opened the modal to see sits below the fold behind
 * ~460px of config. The only way to see it is to measure the rendered box.
 */
export async function verifyConnectivityModalLayout({ send, evaluate, baseUrl, artifactRoot }) {
  const checks = [];
  const delay = (milliseconds) => new Promise((resolve) => setTimeout(resolve, milliseconds));
  const check = (name, actual, expected = true) => {
    assert.deepEqual(actual, expected, name);
    checks.push(name);
    console.log('PASS ' + name);
  };

  const waitFor = async (expression, description) => {
    for (let attempt = 0; attempt < 400; attempt++) {
      if (await evaluate(expression)) return;
      await delay(125);
    }
    throw new Error('Timed out waiting for ' + description);
  };

  await send('Page.navigate', { url: `${baseUrl}/?runId=connectivity-${Date.now()}` });
  await waitFor('!!window.connectivityModalFixture && !!document.querySelector(".ant-modal-body")', 'the modal to mount');
  await delay(400);

  // 1. Collapsed by default.
  check('config section starts collapsed', await evaluate('window.connectivityModalFixture.configHeaderExpanded()'), false);
  check(
    'collapsed config keeps its controls out of the tab order',
    await evaluate('window.connectivityModalFixture.hiddenConfigControlsAreNotFocusable()'),
  );

  // 2. The whole point: the model list is visible on open.
  check('model table is inside the first viewport', await evaluate('window.connectivityModalFixture.modelTableInViewport()'));
  check('modal body fits the viewport without scrolling', await evaluate('window.connectivityModalFixture.modalBodyFitsViewport()'));

  const collapsedHeight = await evaluate('window.connectivityModalFixture.configSectionHeight()');
  const distanceToTable = await evaluate('window.connectivityModalFixture.distanceFromBodyTopToTable()');
  console.log(`INFO collapsed config section height: ${Math.round(collapsedHeight)}px`);
  console.log(`INFO body top to model table: ${Math.round(distanceToTable)}px`);
  // Collapsed, the card is nothing but one 21px text row plus its own padding.
  // The first cut kept the section's 18px padding and the header's negative
  // margins on top of it, which left 12px of space above the row and 30px
  // below — a card that read as half empty.
  check('collapsed config section stays a single compact row', collapsedHeight < 50);
  check('collapsed config row is vertically centered in its card', await evaluate('window.connectivityModalFixture.collapsedHeaderIsCentered()'));

  // Collapsed, the header carries the card's padding; expanded, the card does.
  // They have to line up, or the title shifts sideways as the section toggles.
  const collapsedInset = await evaluate('window.connectivityModalFixture.headerInsetFromCardLeft()');
  await evaluate('window.connectivityModalFixture.toggleConfigByClick()');
  const expandedInset = await evaluate('window.connectivityModalFixture.headerInsetFromCardLeft()');
  await evaluate('window.connectivityModalFixture.toggleConfigByClick()');
  console.log(`INFO title inset — collapsed: ${Math.round(collapsedInset)}px, expanded: ${Math.round(expandedInset)}px`);
  check('title does not shift when the section toggles', Math.abs(collapsedInset - expandedInset) <= 1);

  // 3. The summary must not spill out of the header, even with the long model id.
  check('summary row does not overflow the header', await evaluate('window.connectivityModalFixture.summaryDoesNotOverflowHeader()'));

  const summaryText = await evaluate('window.connectivityModalFixture.configSummaryText()');
  console.log('INFO collapsed summary text: ' + JSON.stringify(summaryText));
  check('summary shows the prompt', summaryText.includes('say hi!'));
  check('summary shows the vision state', summaryText.includes('视觉关') || summaryText.includes('Vision off'));
  check('summary shows the default model state', summaryText.includes('未设置') || summaryText.includes('Not Set'));

  await send('Page.captureScreenshot', { format: 'png' }).then(async (result) =>
    writeFile(path.join(artifactRoot, 'collapsed.png'), Buffer.from(result.data, 'base64')));

  // 4. Keyboard path expands it too, and the summary gives way to the hint.
  await evaluate('window.connectivityModalFixture.toggleConfigByKeyboard()');
  check('keyboard (Enter) expands the config section', await evaluate('window.connectivityModalFixture.configHeaderExpanded()'));
  const expandedHeight = await evaluate('window.connectivityModalFixture.configSectionHeight()');
  console.log(`INFO expanded config section height: ${Math.round(expandedHeight)}px`);
  check('expanding actually reveals the form', expandedHeight > collapsedHeight + 200);
  check(
    'expanded config exposes its controls to the tab order',
    await evaluate('window.connectivityModalFixture.hiddenConfigControlsAreNotFocusable()'),
    false,
  );

  await send('Page.captureScreenshot', { format: 'png' }).then(async (result) =>
    writeFile(path.join(artifactRoot, 'expanded.png'), Buffer.from(result.data, 'base64')));

  // 5. The mouse path collapses it again, restoring the first-viewport property.
  await evaluate('window.connectivityModalFixture.toggleConfigByClick()');
  check('clicking the header collapses the config section again', await evaluate('window.connectivityModalFixture.configHeaderExpanded()'), false);
  check('model table is inside the viewport again', await evaluate('window.connectivityModalFixture.modelTableInViewport()'));

  // 6. The collapse introduced a silent-failure path: the prompt is required,
  //    so starting a test with an empty prompt used to reject validation with
  //    the error box hidden inside the collapsed section. It must reopen.
  await evaluate('window.connectivityModalFixture.selectFirstModel()');
  await evaluate('window.connectivityModalFixture.toggleConfigByClick()');
  await evaluate('window.connectivityModalFixture.clearPrompt()');
  await evaluate('window.connectivityModalFixture.toggleConfigByClick()');
  check('config is collapsed again before starting the test', await evaluate('window.connectivityModalFixture.configHeaderExpanded()'), false);
  await evaluate('window.connectivityModalFixture.clickStartTest()');
  check('a rejected validation reopens the collapsed config', await evaluate('window.connectivityModalFixture.configHeaderExpanded()'));
  check(
    'the validation error is visible, not hidden',
    await evaluate('!!document.querySelector(".ant-modal-body .ant-form-item-explain-error")'),
  );

  await send('Page.captureScreenshot', { format: 'png' }).then(async (result) =>
    writeFile(path.join(artifactRoot, 'validation-reopened.png'), Buffer.from(result.data, 'base64')));

  // 7. Narrow viewport. The collapsed card's `padding: 0` ties on specificity
  //    with the later `.sectionCard { padding: 14px }` media rule, so at equal
  //    specificity the media rule won and the card ended up padded twice —
  //    once by the card, once by the header — pushing the title 14px inward
  //    and out of line with the expanded state. Wide viewports hid this.
  await send('Emulation.setDeviceMetricsOverride', { width: 700, height: 900, deviceScaleFactor: 1, mobile: false });
  await send('Page.navigate', { url: `${baseUrl}/?runId=narrow-${Date.now()}` });
  await waitFor('!!window.connectivityModalFixture && !!document.querySelector(".ant-modal-body")', 'the modal to remount at 700px');
  await delay(400);

  const narrowCollapsedInset = await evaluate('window.connectivityModalFixture.headerInsetFromCardLeft()');
  await evaluate('window.connectivityModalFixture.toggleConfigByClick()');
  const narrowExpandedInset = await evaluate('window.connectivityModalFixture.headerInsetFromCardLeft()');
  console.log(`INFO narrow (700px) title inset — collapsed: ${Math.round(narrowCollapsedInset)}px, expanded: ${Math.round(narrowExpandedInset)}px`);
  check('title does not shift at a narrow viewport either', Math.abs(narrowCollapsedInset - narrowExpandedInset) <= 1);

  await evaluate('window.connectivityModalFixture.toggleConfigByClick()');
  check('narrow collapsed row is still vertically centered', await evaluate('window.connectivityModalFixture.collapsedHeaderIsCentered()'));
  check('narrow summary does not overflow', await evaluate('window.connectivityModalFixture.summaryDoesNotOverflowHeader()'));

  await send('Page.captureScreenshot', { format: 'png' }).then(async (result) =>
    writeFile(path.join(artifactRoot, 'narrow-collapsed.png'), Buffer.from(result.data, 'base64')));

  // 8. Removal is opt-in. The fixture passes `onRemoveModels` but no
  //    `removableModelIds`, so every probe can fail without the footer growing
  //    a button — that default used to be "everything the probe touched".
  await send('Emulation.clearDeviceMetricsOverride');
  await send('Page.navigate', { url: `${baseUrl}/?runId=removal-${Date.now()}` });
  await waitFor('!!window.connectivityModalFixture && !!document.querySelector(".ant-modal-body")', 'the modal to remount for the removal check');
  await delay(400);

  check('removal is not offered without removable model ids', await evaluate('window.connectivityModalFixture.removalButtonText()') === null);
  await evaluate('window.connectivityModalFixture.runFailingTest()');
  const failedWithoutRemovable = await evaluate('window.connectivityModalFixture.removalButtonText()');
  console.log('INFO removal button without removableModelIds: ' + JSON.stringify(failedWithoutRemovable));
  check('a failing test still offers no removal without removable model ids', failedWithoutRemovable === null);

  // 9. The same failing test with the list declared does offer removal, and
  //    pre-checks only the declared rows.
  await evaluate('window.connectivityModalFixture.mountModal({ removableModelIds: ["openai/gpt-5.5"] })');
  await delay(400);
  await evaluate('window.connectivityModalFixture.runFailingTest()');
  const removalText = await evaluate('window.connectivityModalFixture.removalButtonText()');
  console.log('INFO removal button with removableModelIds: ' + JSON.stringify(removalText));
  check('declaring removable model ids adds the footer action', removalText !== null);
  // Every row fails, but only the declared id may be pre-checked: that is what
  // keeps a config-only or official id out of the delete path.
  assert.deepEqual(
    await evaluate('window.connectivityModalFixture.checkedModelIds()'),
    ['openai/gpt-5.5'],
    'declaring removable model ids pre-checks exactly those rows',
  );
  checks.push('declaring removable model ids pre-checks exactly those rows');

  await send('Page.captureScreenshot', { format: 'png' }).then(async (result) =>
    writeFile(path.join(artifactRoot, 'removal-offered.png'), Buffer.from(result.data, 'base64')));

  // 10. The whole path a user takes: confirm the Popconfirm, the caller gets
  //     exactly the declared ids, and the modal closes itself.
  const clickInBrowser = async (point) => {
    await send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: point.x, y: point.y });
    await send('Input.dispatchMouseEvent', { type: 'mousePressed', x: point.x, y: point.y, button: 'left', clickCount: 1 });
    await send('Input.dispatchMouseEvent', { type: 'mouseReleased', x: point.x, y: point.y, button: 'left', clickCount: 1 });
  };
  await clickInBrowser(await evaluate('window.connectivityModalFixture.openRemovalPopconfirm()'));
  await delay(500);
  const confirmPoint = await evaluate('window.connectivityModalFixture.confirmButtonPoint()');
  assert.ok(confirmPoint, 'the removal Popconfirm opened');
  await clickInBrowser(confirmPoint);
  await delay(900);
  assert.deepEqual(
    await evaluate('window.connectivityModalFixture.removedModelIds()'),
    ['openai/gpt-5.5'],
    'removal hands the caller exactly the declared failed ids',
  );
  checks.push('removal hands the caller exactly the declared failed ids');
  // antd keeps the dialog mounted through its closing animation.
  for (let attempt = 0; attempt < 40; attempt++) {
    if (await evaluate('window.connectivityModalFixture.modalClosed()')) break;
    await delay(100);
  }
  check('a successful removal closes the modal', await evaluate('window.connectivityModalFixture.modalClosed()'));

  return checks;
}
