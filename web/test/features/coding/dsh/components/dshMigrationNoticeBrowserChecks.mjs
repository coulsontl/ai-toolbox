import assert from 'node:assert/strict';
import { writeFile } from 'node:fs/promises';
import path from 'node:path';

/**
 * Pins the DSH migration notice: it must be one dismissible line.
 *
 * Both halves were user reports on 2026-10-22. First the notice "never goes
 * away" (it was a permanent banner with no dismissal), then "I don't see a
 * close button, and the banner is too big" — the × was missing because antd 6
 * only counts an object-form `closable` as closable when it carries
 * `closeIcon`/`closeText`. Nothing in the type system, the i18n check or the
 * unit tests can see that; only rendering it can.
 *
 * The third check ("remembered after a remount") is what makes the first two
 * useful: a × that hides the notice until the next render is not a fix.
 */
export async function verifyDshMigrationNotice({ send, evaluate, baseUrl, artifactRoot }) {
  const checks = [];
  const delay = milliseconds => new Promise(resolve => setTimeout(resolve, milliseconds));
  const check = (name, actual, expected = true) => {
    assert.deepEqual(actual, expected, name);
    checks.push(name);
    console.log('PASS ' + name);
  };
  const waitFor = async (expression, description) => {
    for (let attempt = 0; attempt < 200; attempt++) {
      if (await evaluate(expression)) return;
      await delay(100);
    }
    throw new Error(`Timed out waiting for ${description}`);
  };

  await send('Page.navigate', { url: `${baseUrl}/?runId=dsh-migration-notice-${Date.now()}` });
  // `Runtime.evaluate` runs with `returnByValue`, so the expression must yield
  // a primitive — returning the DOM node itself fails serialization with
  // "Object reference chain is too long".
  await waitFor(
    'Boolean(window.dshMigrationNoticeFixture) && dshMigrationNoticeFixture.exists()',
    'the notice to render',
  );
  await delay(200);

  check(
    'the notice states both paths it moved between',
    await evaluate(`(() => {
      const text = dshMigrationNoticeFixture.noticeText();
      return text.includes(dshMigrationNoticeFixture.expectedPatchPath)
        && text.includes('settings.yaml.imported');
    })()`),
  );
  check(
    'the notice is a single line of text, not a title + description banner',
    await evaluate('dshMigrationNoticeFixture.isSingleLine()'),
  );
  check(
    'the notice has no description block',
    await evaluate('dshMigrationNoticeFixture.hasDescriptionBlock()'),
    false,
  );
  check(
    'the close button is rendered and clickable',
    await evaluate('dshMigrationNoticeFixture.closeButtonIsClickable()'),
  );

  await waitFor(
    '!document.querySelector("[data-dsh-migration-notice]")',
    'the notice to disappear after clicking close',
  );
  check('clicking close hides the notice', await evaluate('!dshMigrationNoticeFixture.exists()'));
  check(
    'the dismissal is recorded against the patch path',
    await evaluate('dshMigrationNoticeFixture.dismissedPathInStorage()'),
    await evaluate('dshMigrationNoticeFixture.expectedPatchPath'),
  );

  // A dismissal that only lasts until the next render is not a fix: remount
  // from storage and confirm the notice stays gone.
  await evaluate('dshMigrationNoticeFixture.reset()');
  await delay(300);
  check(
    'the notice stays hidden after a remount',
    await evaluate('!dshMigrationNoticeFixture.exists()'),
  );

  const { data } = await send('Page.captureScreenshot', { format: 'png' });
  await writeFile(path.join(artifactRoot, 'dsh-migration-notice.png'), Buffer.from(data, 'base64'));

  return checks;
}
