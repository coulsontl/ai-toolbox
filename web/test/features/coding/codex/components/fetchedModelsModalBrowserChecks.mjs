import assert from 'node:assert/strict';
import { randomUUID } from 'node:crypto';
import { writeFile } from 'node:fs/promises';
import path from 'node:path';

/**
 * Drives the shared `FetchModelsModal` — the dialog behind the Codex card's
 * 「获取模型」 action — and asserts what reaches the backend and what the user
 * can observe.
 *
 * These claims used to live in `codexModelImportBrowserChecks.mjs`, mounted
 * through the provider form. The catalog moved to the card (2026-09-19) and the
 * form's import button was removed (2026-10-17), which left that suite with no
 * way to open the dialog. The dialog itself did not change owner: the page now
 * passes it the provider's derived `sdkType`, `baseUrl` and API key, so the
 * protocol → request mapping below is still the thing that can silently break
 * and still the thing no unit test can see.
 */
export async function verifyFetchedModelsModal({ send, evaluate, baseUrl, artifactRoot }) {
  const checks = [];
  const delay = (milliseconds) => new Promise((resolve) => setTimeout(resolve, milliseconds));
  const check = (name, actual, expected = true) => {
    assert.deepEqual(actual, expected, name);
    checks.push(name);
    console.log('PASS ' + name);
  };
  const waitFor = async (expression, label) => {
    for (let attempt = 0; attempt < 160; attempt += 1) {
      if (await evaluate(expression)) return;
      await delay(50);
    }
    throw new Error('Timed out: ' + (label || expression));
  };
  const action = (expression) => evaluate('fetchedModelsModalFixture.' + expression);
  const screenshot = async (name) => {
    const { data } = await send('Page.captureScreenshot', { format: 'png' });
    await writeFile(path.join(artifactRoot, name + '.png'), Buffer.from(data, 'base64'));
  };
  const openFixture = async (query) => {
    const runId = randomUUID();
    await send('Page.navigate', { url: `${baseUrl}/?${query}&runId=${runId}` });
    await waitFor(
      `window.fetchedModelsModalFixture?.state.runId === ${JSON.stringify(runId)}`
        + ' && fetchedModelsModalFixture.visibleModalCount() === 1',
      `fixture ${query}`,
    );
    await delay(200);
  };
  const standardModels = [
    { id: 'model-alpha', name: 'API Alpha', ownedBy: 'openai' },
    { id: 'model-beta', name: 'API Beta', ownedBy: 'openai' },
    { id: 'claude-opus-4-8', ownedBy: 'anthropic' },
  ];

  // --- the provider's protocol decides the discovery request ---------------
  await openFixture('scenario=responses');
  await action(`setModels(${JSON.stringify(standardModels)})`);
  await action('fetchModels()');
  await waitFor('fetchedModelsModalFixture.rows().length === 3', 'three fetched rows');
  check(
    'an OpenAI Responses provider discovers through the compatible /models path',
    await action('lastRequest().args.request'),
    {
      providerId: 'provider-responses',
      baseUrl: 'https://responses.example.test/v1',
      apiKey: 'sk-responses-fixture',
      apiType: 'openai_compat',
      sdkType: '@ai-sdk/openai',
      customUrl: 'https://responses.example.test/v1/models',
    },
  );
  check('the same dialog shows the fetched rows', await action('rows()'), [
    'model-alpha', 'model-beta', 'claude-opus-4-8',
  ]);
  check('priority owner is pinned ahead of the alphabetical group', (await action('rows()'))[0], 'model-alpha');
  await screenshot('responses-fetched');

  // Anthropic can discover natively: the radio appears and is preselected.
  await openFixture('scenario=anthropic');
  await action(`setModels(${JSON.stringify(standardModels)})`);
  await action('fetchModels()');
  await waitFor('fetchedModelsModalFixture.rows().length === 3', 'anthropic rows');
  check(
    'an Anthropic provider discovers natively and defaults to that mode',
    await action('lastRequest().args.request.apiType'),
    'native',
  );
  check(
    'the Anthropic request carries its SDK so the backend picks the right endpoint',
    await action('lastRequest().args.request.sdkType'),
    '@ai-sdk/anthropic',
  );

  // Google native: versioned path, and the key becomes a query parameter.
  await openFixture('scenario=google');
  await action(`setModels(${JSON.stringify(standardModels)})`);
  check('a Google provider offers both discovery modes', await action('apiTypeOptionCount()'), 2);
  await action('fetchModels()');
  await waitFor('fetchedModelsModalFixture.rows().length === 3', 'google rows');
  check(
    'Google native discovery keeps an explicit version and appends the query key',
    await action('lastRequest().args.request.customUrl'),
    'https://google.example.test/v1/models?key=sk-google%2Bfixture',
  );

  // Switching mode rebuilds the URL preview, and so does every fetch.
  await action('setApiType("openai_compat")');
  check(
    'switching to compatibility drops the native version and query key',
    await action('urlValue()'),
    'https://google.example.test/v1/models',
  );
  const requestsBeforeRefetch = await action('state.requests.length');
  await action('fetchModels()');
  // Wait for the *new* request, not just "any request": two are already on the
  // record by now, so a bare `> 0` would pass before the refetch ever happened.
  await waitFor(
    `fetchedModelsModalFixture.state.requests.length > ${requestsBeforeRefetch}`,
    'refetch recorded',
  );
  check(
    'the rebuilt URL is the one actually sent',
    await action('lastRequest().args.request.customUrl'),
    'https://google.example.test/v1/models',
  );

  // A stored base URL without a version segment gets the native default.
  await openFixture('scenario=google-unversioned');
  await action(`setModels(${JSON.stringify(standardModels)})`);
  await action('fetchModels()');
  await waitFor('fetchedModelsModalFixture.rows().length === 3', 'unversioned google rows');
  check(
    'a base URL with no version segment gets the native default version',
    await action('lastRequest().args.request.customUrl'),
    'https://google-unversioned.example.test/v1beta/models?key=sk-google-fixture',
  );

  // --- refreshing keeps the fetched rows, a failure replaces neither ------
  await openFixture('scenario=responses');
  await action(`setModels(${JSON.stringify(standardModels)})`);
  await action('fetchModels()');
  await waitFor('fetchedModelsModalFixture.rows().length === 3', 'initial rows');
  // Defer the refresh so the in-flight state is observable, proving the
  // assertion below is not vacuously true.
  await action('deferNextFetch()');
  await action('refreshModels()');
  check('an in-flight fetch marks the fetch control as loading', await action('fetchButtonLoading()'), true);
  await action('failPending("Fixture discovery failed")');
  await waitFor('fetchedModelsModalFixture.errorVisible()', 'error alert');
  check('a failed refresh keeps the previous rows visible', await action('rows()'), [
    'model-alpha', 'model-beta', 'claude-opus-4-8',
  ]);
  check(
    'a failed refresh explains itself instead of reporting success',
    (await action('bodyText()')).includes('Fixture discovery failed'),
  );
  check(
    'a failed refresh clears the loading state on the fetch control',
    await action('fetchButtonLoading()'),
    false,
  );
  await screenshot('failed-refresh');

  // --- selection is explicit and survives filtering ------------------------
  await openFixture('scenario=responses');
  await action(`setModels(${JSON.stringify(standardModels)})`);
  await action('fetchModels()');
  await waitFor('fetchedModelsModalFixture.rows().length === 3', 'selection rows');
  check('apply starts disabled with nothing selected', await action('confirmButtonDisabled()'), true);
  await action('select("model-alpha")');
  await action('search("model-beta")');
  await waitFor('fetchedModelsModalFixture.rows().length === 1', 'filtered to one row');
  await action('select("model-beta")');
  await action('apply()');
  await waitFor('!!fetchedModelsModalFixture.state.applied', 'applied result');
  check(
    'a selection made under one search survives selecting under another',
    (await action('state.applied.selectedModels')).map((model) => model.id),
    ['model-alpha', 'model-beta'],
  );
  check(
    'the applied order follows the displayed sorted list, not the click order',
    await action('state.applied.orderedModelIds'),
    ['model-alpha', 'model-beta', 'claude-opus-4-8'],
  );
  check(
    'applying without opting into cleanup removes nothing',
    await action('state.applied.removedModelIds'),
    [],
  );

  // --- removing models requires an explicit opt-in -------------------------
  // `existing` starts with `model-kept` in the provider catalog; the upstream
  // returns `model-alpha` plus `model-kept` so both the "already exists" tag and
  // the missing-from-fetch cleanup count are observable in one pass.
  await openFixture('scenario=existing');
  await action('setModels([{"id":"model-alpha","ownedBy":"openai"},{"id":"model-kept","ownedBy":"openai"}])');
  await action('fetchModels()');
  await waitFor('fetchedModelsModalFixture.rows().length === 2', 'existing scenario rows');
  check('an already-present model is shown and tagged', await action('rowHasExistingTag("model-kept")'), true);
  check('an already-present model cannot be selected again', await action('rowDisabled("model-kept")'), true);
  await action('select("model-alpha")');
  await action('apply()');
  await waitFor('!!fetchedModelsModalFixture.state.applied', 'applied without cleanup');
  check(
    'a fluctuating upstream list does not implicitly remove local models',
    await action('state.applied.removedModelIds'),
    [],
  );

  await action('reopen()');
  await waitFor('fetchedModelsModalFixture.visibleModalCount() === 1', 'reopened dialog');
  // Now the upstream no longer returns `model-kept`, so it becomes removable.
  await action('setModels([{"id":"model-alpha","ownedBy":"openai"}])');
  await action('fetchModels()');
  await waitFor('fetchedModelsModalFixture.rows().length === 1', 'reopened row');
  check(
    'a model missing from the fetch is counted but not removed by default',
    (await action('bodyText()')).includes(
      await evaluate('fetchedModelsModalFixture.t("opencode.fetchModels.removeMissing", { count: 1 })'),
    ),
  );
  await action('select("model-alpha")');
  await action('toggleCleanup()');
  await action('apply()');
  await waitFor('!!fetchedModelsModalFixture.state.applied', 'applied with cleanup');
  check(
    'opting into cleanup removes only the models missing from the fetch',
    await action('state.applied.removedModelIds'),
    ['model-kept'],
  );

  // --- a closed dialog cannot apply a late response -----------------------
  await openFixture('scenario=responses');
  await action('deferNextFetch()');
  await action('fetchModels()');
  await action('cancel()');
  await waitFor('fetchedModelsModalFixture.visibleModalCount() === 0', 'dialog closed');
  await action('reopen()');
  await waitFor('fetchedModelsModalFixture.visibleModalCount() === 1', 'reopened after cancel');
  await action(`setModels(${JSON.stringify(standardModels)})`);
  await action('fetchModels()');
  await waitFor('fetchedModelsModalFixture.rows().length === 3', 'current rows');
  // The cancelled request resolves now, carrying a model the user must never see.
  await action('resolvePending([{"id":"stale-cancelled-model","ownedBy":"openai"}])');
  await delay(300);
  check('a response from a cancelled dialog cannot replace the current rows', await action('rows()'), [
    'model-alpha', 'model-beta', 'claude-opus-4-8',
  ]);
  check(
    'the stale model is not visible anywhere',
    !(await action('bodyText()')).includes('stale-cancelled-model'),
  );

  // --- the dialog is laid out for a narrow window -------------------------
  await send('Emulation.setDeviceMetricsOverride', { width: 620, height: 900, deviceScaleFactor: 1, mobile: false });
  await action(`setModels(${JSON.stringify(standardModels)})`);
  await action('refreshModels()');
  await waitFor('fetchedModelsModalFixture.rows().length === 3', 'narrow rows');
  check(
    'the narrow dialog stays inside the viewport',
    await evaluate('document.documentElement.scrollWidth <= innerWidth'),
  );
  await screenshot('narrow-fetched');
  check(
    'the narrow dialog still lists every fetched model',
    await action('rows()'),
    ['model-alpha', 'model-beta', 'claude-opus-4-8'],
  );

  // --- the other language renders the same dialog -------------------------
  await send('Emulation.setDeviceMetricsOverride', { width: 1280, height: 900, deviceScaleFactor: 1, mobile: false });
  await openFixture('scenario=responses&language=en-US&theme=dark');
  await action(`setModels(${JSON.stringify(standardModels)})`);
  await action('fetchModels()');
  await waitFor('fetchedModelsModalFixture.rows().length === 3', 'english rows');
  check(
    'the English dialog renders the same rows',
    await action('rows()'),
    ['model-alpha', 'model-beta', 'claude-opus-4-8'],
  );
  await screenshot('english-fetched');

  check(
    'the fixture invoked only the allowlisted discovery command',
    await evaluate('fetchedModelsModalFixture.state.requests.every(r => r.command === "fetch_provider_models")'),
  );
  return checks;
}
