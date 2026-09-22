import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const readSource = (relativePath: string) =>
  readFileSync(new URL(relativePath, import.meta.url), 'utf8');

/** Slice between two markers, failing loudly when either one moved. */
const sliceBetween = (source: string, startMarker: string, endMarker: string) => {
  const start = source.indexOf(startMarker);
  const end = source.indexOf(endMarker, start + startMarker.length);
  assert.ok(
    start >= 0 && end > start,
    `expected both "${startMarker}" and "${endMarker}" to exist in order`,
  );
  return source.slice(start, end);
};

/** Body of a top-level rule. Line-anchored so nested/compound selectors miss. */
const ruleBlock = (source: string, selector: string) => {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const match = new RegExp(`^${escaped}\\s*\\{`, 'm').exec(source);
  assert.ok(match, `expected to find the "${selector}" rule`);
  const open = match.index + match[0].length - 1;
  const close = source.indexOf('\n}', open);
  assert.ok(close > open, `expected "${selector}" to open a block`);
  return source.slice(open, close);
};

const appCss = readSource('../../../App.css');
const skillsToolbar = readSource(
  '../../../features/coding/skills/pages/SkillsPage.module.less',
);
const mcpToolbar = readSource('../../../features/coding/mcp/pages/McpPage.module.less');
const sessionToolbar = readSource(
  '../../../features/coding/shared/sessionManager/SessionManagerPanel.module.less',
);
const gatewayPage = readSource('../../../features/coding/gateway/pages/GatewayPage.tsx');
const gatewayHeader = readSource(
  '../../../features/coding/gateway/pages/GatewayPage.module.less',
);
const gatewayStats = readSource(
  '../../../features/coding/gateway/components/GatewayStatisticsView.module.less',
);
const gatewayRequests = readSource(
  '../../../features/coding/gateway/components/GatewayRequestsView.module.less',
);
const imageHeader = readSource(
  '../../../features/coding/image/pages/ImagePage.module.less',
);

const collapseBlock = sliceBetween(
  appCss,
  '/* Global Collapse card style',
  '/* ProLayout dark mode support',
);

test('page-level collapse clips with `clip` so sticky headers keep rounded corners', () => {
  const card = ruleBlock(collapseBlock, '.ant-collapse');
  // `hidden` makes the card a scroll container and the sticky headers pin to
  // a box that never scrolls; `visible` loses the rounded clipping that the
  // single-item cards rely on (antd's `:last-child` radius squares off the
  // header's top corners). `clip` clips without scrolling.
  assert.match(card, /overflow:\s*clip;/);
  const declared = card.match(/overflow:\s*\w+;/g) ?? [];
  assert.match(declared[declared.length - 1] ?? '', /clip/);
  // Scan each rule body on its own so the modal/drawer overrides (separate
  // selectors, which must keep clipping) cannot mask a re-clipping card rule.
  assert.doesNotMatch(
    collapseBlock.replace(/\/\*[\s\S]*?\*\//g, ''),
    /^\.ant-collapse\s*\{[^}]*overflow:\s*hidden;/m,
  );
});

test('page-level collapse headers freeze at the content box, not under a second header offset', () => {
  // `main` already pads by `--content-top-offset`. Sticky top is relative to
  // that padded content box; repeating the offset leaves a blank band.
  assert.match(
    collapseBlock,
    /\.ant-collapse > \.ant-collapse-item > \.ant-collapse-header\.ant-collapse-header \{[\s\S]*?position:\s*sticky;[\s\S]*?top:\s*0;/,
  );
  assert.doesNotMatch(
    collapseBlock,
    /\.ant-collapse > \.ant-collapse-item > \.ant-collapse-header\.ant-collapse-header \{[\s\S]*?top:\s*var\(--content-top-offset/,
  );
  assert.doesNotMatch(
    collapseBlock,
    /\.ant-collapse > \.ant-collapse-item > \.ant-collapse-header\.ant-collapse-header \{[\s\S]*?position:\s*fixed;/,
  );
});

test('header rules outrank antd runtime styles by specificity, not by `!important`', () => {
  // antd injects `.ant-collapse > .ant-collapse-item > .ant-collapse-header
  // { position: relative }` at runtime, i.e. later than this stylesheet, so
  // an equal-specificity `position: sticky` loses and nothing freezes. The
  // repeated class is what outranks it.
  assert.match(
    collapseBlock,
    /\.ant-collapse > \.ant-collapse-item > \.ant-collapse-header\.ant-collapse-header/,
  );
  assert.doesNotMatch(collapseBlock, /position:\s*sticky\s*!important/);
});

test('header corners match the card radius instead of antd`s single-item `:last-child`', () => {
  // A single-item card is antd's `:first-child` *and* `:last-child`; the
  // later bottom-only shorthand wins, leaving the header's top corners square
  // over the card's radius wherever the card's `overflow: clip` is absent.
  assert.match(
    collapseBlock,
    /\.ant-collapse > \.ant-collapse-item:first-child > \.ant-collapse-header\.ant-collapse-header \{[\s\S]*?border-top-left-radius:\s*12px;[\s\S]*?border-top-right-radius:\s*12px;/,
  );
  assert.match(
    collapseBlock,
    /\.ant-collapse > \.ant-collapse-item:last-child:not\(\.ant-collapse-item-active\) > \.ant-collapse-header\.ant-collapse-header \{[\s\S]*?border-bottom-left-radius:\s*12px;[\s\S]*?border-bottom-right-radius:\s*12px;/,
  );
});

test('nested, modal and drawer collapse headers unfreeze instead of stacking', () => {
  assert.match(
    collapseBlock,
    /\.ant-collapse \.ant-collapse > \.ant-collapse-item > \.ant-collapse-header\.ant-collapse-header,[\s\S]*?position:\s*static;/,
  );
  assert.match(collapseBlock, /\.ant-modal \.ant-collapse,[\s\S]*?overflow:\s*hidden;/);
});

test('the animating panel keeps antd clip instead of spilling out of the card', () => {
  // antd puts the motion className on the `.ant-collapse-panel` element, so
  // its `.ant-motion-collapse { overflow: hidden }` is what clips the body
  // during the height/opacity transition. The `overflow: visible` above
  // outranks it, so the clip has to be re-asserted for the motion class.
  assert.match(
    collapseBlock,
    /\.ant-collapse-panel\.ant-motion-collapse\b[\s\S]*?\{\s*overflow:\s*hidden;/,
  );
});

test('browse toolbars freeze at the content box and unfreeze with their page', () => {
  for (const source of [skillsToolbar, mcpToolbar, gatewayHeader, imageHeader]) {
    assert.match(source, /position:\s*sticky;/);
    assert.match(source, /top:\s*0;/);
    assert.doesNotMatch(source, /position:\s*fixed;/);
    assert.doesNotMatch(source, /top:\s*var\(--content-top-offset/);
  }

  assert.match(sessionToolbar, /\.toolbar \{[\s\S]*?position:\s*sticky;/);
  assert.match(sessionToolbar, /\.toolbar \{[\s\S]*?top:\s*48px;/);
  assert.doesNotMatch(sessionToolbar, /\.toolbar \{[\s\S]*?top:\s*calc\(var\(--content-top-offset/);
});

test('gateway filter bars follow the measured header height', () => {
  // The header height is content-driven (the controls wrap at narrow widths),
  // so a constant offset drifts into a gap or an overlap.
  assert.match(gatewayPage, /--gateway-header-height/);
  assert.match(gatewayPage, /new ResizeObserver/);

  for (const source of [gatewayStats, gatewayRequests]) {
    const filterBar = ruleBlock(source, '.filterBar');
    assert.match(filterBar, /position:\s*sticky;/);
    assert.match(filterBar, /top:\s*var\(--gateway-header-height,\s*64px\);/);
  }
});

test('session toolbar keeps its flow spacing instead of bleeding to the card edges', () => {
  // The bar sits in the card body, whose background is the same opaque
  // colour, so the freeze needs no bleed: a negative margin would only move
  // the content off its unfrozen position.
  const toolbar = ruleBlock(sessionToolbar, '.toolbar');
  assert.match(toolbar, /position:\s*sticky;/);
  assert.match(toolbar, /margin-bottom:\s*12px;/);
  assert.doesNotMatch(toolbar, /-1[0-9]px/);
});

test('frozen surfaces carry no decoration: sticky, offset, order and an opaque background', () => {
  // Freezing must not restyle the page: no shadow/divider line, no padding
  // that would shift the unfrozen layout. The opaque background is the one
  // style the freeze needs, so scrolled content cannot show through.
  const surfaces: Array<[string, string, string]> = [
    [
      'collapse header',
      collapseBlock,
      '.ant-collapse > .ant-collapse-item > .ant-collapse-header.ant-collapse-header',
    ],
    ['skills toolbar', skillsToolbar, '.toolbar'],
    ['mcp toolbar', mcpToolbar, '.toolbar'],
    ['session toolbar', sessionToolbar, '.toolbar'],
    ['gateway header', gatewayHeader, '.header'],
    ['gateway statistics filter bar', gatewayStats, '.filterBar'],
    ['gateway requests filter bar', gatewayRequests, '.filterBar'],
    ['image header', imageHeader, '.pageHeader'],
  ];
  for (const [label, source, selector] of surfaces) {
    const body = ruleBlock(source, selector);
    assert.match(body, /position:\s*sticky;/, `${label} must freeze`);
    assert.match(
      body,
      /background:\s*var\(--color-bg-(?:layout|container)\);/,
      `${label} needs an opaque background`,
    );
    assert.doesNotMatch(body, /box-shadow/, `${label} must not draw a shadow`);
    assert.doesNotMatch(body, /padding:\s*8px/, `${label} must not add padding`);
  }
});
