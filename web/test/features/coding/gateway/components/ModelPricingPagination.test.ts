import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { runInNewContext } from 'node:vm';
import ts from 'typescript';

// Exercise the production Table's actual pagination props with the installed
// Ant Design hook, not a copied pagination implementation. Only React's state
// storage is simulated: these are hook-boundary tests, not browser/DOM tests.
const componentUrl = new URL('../../../../../features/coding/gateway/components/ModelPricingModal.tsx', import.meta.url);
const component = ts.createSourceFile('ModelPricingModal.tsx', readFileSync(componentUrl, 'utf8'),
  ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
const paginationExpressions: string[] = [];
function visit(node: ts.Node) {
  if ((ts.isJsxSelfClosingElement(node) || ts.isJsxOpeningElement(node))
    && node.tagName.getText(component) === 'Table') {
    const attributes = node.attributes.properties;
    const modelTable = attributes.some(attribute => ts.isJsxAttribute(attribute)
      && attribute.name.getText(component) === 'rowKey'
      && attribute.initializer && ts.isStringLiteral(attribute.initializer)
      && attribute.initializer.text === 'model_id');
    if (modelTable) {
      const pagination = attributes.find(attribute => ts.isJsxAttribute(attribute)
        && attribute.name.getText(component) === 'pagination');
      assert.ok(pagination && ts.isJsxAttribute(pagination)
        && pagination.initializer && ts.isJsxExpression(pagination.initializer)
        && pagination.initializer.expression, 'real model pricing Table needs pagination props');
      paginationExpressions.push(pagination.initializer.expression.getText(component));
    }
  }
  ts.forEachChild(node, visit);
}
visit(component);
assert.equal(paginationExpressions.length, 1, 'select exactly the production model pricing Table');

type Pagination = {
  current: number;
  pageSize: number;
  total: number;
  size?: string;
  onChange: (page: number, pageSize: number) => void;
};

type PricingState = { page: number };

// The production pagination props are controlled for `current` only: the
// component owns the page number (so a new search starts at page one) while
// Ant Design keeps the page size from `defaultPageSize`. Evaluating the props
// therefore needs the component's page state; the props are re-evaluated on
// every render so `onChange` feeds back into the next one.
const actualProps = (pricingState: PricingState) => () => ({
  ...runInNewContext(`(${paginationExpressions[0]})`, {
    pricingPage: pricingState.page,
    setPricingPage: (page: number) => { pricingState.page = page; },
  }, { timeout: 1000 }),
});
const require = createRequire(import.meta.url);
const hookPath = require.resolve('antd/lib/table/hooks/usePagination.js');
const hookRequire = createRequire(hookPath);
const hookCode = readFileSync(hookPath, 'utf8');

function createPagination(total = 137, options: { props?: () => object; page?: number } = {}) {
  const pricingState: PricingState = { page: options.page ?? 1 };
  const props = options.props ?? actualProps(pricingState);
  let initialized = false;
  let state: unknown;
  let stateCalls = 0;
  const changes: [number, number][] = [];
  const module = { exports: {} as { default: (total: number, onChange: Pagination['onChange'], props: object) => [Pagination, unknown] } };
  runInNewContext(hookCode, {
    module, exports: module.exports,
    require: (name: string) => name === 'react' ? {
      useState(initial: () => unknown) {
        stateCalls += 1;
        if (!initialized) { state = initial(); initialized = true; }
        return [state, (next: unknown) => { state = next; }];
      },
    } : hookRequire(name),
  }, { filename: hookPath, timeout: 1000 });
  const render = (nextTotal = total) => {
    total = nextTotal;
    stateCalls = 0;
    const [pagination] = module.exports.default(total, (page, size) => changes.push([page, size]), props());
    assert.equal(stateCalls, 1, 'fail visibly if the installed hook changes its state contract');
    return pagination;
  };
  return { render, changes, pricingState };
}

test('pricing Table starts at ten rows and preserves the actual total', () => {
  const page = createPagination().render();
  assert.equal(page.current, 1);
  assert.equal(page.pageSize, 10);
  assert.equal(page.total, 137);
  assert.equal(page.size, 'small');
});

test('pricing Table applies a selected twenty-row page size after rerender', () => {
  const table = createPagination();
  table.render().onChange(1, 20);
  const page = table.render();
  assert.equal(page.pageSize, 20);
  assert.equal(page.current, 1);
  assert.equal(page.total, 137);
  assert.deepEqual(table.changes, [[1, 20]]);
});

test('pricing Table keeps a valid current page when changing page size', () => {
  const table = createPagination();
  table.render().onChange(4, 10);
  table.render().onChange(4, 20);
  const page = table.render();
  assert.equal(page.current, 4);
  assert.equal(page.pageSize, 20);
  assert.equal(page.total, 137);
});

test('pricing Table clamps a now-out-of-range page after increasing page size', () => {
  const table = createPagination();
  table.render().onChange(14, 10);
  assert.equal(table.render().current, 14);
  table.render().onChange(14, 50);
  const page = table.render();
  assert.equal(page.pageSize, 50);
  assert.equal(page.current, 3);
  assert.equal(page.total, 137);
});

test('pricing Table retains the selected size across pagination and unchanged rerenders', () => {
  const table = createPagination();
  table.render().onChange(1, 20);
  table.render().onChange(3, 20);
  for (let render = 0; render < 3; render += 1) {
    const page = table.render();
    assert.equal(page.current, 3);
    assert.equal(page.pageSize, 20);
    assert.equal(page.total, 137);
  }
});

test('pricing Table accepts repeated changes in both directions without fixing the total', () => {
  const table = createPagination();
  for (const size of [20, 50, 100, 20, 10]) {
    table.render().onChange(1, size);
    const page = table.render();
    assert.equal(page.pageSize, size);
    assert.equal(page.current, 1);
    assert.equal(page.total, 137);
  }
});

test('pricing Table updates total and clamps current when the data shrinks', () => {
  const table = createPagination();
  table.render().onChange(6, 20);
  const page = table.render(12);
  assert.equal(page.total, 12);
  assert.equal(page.current, 1);
  assert.equal(page.pageSize, 20);
});

test('pricing Table handles empty results and later data without losing the selected size', () => {
  const table = createPagination();
  table.render().onChange(1, 50);
  const empty = table.render(0);
  assert.equal(empty.total, 0);
  assert.equal(empty.current, 1);
  assert.equal(empty.pageSize, 50);
  const loaded = table.render(81);
  assert.equal(loaded.total, 81);
  assert.equal(loaded.current, 1);
  assert.equal(loaded.pageSize, 50);
});

test('hook harness detects a deliberately fixed controlled page size', () => {
  const table = createPagination(137, { props: () => ({ pageSize: 10, size: 'small' }) });
  table.render().onChange(1, 20);
  assert.equal(table.render().pageSize, 10);
  assert.deepEqual(table.changes, [[1, 20]]);
});

// The component keeps `pageSize` with Ant Design but controls `current`, so a
// page number written by the component must reach the next render.
test('a controlled current page survives a rerender', () => {
  const table = createPagination(137, { page: 3 });
  assert.equal(table.render().current, 3);
  assert.equal(table.render().current, 3);
});

// Ordinary pagination writes the page back into the component state through
// the props' own onChange, and the next render reads it back.
test('clicking a page writes it back into the component page state', () => {
  const table = createPagination();
  table.render().onChange(4, 10);
  assert.equal(table.pricingState.page, 4);
  assert.equal(table.render().current, 4);
});

// A search resets the page to one without touching the selected page size.
test('starting a new search keeps the selected page size', () => {
  const table = createPagination();
  table.render().onChange(1, 50);
  table.pricingState.page = 1;
  const page = table.render(20);
  assert.equal(page.current, 1);
  assert.equal(page.pageSize, 50);
  assert.equal(page.total, 20);
});
