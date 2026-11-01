import assert from 'node:assert/strict';
import test from 'node:test';

import {
  canOfferModelRemoval,
  selectRemovableModelIds,
} from '../../../../../features/coding/shared/providerConnectivity/removableModels.ts';

test('no removable list means no removable models, even when every probe failed', () => {
  const failed = ['config-only-default', 'catalog-row', 'official-readonly'];

  assert.deepEqual(selectRemovableModelIds(failed, undefined), []);
  assert.deepEqual(selectRemovableModelIds(failed, []), []);
});

test('removable list filters failures down to the caller-owned ids', () => {
  const failed = ['config-only-default', 'catalog-row', 'official-readonly'];

  assert.deepEqual(selectRemovableModelIds(failed, ['catalog-row']), ['catalog-row']);
});

test('removable list never invents removals for models that passed', () => {
  assert.deepEqual(selectRemovableModelIds(['failed-row'], ['passed-row', 'failed-row']), ['failed-row']);
});

test('removal is offered only when a callback and at least one removable id exist', () => {
  const onRemoveModels = async () => {};

  assert.equal(canOfferModelRemoval(onRemoveModels, ['catalog-row']), true);
  assert.equal(canOfferModelRemoval(onRemoveModels, undefined), false);
  assert.equal(canOfferModelRemoval(onRemoveModels, []), false);
  assert.equal(canOfferModelRemoval(undefined, ['catalog-row']), false);
});
