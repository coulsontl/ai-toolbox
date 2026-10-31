import assert from 'node:assert/strict';
import { writeFile } from 'node:fs/promises';
import path from 'node:path';

/**
 * Spatial check that paired fields really share a row.
 *
 * The source scan in `scripts/verify-form-modal-layout.mjs` can only see that
 * two `Form.Item`s are inside a `MODEL_FORM_PAIR_STYLE` div. Whether that div
 * actually puts them side by side depends on the grid, the dialog width and the
 * control's intrinsic width — a `<Select mode="multiple">` with a long
 * placeholder can still force a wrap. Only a real render answers it.
 *
 * Measured on `getBoundingClientRect().top`: two fields are "on one row" when
 * their label tops coincide.
 *
 * One browser, several navigations: the fixture picks its dialog from the URL,
 * so each navigation remounts the page with a different one.
 */
const PAIRS = {
  codex: [
    ['支持的思考档位', '默认思考档位'],
    ['输入模态', '输出模态'],
  ],
  grok: [['支持的思考等级', '默认思考等级']],
  kimi: [['支持的思考档位', '默认思考档位']],
  openclaw: [['上下文限制', '输出限制']],
  zcode: [['上下文窗口', '最大输出 Token']],
};

/** Only these run when `MODEL_DIALOG_KEY` is unset. */
const ALL_DIALOGS = Object.keys(PAIRS);

export async function verifyModelDialogPairing({ send, evaluate, baseUrl, artifactRoot }) {
  const theme = process.env.MODEL_DIALOG_THEME === 'dark' ? 'dark' : 'light';
  const selected = process.env.MODEL_DIALOG_KEY
    ? [process.env.MODEL_DIALOG_KEY]
    : ALL_DIALOGS;
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

  for (const dialog of selected) {
    const pairs = PAIRS[dialog];
    if (!pairs) throw new Error('Unknown dialog: ' + dialog);

    await send('Page.navigate', {
      url: `${baseUrl}/?theme=${theme}&dialog=${dialog}&runId=pairing-${dialog}-${Date.now()}`,
    });
    // `evaluate` hands the result back over CDP, so a DOM node fails with
    // "Object reference chain is too long" — every expression must be serializable.
    await waitFor('Boolean(window.__dialogFixture) && Boolean(document.querySelector(".ant-modal"))');
    await delay(400);

    // The advanced panels start collapsed on some dialogs.
    await evaluate(`(() => {
      const toggle = [...document.querySelectorAll('button')]
        .find((node) => node.textContent.includes('高级设置') || node.textContent.includes('Advanced'));
      if (toggle) toggle.click();
    })()`);
    await delay(300);

    const rows = JSON.parse(await evaluate(`(() => {
      const labels = [...document.querySelectorAll('.ant-form-item-label > label')];
      return JSON.stringify(labels.map((label) => ({
        text: label.textContent.replace(':', '').trim(),
        top: Math.round(label.getBoundingClientRect().top),
        left: Math.round(label.getBoundingClientRect().left),
      })));
    })()`));
    const findRow = (text) => rows.find((row) => row.text.includes(text));

    for (const [leftText, rightText] of pairs) {
      const left = findRow(leftText);
      const right = findRow(rightText);
      check(`${dialog} shows "${leftText}"`, Boolean(left));
      check(`${dialog} shows "${rightText}"`, Boolean(right));
      check(
        `${dialog}: "${leftText}" and "${rightText}" share one row`,
        left.top === right.top,
      );
      check(
        `${dialog}: "${rightText}" sits to the right of "${leftText}"`,
        right.left > left.left,
      );
    }

    const shot = await send('Page.captureScreenshot', { format: 'png' });
    await writeFile(
      path.join(artifactRoot, `model-dialog-pairing-${dialog}-${theme}.png`),
      Buffer.from(shot.data, 'base64'),
    );
  }

  return checks;
}
