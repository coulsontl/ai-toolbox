import assert from 'node:assert/strict';
import test from 'node:test';

import { deriveMiniBrowserSiteName } from '../../../../features/mini-browser/utils/miniBrowserSiteName';

// ---- default site name extraction -------------------------------------------

test('mini browser names a new site after the address host', () => {
  assert.equal(
    deriveMiniBrowserSiteName('https://relay.example.com/console'),
    'relay.example.com',
  );
  // A bare host is accepted as well, so the helper does not depend on the caller
  // having normalised the address first.
  assert.equal(deriveMiniBrowserSiteName('relay.example.com/console'), 'relay.example.com');
});

test('mini browser keeps the port in an extracted site name', () => {
  // Two dashboards on one machine must not collapse into the same name.
  assert.equal(deriveMiniBrowserSiteName('http://127.0.0.1:3000/balance'), '127.0.0.1:3000');
  assert.equal(deriveMiniBrowserSiteName('https://relay.example.com:8443/'), 'relay.example.com:8443');
});

test('mini browser extracts the host from a normalised address', () => {
  // `handleSaveSite` normalises first (bare host -> https://host/), and the
  // extracted name must survive that round trip unchanged.
  assert.equal(deriveMiniBrowserSiteName('https://relay.example.com/'), 'relay.example.com');
  assert.equal(
    deriveMiniBrowserSiteName('https://api.example.com/usage?tab=credits'),
    'api.example.com',
  );
});

test('mini browser refuses to name a site after a non-web address', () => {
  assert.equal(deriveMiniBrowserSiteName('javascript:alert(1)'), null);
  assert.equal(deriveMiniBrowserSiteName('file:///C:/Windows/win.ini'), null);
  assert.equal(deriveMiniBrowserSiteName('data:text/html,<h1>x</h1>'), null);
  assert.equal(deriveMiniBrowserSiteName('ms-settings:privacy'), null);
});

test('mini browser returns no name for empty or invalid input', () => {
  assert.equal(deriveMiniBrowserSiteName(''), null);
  assert.equal(deriveMiniBrowserSiteName('   '), null);
  assert.equal(deriveMiniBrowserSiteName('https://ok.example.com/\nheader'), null);
});
