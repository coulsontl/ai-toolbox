import assert from 'node:assert/strict';
import { randomUUID } from 'node:crypto';
import { writeFile } from 'node:fs/promises';
import path from 'node:path';

export async function verifyCodexFetchedModelPicker({ send, evaluate, baseUrl, artifactRoot }) {
  const checks = [];
  const expectedLabels = Array.from({ length: 8 }, (_, index) => `Synthetic Model ${index} (synthetic-model-${index})`);
  const delay = milliseconds => new Promise(resolve => setTimeout(resolve, milliseconds));
  const check = (name, actual, expected = true) => {
    assert.deepEqual(actual, expected, name); checks.push(name); console.log('PASS ' + name);
  };
  const waitFor = async expression => {
    for (let attempt = 0; attempt < 160; attempt++) {
      if (await evaluate(expression)) return;
      await delay(50);
    }
    throw new Error('Timed out: ' + expression);
  };
  const action = expression => evaluate('modelPickerFixture.' + expression);
  const key = async (key, code, windowsVirtualKeyCode) => {
    await send('Input.dispatchKeyEvent', { type: 'keyDown', key, code, windowsVirtualKeyCode });
    await send('Input.dispatchKeyEvent', { type: 'keyUp', key, code, windowsVirtualKeyCode });
    await delay(120);
    if (key === 'Escape') await waitFor('modelPickerFixture.options().length === 0');
  };
  const openPicker = async () => {
    await waitFor('!!modelPickerFixture.pickerInput() && !modelPickerFixture.pickerInput().disabled');
    const point = await evaluate(`(() => {
      const input = modelPickerFixture.pickerInput();
      input.scrollIntoView({ block: 'nearest' });
      const rect = input.getBoundingClientRect();
      return { x: rect.x + Math.min(20, rect.width / 2), y: rect.y + rect.height / 2 };
    })()`);
    await send('Input.dispatchMouseEvent', { type: 'mousePressed', ...point, button: 'left', clickCount: 1 });
    await send('Input.dispatchMouseEvent', { type: 'mouseReleased', ...point, button: 'left', clickCount: 1 });
    await waitFor('modelPickerFixture.options().length > 0');
  };
  const openFixture = async parameters => {
    const runId = randomUUID();
    await send('Page.navigate', { url: `${baseUrl}/?${parameters}&runId=${runId}` });
    await waitFor(`window.modelPickerFixture?.state.runId === ${JSON.stringify(runId)} && modelPickerFixture.modelInput()?.value === 'gpt-6.1-sol'`);
  };
  const screenshot = async name => {
    const { data } = await send('Page.captureScreenshot', { format: 'png' });
    await writeFile(path.join(artifactRoot, name + '.png'), Buffer.from(data, 'base64'));
  };

  for (const [language, theme] of [['zh-CN', 'light'], ['en-US', 'dark']]) {
    const label = `${language}/${theme}`;
    await openFixture(`language=${language}&theme=${theme}`);
    await action('click("codex.fetchModels.button")');
    await waitFor('!!modelPickerFixture.pickerInput()');
    check(label + ' fetch does not replace unknown current model', await action('modelInput().value'), 'gpt-6.1-sol');
    check(label + ' fetch preserves TOML', (await action('toml()')).includes('model = "gpt-6.1-sol"'));
    await openPicker();
    check(label + ' all eight fetched models can be inspected', await action('options()'), expectedLabels);
    await screenshot(language + '-eight-models');
    await action('search("synthetic-model-7")');
    await waitFor('modelPickerFixture.options().length === 1');
    check(label + ' search matches model id', (await action('options()'))[0].includes('synthetic-model-7'));
    await action('search("not-a-model")');
    await waitFor('modelPickerFixture.options().length === 0');
    check(label + ' no-match state is localized', await evaluate('document.body.textContent.includes(modelPickerFixture.t("codex.fetchModels.noMatches"))'));
    await key('Escape', 'Escape', 27);
    check(label + ' Escape cancels browsing without changing model', await action('modelInput().value'), 'gpt-6.1-sol');
    await openPicker();
    check(label + ' reopening clears the previous search', (await action('options()')).length, 8);
    await action('search("synthetic-model-7")');
    await waitFor('modelPickerFixture.options().length === 1');
    await key('ArrowDown', 'ArrowDown', 40);
    await key('Enter', 'Enter', 13);
    await waitFor('modelPickerFixture.modelInput().value === "synthetic-model-7"');
    check(label + ' keyboard selection updates TOML', (await action('toml()')).includes('model = "synthetic-model-7"'));
    check(label + ' unrelated TOML fields survive selection', (await action('toml()')).includes('model_reasoning_effort = "high"'));
    await action('click("common.cancel")');
    check(label + ' cancelling form does not save', await action('state.saved'), null);
    await action('reopen()');
    await waitFor('modelPickerFixture.modelInput()?.value === "gpt-6.1-sol" && !modelPickerFixture.pickerInput()');
    check(label + ' cancelled selection does not persist', await action('modelInput().value'), 'gpt-6.1-sol');
    check(label + ' invokes only fake allowlisted commands', await action('state.requests.every(request => ["read_opencode_config", "proxy_gateway_supported_cli_keys", "fetch_codex_official_models"].includes(request.command))'));
  }

  await openFixture('language=zh-CN');
  await action('deferNext()'); await action('click("codex.fetchModels.button")');
  check('deferred fetch shows loading', await evaluate('!!document.querySelector(".ant-btn-loading")'));
  await action('click("common.cancel")'); await action('reopen()');
  await waitFor('modelPickerFixture.modelInput()?.value === "gpt-6.1-sol" && !modelPickerFixture.pickerInput()');
  await action('click("codex.fetchModels.button")');
  await action('resolveNext(["stale-cancelled-model"])');
  await openPicker();
  check('late cancelled fetch cannot overwrite reopened session', await action('options()'), expectedLabels);
  check('stale result is not visible', !(await action('options()')).some(text => text.includes('stale-cancelled-model')));
  await key('Escape', 'Escape', 27);

  await action('deferNext()'); await action('click("codex.fetchModels.button")');
  check('refresh disables the existing chooser', await evaluate('modelPickerFixture.pickerInput().disabled'));
  await action('switchProvider()');
  await waitFor('!modelPickerFixture.pickerInput() && !document.querySelector(".ant-btn-loading")');
  await action('click("codex.fetchModels.button")');
  await action('resolveNext(["stale-provider-model"])');
  await openPicker();
  check('late result from previous provider cannot overwrite current results', await action('options()'), expectedLabels);
  await key('Escape', 'Escape', 27);
  await action('rejectNext()'); await action('click("codex.fetchModels.button")');
  await openPicker();
  const fallbackLabels = await action('options()');
  check('official failure exposes bundled fallback instead of the previous list', fallbackLabels.some(text => text.includes('gpt-5.2')) && fallbackLabels.every(text => !text.includes('synthetic-model')));
  await key('Escape', 'Escape', 27);

  await send('Emulation.setDeviceMetricsOverride', { width: 620, height: 900, deviceScaleFactor: 1, mobile: false });
  await action('click("codex.fetchModels.button")');
  check('narrow dialog model row remains inside the viewport', await evaluate(`(() => {
    const row = document.querySelector('[class*="modelFieldRow"]').getBoundingClientRect();
    return row.left >= 0 && row.right <= innerWidth;
  })()`));
  await openPicker();
  // The dropdown is read **before** the screenshot, not after. Capturing the
  // page takes long enough on a loaded CI runner that the `Select` can lose
  // focus and close itself, and the assertion then reads an empty list from a
  // picker that opened correctly — the failure says "no models" about a state
  // the screenshot itself shows populated. Asserting first also means the check
  // describes the moment `openPicker` waited for.
  check('narrow picker still exposes all models', await action('options()'), expectedLabels);
  await screenshot('narrow-model-picker');
  check('fixture invoked only fake allowlisted commands', await action('state.requests.every(request => ["read_opencode_config", "proxy_gateway_supported_cli_keys", "fetch_codex_official_models"].includes(request.command))'));
  return checks;
}
