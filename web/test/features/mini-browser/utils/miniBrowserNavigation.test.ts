import assert from 'node:assert/strict';
import test from 'node:test';

import {
  MINI_BROWSER_PAGE_PATH,
  isMiniBrowserPath,
} from '../../../../features/mini-browser/utils/miniBrowserNavigation';

test('mini browser page path stays the single route literal', () => {
  assert.equal(MINI_BROWSER_PAGE_PATH, '/mini-browser');
});

test('mini browser path detection matches the page and its sub-paths', () => {
  assert.equal(isMiniBrowserPath('/mini-browser'), true);
  assert.equal(isMiniBrowserPath('/mini-browser/sites'), true);
  // The route guard in MainLayout has to keep excluding lookalike paths.
  assert.equal(isMiniBrowserPath('/mini-browser-settings'), false);
  assert.equal(isMiniBrowserPath('/gateway'), false);
  assert.equal(isMiniBrowserPath('/images'), false);
});
