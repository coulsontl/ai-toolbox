import assert from 'node:assert/strict';
import test from 'node:test';

import { filterModelPricing } from '../../../../../features/coding/gateway/utils/modelPricing.ts';
import type { ModelPricing } from '../../../../../services/proxyGatewayApi.ts';

const pricingList: ModelPricing[] = [
  {
    model_id: 'claude-opus-4-7',
    display_name: 'Claude Opus 4.7',
    input_cost_per_million: '5',
    output_cost_per_million: '25',
    cache_read_cost_per_million: '0.50',
    cache_creation_cost_per_million: '6.25',
  },
  {
    model_id: 'gpt-5.2-codex',
    display_name: 'GPT-5.2 Codex',
    input_cost_per_million: '1.25',
    output_cost_per_million: '10',
    cache_read_cost_per_million: '0.125',
    cache_creation_cost_per_million: '0',
  },
  {
    model_id: 'gemini-3-pro',
    display_name: 'Gemini 3 Pro',
    input_cost_per_million: '2',
    output_cost_per_million: '12',
    cache_read_cost_per_million: '0.5',
    cache_creation_cost_per_million: '0',
  },
];

const ids = (keyword: string) => filterModelPricing(pricingList, keyword).map((pricing) => pricing.model_id);

test('matches model ids and display names case-insensitively', () => {
  assert.deepEqual(ids('OPUS'), ['claude-opus-4-7']);
  assert.deepEqual(ids('codex'), ['gpt-5.2-codex']);
  assert.deepEqual(ids('gemini 3'), ['gemini-3-pro']);
});

test('trims the keyword before matching', () => {
  assert.deepEqual(ids('  opus  '), ['claude-opus-4-7']);
});

test('an empty or whitespace keyword returns the original list', () => {
  assert.equal(filterModelPricing(pricingList, ''), pricingList);
  assert.equal(filterModelPricing(pricingList, '   '), pricingList);
});

test('unmatched keywords return an empty list', () => {
  assert.deepEqual(ids('llama'), []);
});

test('a row with an empty display name still matches on its id', () => {
  const unnamed: ModelPricing = { ...pricingList[0], model_id: 'custom-unnamed', display_name: '' };
  assert.deepEqual(filterModelPricing([unnamed], 'custom'), [unnamed]);
});

// Price strings are digits like "25" or "0.50"; searching them would make a
// query match rows that have nothing to do with the model name. "25" appears
// only in claude-opus-4-7's output price, "0.50" only in its cache-read price.
test('price fields are not searchable', () => {
  assert.deepEqual(ids('25'), []);
  assert.deepEqual(ids('0.50'), []);
});
