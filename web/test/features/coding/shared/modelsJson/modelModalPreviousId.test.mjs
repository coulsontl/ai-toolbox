import assert from 'node:assert/strict';
import { test } from 'node:test';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const modalPath = path.resolve(
  here,
  '../../../../../features/coding/shared/modelsJson/ModelsJsonModelModal.tsx',
);

/**
 * Renaming a model must reach the backend.
 *
 * The page parks the edited entry's id on the form as `previousId` and the
 * backend uses it to merge onto the existing row. `Form.validateFields()`
 * collects only *registered* field names, so a `previousId` that is merely set
 * via `setFieldsValue` is dropped on submit — `previous_id` arrives as `null`,
 * the backend takes the "append" path, and an ID edit silently adds a second
 * row while the original keeps its product-owned keys.
 *
 * This test reads the real modal source: the guard is the presence of a
 * `Form.Item` that registers `previousId`. A component render test cannot
 * catch the regression any better and would need a DOM, while the failure mode
 * here is purely "the field is not registered".
 */
test('the model modal registers previousId so a rename survives validation', async () => {
  const source = await readFile(modalPath, 'utf8');

  assert.match(
    source,
    /<Form\.Item\s+name="previousId"/,
    'previousId must be a registered Form.Item, otherwise validateFields() drops it',
  );

  assert.doesNotMatch(
    source,
    /name="previousId"[^/]*rules=/,
    'the hidden field must not carry validation rules of its own',
  );
});
