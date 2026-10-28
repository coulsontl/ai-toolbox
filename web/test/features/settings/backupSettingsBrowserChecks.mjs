import assert from 'node:assert/strict';
import { writeFile } from 'node:fs/promises';
import path from 'node:path';

export async function verifyBackupSettings({ send, evaluate, baseUrl, artifactRoot }) {
  const checks = [];
  const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
  const waitFor = async expression => {
    for (let attempt = 0; attempt < 100; attempt++) {
      if (await evaluate(expression)) return;
      await delay(50);
    }
    throw new Error('Timed out: ' + expression);
  };
  const record = (name, actual) => checks.push({ name, actual });
  const call = (method, ...args) => evaluate('backupFixture.' + method + '(' + args.map(JSON.stringify).join(',') + ')');
  const navigate = async query => {
    await send('Page.navigate', { url: baseUrl + '/?' + new URLSearchParams(query) });
    await waitFor('!!window.backupFixture && !!document.querySelector("[data-testid=open-settings]")');
  };
  const openSettings = async () => {
    await call('open');
    await waitFor('backupFixture.canSave()');
    await delay(350);
  };
  const screenshot = async name => {
    await delay(350);
    const { data } = await send('Page.captureScreenshot', { format: 'png' });
    await writeFile(path.join(artifactRoot, name + '.png'), Buffer.from(data, 'base64'));
  };
  const assertFade = async (name, shouldOverflow) => {
    await waitFor('!!document.querySelector("[data-scroll-fade-visible]")');
    await waitFor('document.querySelector("[data-scroll-fade-visible]").dataset.scrollFadeVisible === ' + JSON.stringify(String(shouldOverflow)));
    const measured = await evaluate('(() => {'
      + 'const body=document.querySelector(".ant-modal-body");'
      + 'const fade=body.querySelector("[data-scroll-fade-visible]");'
      + 'const before=body.scrollHeight;fade.style.display="none";const without=body.scrollHeight;fade.style.display="";'
      + 'const footer=document.querySelector(".ant-modal-footer");'
      + 'return {height:body.clientHeight,scrollHeight:before,without,scrollWidth:body.scrollWidth,width:body.clientWidth,'
      + 'anchorHeight:fade.getBoundingClientRect().height,anchorBottom:fade.getBoundingClientRect().bottom,'
      + 'bodyBottom:body.getBoundingClientRect().bottom,pointerEvents:getComputedStyle(fade).pointerEvents,'
      + 'hidden:fade.getAttribute("aria-hidden"),footerBottom:footer?.getBoundingClientRect().bottom,viewport:innerHeight};'
      + '})()');
    assert.equal(measured.scrollHeight, measured.without, name + ': fade must not create scroll height');
    assert.equal(measured.anchorHeight, 0, name + ': overlay anchor has no flow height');
    assert.equal(measured.pointerEvents, 'none');
    assert.equal(measured.hidden, 'true');
    assert.ok(measured.scrollWidth <= measured.width + 1, name + ': no horizontal overflow');
    assert.ok(!measured.footerBottom || measured.footerBottom < measured.viewport, name + ': footer visible');
    if (shouldOverflow) {
      assert.ok(measured.scrollHeight > measured.height);
      assert.ok(Math.abs(measured.anchorBottom - measured.bodyBottom) < 2, name + ': fade follows visible bottom');
    }
    record(name, measured);
  };

  for (const scenario of [
    { theme: 'light', language: 'zh-CN', system: 'light' },
    { theme: 'dark', language: 'en-US', system: 'dark' },
    { theme: 'system', language: 'en-US', system: 'light' },
    { theme: 'system', language: 'zh-CN', system: 'dark' },
  ]) {
    const name = scenario.theme + '-' + scenario.system;
    await send('Emulation.setEmulatedMedia', { features: [{ name: 'prefers-color-scheme', value: scenario.system }] });
    for (const channel of ['local', 'webdav', 'repository']) {
      await navigate({ ...scenario, channel });
      await openSettings();
      if (channel === 'local') {
        await call('toggle', 'settings.autoBackup.title');
        await call('number', 'settings.autoBackup.interval', '2');
        await call('number', 'settings.autoBackup.maxKeep', '3');
      }
      if (channel === 'repository') {
        assert.equal(await evaluate('document.querySelector("#repository_directory").value'), '', 'root directory remains empty');
        await assertFade(name + ' settings initial overflow', true);
        await screenshot(name + '-settings');
        await evaluate('document.querySelector(".ant-modal-body").scrollTop=100000');
        await waitFor('document.querySelector("[data-scroll-fade-visible]").dataset.scrollFadeVisible === "false"');
        await send('Emulation.setDeviceMetricsOverride', { width: 1280, height: 1400, deviceScaleFactor: 1, mobile: false });
        await assertFade(name + ' no overflow after resize', false);
        await send('Emulation.setDeviceMetricsOverride', { width: 800, height: 650, deviceScaleFactor: 1, mobile: false });
        await assertFade(name + ' narrow settings', true);
        await screenshot(name + '-narrow');
        await send('Emulation.setDeviceMetricsOverride', { width: 1280, height: 900, deviceScaleFactor: 1, mobile: false });
      }
      await call('click', 'common.save');
      await waitFor('backupFixture.state.closed');
      const saved = await evaluate('backupFixture.savedRequests()');
      assert.equal(saved.length, 1);
      const payload = saved[0].args.payload;
      assert.equal(payload.backup_type, channel);
      assert.equal(payload.webdav.url, 'https://dav.example.invalid');
      assert.equal(payload.repository.owner, 'fixture-owner');
      assert.equal(payload.repository.directory, '');
      assert.equal(payload.encryption_password, undefined);
      if (channel === 'local') {
        assert.deepEqual(await call('autoSettings'), { enabled: true, interval: 2, maxKeep: 3 });
        await delay(350);
        await openSettings();
        assert.deepEqual(await call('autoSettings'), { enabled: true, interval: 2, maxKeep: 3 });
        await call('click', 'common.save');
        await waitFor('backupFixture.state.closed');
        const savedAgain = await evaluate('backupFixture.savedRequests().at(-1).args.payload');
        assert.equal(savedAgain.auto_backup_enabled, true);
        assert.equal(savedAgain.auto_backup_interval_days, 2);
        assert.equal(savedAgain.auto_backup_max_keep, 3);
      }
      record(name + ' ' + channel + ' settings round trip', payload.backup_type);
    }

    await navigate({ ...scenario, deferFiles: '1' });
    await call('open', 'remote');
    await waitFor('backupFixture.state.pendingFileLoads.length === 1');
    await assertFade(name + ' remote loading', false);
    await call('resolveFiles');
    await waitFor('document.querySelectorAll(".ant-list-item").length === 24');
    await assertFade(name + ' remote loaded', true);
    await screenshot(name + '-remote');
    await evaluate('document.querySelector(".ant-modal-body").scrollTop=100000');
    await waitFor('document.querySelector("[data-scroll-fade-visible]").dataset.scrollFadeVisible === "false"');
  }

  await navigate({ theme: 'light', channel: 'repository', deferLoad: '1' });
  await call('open');
  await waitFor('backupFixture.state.pendingLoads.length === 1');
  assert.equal(await call('canSave'), false);
  await call('click', 'common.save');
  assert.equal((await call('savedRequests')).length, 0);
  await call('resolveLoad');
  await waitFor('backupFixture.canSave()');
  record('repository cannot be cleared before its stored connection loads', true);

  await call('input', '#repository_owner', '');
  await call('channel', 'local');
  await call('click', 'common.save');
  await waitFor('backupFixture.state.closed');
  assert.equal((await call('savedRequests'))[0].args.payload.repository.owner, 'fixture-owner');
  record('hidden incomplete repository draft preserves loaded connection', true);

  await navigate({ theme: 'light', channel: 'repository' });
  await openSettings();
  await call('input', 'input[autocomplete="new-password"]', 'fixture-github-token');
  await call('platform', 'Gitee');
  assert.equal(await evaluate('document.querySelector("input[autocomplete=new-password]").value'), '');
  await call('click', 'settings.webdav.testConnection');
  const testRequest = await evaluate('backupFixture.state.requests.find(request => request.command==="test_backup_repository_connection")');
  assert.equal(testRequest.args.config.platform, 'gitee');
  assert.equal(testRequest.args.token, null);
  record('platform switch clears draft token before connection test', true);

  await navigate({ theme: 'dark', channel: 'webdav', language: 'en-US' });
  await openSettings();
  await call('toggle', 'settings.backupSettings.encryption.title');
  const passwordLabel = await evaluate('(() => {const label=[...document.querySelectorAll(".ant-form-item-label label")].find(label=>label.textContent==="Encryption password");return label?{width:label.clientWidth,scrollWidth:label.scrollWidth}:null;})()');
  assert.ok(passwordLabel, 'the encryption password has a visible label');
  assert.ok(passwordLabel.scrollWidth <= passwordLabel.width + 1, 'the English password label is not clipped');
  await call('input', 'input[autocomplete="new-password"]', 'fixture-encryption-password');
  await evaluate('backupFixture.state.failSave=true');
  await call('click', 'common.save');
  await waitFor('backupFixture.canSave()');
  assert.equal(await evaluate('backupFixture.state.closed'), false);
  assert.equal(await evaluate('document.querySelector("input[autocomplete=new-password]").value'), 'fixture-encryption-password');
  await call('click', 'common.save');
  await waitFor('backupFixture.state.closed');
  await delay(350);
  await openSettings();
  assert.equal(await evaluate('document.querySelector("input[autocomplete=new-password]").value'), '');
  record('failed save retains password draft; successful close clears it', true);

  await navigate({ theme: 'light', blank: '1', channel: 'local' });
  await openSettings();
  await call('click', 'common.save');
  await waitFor('backupFixture.state.closed');
  record('fresh local settings save without a repository connection', true);

  // Directory entries support regex exclusions: the section only shows for the
  // directory type, invalid patterns block the save inline, and valid patterns
  // round-trip into the saved payload.
  await navigate({ theme: 'light', language: 'zh-CN', channel: 'local' });
  await openSettings();
  await call('click', 'settings.backupSettings.customEntries.add');
  await waitFor('backupFixture.visibleModalCount() === 2');
  assert.equal(
    await call('nestedFormItem', 'settings.backupSettings.customEntries.excludePatterns').then(() => true).catch(() => false),
    false,
    'file entries do not show the exclude section',
  );
  await call('nestedSelectOption', 'settings.backupSettings.customEntries.type', 'settings.backupSettings.customEntries.directory');
  assert.equal(await call('nestedExcludePatternCount'), 0);
  await call('nestedInput', 'settings.backupSettings.customEntries.name', 'memories');
  await call('nestedInput', 'settings.backupSettings.customEntries.sourcePath', '~/.agents/memories');
  await call('nestedClickButton', 'settings.backupSettings.customEntries.addExcludePattern');
  assert.equal(await call('nestedExcludePatternCount'), 1);
  await screenshot('custom-entry-exclude-modal');
  // The frontend check is a *Rust-syntax* pre-check, not a regex validator:
  // structural typos like `[` are deliberately passed through to the backend
  // compiler (see `rustRegexCompat.ts`). Use a construct Rust rejects but
  // JavaScript accepts — a backreference — so the inline error is the Rust
  // incompatibility message and not a false positive.
  await call('nestedInput', 'settings.backupSettings.customEntries.excludePatterns', '(a)\\1', 0);
  await call('nestedClickButton', 'common.save');
  assert.equal(await call('visibleModalCount'), 2, 'an incompatible pattern keeps the entry modal open');
  assert.ok(await evaluate('!!document.querySelector(".ant-form-item-explain-error")'), 'an incompatible pattern shows an inline error');
  // This scenario runs in zh-CN; assert the specific Rust-incompatibility
  // message rather than the generic "invalid" one.
  assert.ok(
    await evaluate('document.body.textContent.includes("反向引用")'),
    'the inline error names the Rust incompatibility, not a generic failure',
  );
  await call('nestedInput', 'settings.backupSettings.customEntries.excludePatterns', '^local$', 0);
  await call('nestedClickButton', 'settings.backupSettings.customEntries.addExcludePattern');
  await call('nestedInput', 'settings.backupSettings.customEntries.excludePatterns', '(^|/)\\.git$', 1);
  await call('nestedClickButton', 'common.save');
  await waitFor('backupFixture.visibleModalCount() === 1');
  await screenshot('custom-entry-exclude-list');
  assert.ok(
    await evaluate('document.body.textContent.includes("^local$") && document.body.textContent.includes(".git$")'),
    'the entry list shows the configured exclude rules',
  );
  await call('click', 'common.save');
  await waitFor('backupFixture.state.closed');
  const excludePayload = (await call('savedRequests')).at(-1).args.payload.backup_custom_entries;
  assert.deepEqual(excludePayload.map(entry => entry.exclude_patterns), [['^local$', '(^|/)\\.git$']]);
  assert.equal(excludePayload[0].entry_type, 'directory');
  record('directory exclude rules validate inline and round trip', true);

  // Reopening the saved entry must pre-fill the rules and a file-type switch
  // must drop them, so editing never silently loses or keeps stale patterns.
  await openSettings();
  await call('clickLabel', 'settings.backupSettings.customEntries.edit');
  await waitFor('backupFixture.visibleModalCount() === 2');
  assert.equal(await call('nestedInputValue', 'settings.backupSettings.customEntries.excludePatterns', 0), '^local$');
  assert.equal(await call('nestedInputValue', 'settings.backupSettings.customEntries.excludePatterns', 1), '(^|/)\\.git$');
  await call('nestedSelectOption', 'settings.backupSettings.customEntries.type', 'settings.backupSettings.customEntries.file');
  await waitFor('!document.querySelector(".ant-form-item-explain-error")');
  await call('nestedClickButton', 'common.save');
  await waitFor('backupFixture.visibleModalCount() === 1');
  assert.ok(
    !await evaluate('document.body.textContent.includes("^local$")'),
    'switching to the file type drops the exclude rules from the entry',
  );
  record('editing pre-fills exclude rules and a file switch drops them', true);

  await navigate({ theme: 'light' });
  await evaluate('backupFixture.state.files=[]');
  await call('open', 'remote');
  await waitFor('!!document.querySelector(".ant-empty")');
  await assertFade('empty remote list', false);
  await screenshot('empty-remote');
  return checks;
}
