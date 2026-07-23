/// <reference types="node" />

import test from 'node:test';
import assert from 'node:assert/strict';

import { getUrlOrigin } from '../../utils/urlOrigin.ts';

test('getUrlOrigin strips path and query from https base urls', () => {
  assert.equal(getUrlOrigin('https://api.example.com/v1/chat'), 'https://api.example.com');
  assert.equal(getUrlOrigin('https://api.example.com/v1?token=1'), 'https://api.example.com');
});

test('getUrlOrigin keeps http scheme and non-default ports', () => {
  assert.equal(getUrlOrigin('http://127.0.0.1:3000/openai'), 'http://127.0.0.1:3000');
  assert.equal(getUrlOrigin('http://localhost:8080'), 'http://localhost:8080');
});

test('getUrlOrigin adds https scheme when missing', () => {
  assert.equal(getUrlOrigin('api.example.com/v1'), 'https://api.example.com');
  assert.equal(getUrlOrigin('127.0.0.1:3000/path'), 'https://127.0.0.1:3000');
});

test('getUrlOrigin trims whitespace', () => {
  assert.equal(getUrlOrigin('  https://api.example.com/v1  '), 'https://api.example.com');
});

test('getUrlOrigin returns null for empty or invalid values', () => {
  assert.equal(getUrlOrigin(''), null);
  assert.equal(getUrlOrigin('   '), null);
  assert.equal(getUrlOrigin(null), null);
  assert.equal(getUrlOrigin(undefined), null);
  assert.equal(getUrlOrigin('not a url'), null);
  assert.equal(getUrlOrigin('ftp://example.com/files'), null);
});
