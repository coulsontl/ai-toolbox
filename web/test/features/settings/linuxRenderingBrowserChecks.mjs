import assert from 'node:assert/strict';

export async function verifyLinuxRendering({ send, evaluate, baseUrl }) {
  await send('Page.navigate', { url: baseUrl });
  const waitFor = async expression => {
    for (let attempt = 0; attempt < 100; attempt++) {
      if (await evaluate(expression)) return;
      await new Promise(resolve => setTimeout(resolve, 50));
    }
    throw Error('Timed out waiting for: ' + expression);
  };
  const choosePerformance = async () => {
    await evaluate(`document.querySelector('.ant-select-content').dispatchEvent(new MouseEvent('mousedown', {bubbles:true}))`);
    await waitFor(`!![...document.querySelectorAll('.ant-select-item-option')].find(x=>x.textContent==='性能优先')`);
    await evaluate(`[...document.querySelectorAll('.ant-select-item-option')].find(x=>x.textContent==='性能优先').click()`);
  };
  await waitFor(`!!window.renderingFixture && !!document.querySelector('.ant-select:not(.ant-select-disabled)')`);
  await evaluate('renderingFixture.state.failSave = true');
  await choosePerformance();
  await waitFor(`document.querySelector('[role=alert]')?.textContent.includes('test save failure')`);
  assert.equal(await evaluate('renderingFixture.state.mode'), 'compatible');
  assert.match(await evaluate(`document.querySelector('.ant-select-content').textContent`), /兼容优先/);
  await evaluate('renderingFixture.state.deferSave = true');
  await choosePerformance();
  await waitFor('!!renderingFixture.state.release');
  assert.equal(await evaluate(`!!document.querySelector('.ant-select-disabled')`), true);
  await evaluate('renderingFixture.state.release()');
  await waitFor(`document.querySelector('[role=status]')?.textContent.includes('下次启动生效')`);
  assert.equal(await evaluate('renderingFixture.state.mode'), 'performance');
  assert.equal(await evaluate('renderingFixture.state.active'), 'compatible');
  await evaluate('renderingFixture.mount()');
  await waitFor(`document.querySelector('.ant-select-content')?.textContent==='性能优先'`);
  await evaluate(`[...document.querySelectorAll('button')].find(x=>x.textContent.replaceAll(' ','')===renderingFixture.label('common.restart')).click()`);
  await waitFor(`renderingFixture.state.active==='performance'`);
  await evaluate('renderingFixture.mount()');
  await waitFor(`!!document.querySelector('.ant-select:not(.ant-select-disabled)') && !document.querySelector('[role=status]')`);
  const saves = await evaluate(`renderingFixture.state.calls.filter(x=>x.command==='set_linux_webview_rendering').length`);
  assert.equal(saves, 2);
  await evaluate('renderingFixture.state.environmentOverride = true; renderingFixture.mount()');
  await waitFor(`!!document.querySelector('.ant-select-disabled') && document.body.textContent.includes('环境变量')`);
  await evaluate('renderingFixture.state.environmentOverride = false; renderingFixture.state.failLoad = true; renderingFixture.mount()');
  await waitFor(`document.querySelector('[role=alert]')?.textContent.includes('test load failure')`);
  await evaluate(`[...document.querySelectorAll('button')].find(x=>x.textContent.replaceAll(' ','')===renderingFixture.label('common.retry')).click()`);
  await waitFor(`!!document.querySelector('.ant-select:not(.ant-select-disabled)')`);
  return ['save failure retains persisted choice', 'pending save disables edits', 'saved preference survives remount', 'restart required until next startup', 'environment override disables editing', 'load failure can be retried'];
}
