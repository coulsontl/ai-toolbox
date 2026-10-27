import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import ts from 'typescript';

// The pricing search is local: the backend already returns the whole price
// table, so the modal filters the loaded rows as the user types. These tests
// pull the real JSX expressions out of the component and run them, instead of
// matching source text, so a change to the wiring fails here rather than in
// only a string comparison.
const componentUrl = new URL('../../../../../features/coding/gateway/components/ModelPricingModal.tsx', import.meta.url);
const componentSource = readFileSync(componentUrl, 'utf8');
const component = ts.createSourceFile('ModelPricingModal.tsx', componentSource,
  ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);

const attributeText = (node: ts.JsxAttributes, name: string) => {
  const attribute = node.properties.find((property) => ts.isJsxAttribute(property)
    && property.name.getText(component) === name);
  assert.ok(attribute && ts.isJsxAttribute(attribute), `expected a ${name} attribute`);
  return attribute.initializer?.getText(component) ?? '';
};

const expressionText = (node: ts.JsxAttributes, name: string) => {
  const attribute = node.properties.find((property) => ts.isJsxAttribute(property)
    && property.name.getText(component) === name);
  assert.ok(attribute && ts.isJsxAttribute(attribute), `expected a ${name} attribute`);
  const initializer = attribute.initializer;
  assert.ok(initializer && ts.isJsxExpression(initializer) && initializer.expression,
    `expected ${name} to be a JSX expression`);
  return initializer.expression.getText(component);
};

const findJsxElement = (tagName: string, predicate: (node: ts.JsxAttributes) => boolean) => {
  let found: ts.JsxAttributes | null = null;
  const visit = (node: ts.Node) => {
    if ((ts.isJsxSelfClosingElement(node) || ts.isJsxOpeningElement(node))
      && node.tagName.getText(component) === tagName && predicate(node.attributes)) {
      found = node.attributes;
    }
    ts.forEachChild(node, visit);
  };
  visit(component);
  assert.ok(found, `expected a ${tagName} element matching the predicate`);
  return found;
};

const pricingTable = findJsxElement('Table', (attributes) => attributeText(attributes, 'rowKey') === '"model_id"');
const searchInput = findJsxElement('Input', (attributes) => attributeText(attributes, 'className') === '{styles.searchInput}');

test('the pricing table renders the filtered list, not the raw backend list', () => {
  assert.equal(expressionText(pricingTable, 'dataSource'), 'filteredPricingList');
  assert.match(componentSource, /const filteredPricingList = React\.useMemo\(\s*\(\) => filterModelPricing\(pricingList, pricingKeyword\),/);
});

test('typing a keyword updates it and returns to the first page', () => {
  const received: { keyword: string; page: number | null } = { keyword: '', page: null };
  const onChange = expressionText(searchInput, 'onChange');
  runInNewContext(`
    const handler = ${onChange};
    handler({ target: { value: 'claude' } });
  `, {
    setPricingKeyword: (value: string) => { received.keyword = value; },
    setPricingPage: (page: number) => { received.page = page; },
  }, { timeout: 1000 });
  assert.equal(received.keyword, 'claude');
  assert.equal(received.page, 1);
});

test('the search input is controlled by the keyword, not a draft copy', () => {
  assert.equal(expressionText(searchInput, 'value'), 'pricingKeyword');
  assert.match(expressionText(searchInput, 'placeholder'), /gateway\.page\.pricing\.searchPlaceholder/);
});

// The reset must run on `open` alone. The data-loading effect depends on the
// loaders, which depend on `t`, so hanging the reset there would clear the box
// whenever the user switches language.
test('reopening the modal clears the previous search and page', () => {
  assert.match(
    componentSource,
    /React\.useEffect\(\(\) => \{\s*if \(open\) \{\s*setPricingKeyword\(''\);\s*setPricingPage\(1\);\s*\}\s*\}, \[open\]\);/,
  );
});

// A keyword with no matches must explain that nothing matched, while an empty
// price table keeps Ant Design's own empty state instead of claiming a search
// found nothing.
test('only an active keyword replaces the table empty state', () => {
  const locale = expressionText(pricingTable, 'locale');
  assert.match(locale, /pricingKeyword\.trim\(\)/);
  assert.match(locale, /t\('gateway\.page\.pricing\.noSearchResults'\)/);
  assert.match(locale, /:\s*undefined/);
});

// Guard the PR #442 fix while adding the controlled page: the page size must
// stay with Ant Design (`defaultPageSize`), because a fixed `pageSize` silently
// resets the 20/50/100 selection on every render.
const assertPageSizeStaysWithAntDesign = (pagination: string) => {
  assert.match(pagination, /defaultPageSize: 10/);
  assert.doesNotMatch(pagination, /(^|[^a-zA-Z])pageSize:/);
  assert.match(pagination, /current: pricingPage/);
  assert.match(pagination, /onChange: \(page\) => setPricingPage\(page\)/);
};

test('the page size stays with Ant Design while the page is controlled', () => {
  assertPageSizeStaysWithAntDesign(expressionText(pricingTable, 'pagination'));
});

// Negative control: the check above must be able to fail. Both shapes are what
// PR #442 removed or what the original bug looked like.
test('the page-size check rejects a fixed controlled pageSize', () => {
  assert.throws(() => {
    assertPageSizeStaysWithAntDesign(
      "{ current: pricingPage, pageSize: 10, size: 'small', onChange: (page) => setPricingPage(page) }",
    );
  });
  assert.throws(() => assertPageSizeStaysWithAntDesign("{ pageSize: 10, size: 'small' }"));
});
