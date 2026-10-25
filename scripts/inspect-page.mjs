// Inspect the live dev app through the WebView2 DevTools Protocol.
//
// Why this exists: verifying styles by reading source is unreliable (CSS
// Modules hash class names, antd injects its CSS-in-JS at runtime, and the
// cascade decides what actually wins). This tool reads the rendered page:
// computed styles, matched rules with their source file and line, box metrics,
// layout overlaps, CSS variable values, screenshots, and style diffs.
//
// Requires `pnpm dev:debug` (or WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS pointing
// at a port). Default port 9223; override with --port or AI_TOOLBOX_CDP_PORT.
//
// Usage:
//   node scripts/inspect-page.mjs snapshot <selector>       # HTML + computed style + rules + box
//   node scripts/inspect-page.mjs rules <selector>          # matched CSS rules with source locations
//   node scripts/inspect-page.mjs vars <selector>           # resolved CSS custom properties
//   node scripts/inspect-page.mjs box <selector>            # box model + whether it overflows
//   node scripts/inspect-page.mjs overlaps <selector>       # descendants whose boxes collide
//   node scripts/inspect-page.mjs screenshot [selector]     # PNG of the page or one element
//   node scripts/inspect-page.mjs diff <selectorA> <selectorB>  # computed-style differences
//   node scripts/inspect-page.mjs a11y <selector>           # role/name/contrast-relevant colors
//   node scripts/inspect-page.mjs wait                      # block until #root is rendered
//   node scripts/inspect-page.mjs eval "<expression>"       # evaluate arbitrary JS
import { writeFile } from 'node:fs/promises';
import path from 'node:path';

const CDP_TIMEOUT_MS = 30000;

function parseArguments(argv) {
  const options = {};
  const positional = [];
  for (let index = 0; index < argv.length; index += 1) {
    const value = argv[index];
    if (value === '--port') {
      options.port = Number(argv[++index]);
    } else if (value === '--out') {
      options.out = argv[++index];
    } else if (value === '--depth') {
      options.depth = Number(argv[++index]);
    } else {
      positional.push(value);
    }
  }
  return { options, positional };
}

const { options, positional } = parseArguments(process.argv.slice(2));
const command = positional[0];
const port = options.port ?? Number(process.env.AI_TOOLBOX_CDP_PORT ?? 9223);

async function connect() {
  let targets;
  try {
    targets = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
  } catch (error) {
    throw new Error(
      `No CDP endpoint on port ${port}. Start the app with \`pnpm dev:debug\` first. (${error.message})`,
    );
  }
  const target = targets.find((entry) => entry.type === 'page');
  if (!target) throw new Error('CDP endpoint has no page target yet; wait for the window to load.');

  const socket = new WebSocket(target.webSocketDebuggerUrl);
  await new Promise((resolve, reject) => {
    socket.addEventListener('open', resolve, { once: true });
    socket.addEventListener('error', () => reject(new Error('CDP websocket refused')), { once: true });
  });

  let nextId = 0;
  const pending = new Map();
  const consoleMessages = [];
  socket.addEventListener('message', (event) => {
    const payload = JSON.parse(event.data);
    if (payload.id !== undefined) {
      const request = pending.get(payload.id);
      if (!request) return;
      pending.delete(payload.id);
      clearTimeout(request.timer);
      payload.error ? request.reject(new Error(JSON.stringify(payload.error))) : request.resolve(payload.result);
      return;
    }
    if (payload.method === 'Runtime.consoleAPICalled') {
      const text = payload.params.args
        .map((arg) => arg.value ?? arg.description ?? arg.type)
        .join(' ');
      consoleMessages.push(`[${payload.params.type}] ${text}`);
    }
    if (payload.method === 'Runtime.exceptionThrown') {
      const details = payload.params.exceptionDetails;
      consoleMessages.push(`[exception] ${details.exception?.description ?? details.text}`);
    }
  });

  const send = (method, params = {}, timeoutMs = CDP_TIMEOUT_MS) =>
    new Promise((resolve, reject) => {
      const id = ++nextId;
      const timer = setTimeout(() => {
        pending.delete(id);
        reject(new Error(`CDP timeout: ${method}`));
      }, timeoutMs);
      pending.set(id, { resolve, reject, timer });
      socket.send(JSON.stringify({ id, method, params }));
    });

  const evaluate = async (expression) => {
    const result = await send('Runtime.evaluate', {
      expression,
      returnByValue: true,
      awaitPromise: true,
      // Without this a thrown error comes back as an empty object and the real
      // message is lost; the Python notes hit the same wall from the other side.
      userGesture: false,
    });
    if (result.exceptionDetails) {
      throw new Error(result.exceptionDetails.exception?.description ?? result.exceptionDetails.text);
    }
    return result.result.value;
  };

  return { socket, send, evaluate, consoleMessages, targetUrl: target.url };
}

// The DOM/CSS domains are only needed for source attribution. They are NOT
// cheap to enable here: the dev page carries ~1400 stylesheets (antd injects a
// <style> per style request), and `CSS.enable` walks all of them — it timed out
// at 30s. Enable them only for the commands that read matched rules.
async function enableStyleDomains(send) {
  await send('DOM.enable');
  // 120s, not the default 30s: each of the ~1400 sheets is parsed and indexed
  // here, and a cold run on a busy machine regularly needs more than 30.
  await send('CSS.enable', {}, 120000);
}

const pageStateExpression = `(() => {
  const root = document.getElementById('root');
  return JSON.stringify({
    href: location.href,
    ready: document.readyState,
    rootLength: root ? root.innerHTML.length : -1,
    theme: document.documentElement.dataset.theme ?? null,
  });
})()`;

const describeElement = (selector) => `(() => {
  const element = document.querySelector(${JSON.stringify(selector)});
  if (!element) return JSON.stringify({ error: 'not found', selector: ${JSON.stringify(selector)} });
  return JSON.stringify({
    tag: element.tagName.toLowerCase(),
    id: element.id || null,
    className: element.className || null,
    visible: !!(element.offsetWidth || element.offsetHeight || element.getClientRects().length),
    text: (element.innerText ?? '').slice(0, 400),
  });
})()`;

const computedStyleExpression = (selector, properties) => `(() => {
  const element = document.querySelector(${JSON.stringify(selector)});
  if (!element) return JSON.stringify({ error: 'not found' });
  const computed = getComputedStyle(element);
  const wanted = ${JSON.stringify(properties)};
  const values = {};
  for (const property of wanted) values[property] = computed.getPropertyValue(property);
  return JSON.stringify(values);
})()`;

// The properties that most often explain "why does this look wrong". Keeping a
// fixed list makes diffs readable and short; a full dump is 340+ entries of
// noise where every value is "auto" or "normal".
const STYLE_PROPERTIES = [
  'display', 'position', 'top', 'right', 'bottom', 'left', 'z-index', 'float', 'clear',
  'flex-direction', 'flex-wrap', 'flex-grow', 'flex-shrink', 'flex-basis', 'justify-content', 'align-items', 'align-self', 'gap',
  'grid-template-columns', 'grid-template-rows', 'grid-auto-flow',
  'width', 'height', 'min-width', 'min-height', 'max-width', 'max-height',
  'margin-top', 'margin-right', 'margin-bottom', 'margin-left',
  'padding-top', 'padding-right', 'padding-bottom', 'padding-left',
  'border-top-width', 'border-right-width', 'border-bottom-width', 'border-left-width',
  'border-style', 'border-color', 'border-radius',
  'background-color', 'background-image', 'box-shadow', 'opacity',
  'color', 'font-family', 'font-size', 'font-weight', 'line-height', 'letter-spacing', 'text-align', 'text-overflow', 'white-space',
  'overflow-x', 'overflow-y', 'box-sizing', 'transform', 'transition',
];

const boxModelExpression = (selector) => `(() => {
  const element = document.querySelector(${JSON.stringify(selector)});
  if (!element) return JSON.stringify({ error: 'not found' });
  const rect = element.getBoundingClientRect();
  const computed = getComputedStyle(element);
  return JSON.stringify({
    contentBox: { width: element.clientWidth, height: element.clientHeight },
    borderBox: { width: element.offsetWidth, height: element.offsetHeight },
    viewportRect: { x: Math.round(rect.x), y: Math.round(rect.y), width: Math.round(rect.width), height: Math.round(rect.height) },
    scroll: { width: element.scrollWidth, height: element.scrollHeight },
    overflowsHorizontally: element.scrollWidth > element.clientWidth + 1,
    overflowsVertically: element.scrollHeight > element.clientHeight + 1,
    padding: [computed.paddingTop, computed.paddingRight, computed.paddingBottom, computed.paddingLeft],
    margin: [computed.marginTop, computed.marginRight, computed.marginBottom, computed.marginLeft],
    border: [computed.borderTopWidth, computed.borderRightWidth, computed.borderBottomWidth, computed.borderLeftWidth],
    boxSizing: computed.boxSizing,
    viewport: { width: innerWidth, height: innerHeight },
  });
})()`;

// Reporting which rules apply is what the browser's own cascade knows and
// hand-rolled matching does not. Two approaches were tried and rejected:
//   - `element.matches(rule.selectorText)` throws on pseudo-classes and
//     pseudo-elements, and the catch that keeps it alive silently drops exactly
//     the rules worth seeing.
//   - Walking `document.styleSheets` misses nothing structurally, but the dev
//     page holds ~1400 sheets (antd injects one per style request), and the
//     matched ones are indistinguishable from the rest without the engine.
// `CSS.getMatchedStylesForNode` returns the engine's own answer, with `origin`
// separating user-agent rules from authored ones.
//
// Source locations: `rule.styleSheetId` is an opaque engine handle, so the file
// path comes from the owning <style data-vite-dev-id="/abs/path"> node instead —
// Vite writes the original file path there, which is how a hashed CSS Module
// class (`_header_d1am0_38`) maps back to `styles.module.less`.
async function matchedRules(connection, selector) {
  const { evaluate, send } = connection;
  await enableStyleDomains(send);
  const { root } = await send('DOM.getDocument', { depth: -1 });
  const { nodeId } = await send('DOM.querySelector', { nodeId: root.nodeId, selector });
  if (!nodeId) return { error: 'not found', selector };

  const { matchedCSSRules, inlineStyle, inherited } = await send('CSS.getMatchedStylesForNode', { nodeId });

  // Map each engine stylesheet handle to the <style> element that owns it, by
  // describing every dev-id style node once and comparing text with the sheet.
  const devIdBySheet = new Map();
  const styleNodes = await send('DOM.querySelectorAll', {
    nodeId: root.nodeId,
    selector: 'style[data-vite-dev-id]',
  });
  for (const styleNodeId of styleNodes.nodeIds) {
    const described = await send('DOM.describeNode', { nodeId: styleNodeId });
    const attributes = described.node.attributes ?? [];
    const devIdIndex = attributes.indexOf('data-vite-dev-id');
    if (devIdIndex === -1) continue;
    const devId = attributes[devIdIndex + 1];
    // The engine exposes this style element's sheet through its own id; pairing
    // by content is unnecessary once we ask the engine for the node's sheet.
    devIdBySheet.set(described.node.backendNodeId, devId);
  }

  const sheetTextCache = new Map();
  const readSheet = async (styleSheetId) => {
    if (!styleSheetId) return null;
    if (!sheetTextCache.has(styleSheetId)) {
      try {
        const { text } = await send('CSS.getStyleSheetText', { styleSheetId });
        sheetTextCache.set(styleSheetId, text);
      } catch {
        sheetTextCache.set(styleSheetId, null);
      }
    }
    return sheetTextCache.get(styleSheetId);
  };

  // Resolve a sheet handle to a source file. The engine does not link the two,
  // so locate the <style> whose text is identical to the sheet we are reading.
  const stylesheetSources = new Map();
  const resolveSource = async (styleSheetId) => {
    if (stylesheetSources.has(styleSheetId)) return stylesheetSources.get(styleSheetId);
    const sheetText = await readSheet(styleSheetId);
    let resolved = null;
    if (sheetText !== null) {
      for (const [backendNodeId, devId] of devIdBySheet) {
        try {
          const { outerHTML } = await send('DOM.getOuterHTML', { backendNodeId });
          const match = outerHTML.match(/^<style[^>]*>([\s\S]*)<\/style>$/);
          if (match && match[1] === sheetText) {
            resolved = devId;
            break;
          }
        } catch {
          // Node vanished between enumeration and read; skip it.
        }
      }
    }
    stylesheetSources.set(styleSheetId, resolved);
    return resolved;
  };

  const rules = [];
  for (const entry of matchedCSSRules) {
    const rule = entry.rule;
    if (rule.origin === 'user-agent') continue; // browser defaults are never the bug
    const source = await resolveSource(rule.styleSheetId);
    const range = rule.style.range ?? null;
    const properties = (rule.style.cssProperties ?? [])
      .filter((property) => property.name && property.value)
      .map((property) => (property.implicit ? `(inherited) ${property.name}: ${property.value}` : `${property.name}: ${property.value}`));
    const hasOwnDeclarations = properties.length > 0;
    if (!hasOwnDeclarations) continue; // selector matched, declared nothing
    rules.push({
      selector: rule.selectorList.text,
      origin: rule.origin,
      source: source ?? '(unknown sheet)',
      line: range ? range.startLine + 1 : null, // CDP lines are 0-based
      properties,
    });
  }

  const applied = JSON.parse(await evaluate(computedStyleExpression(selector, STYLE_PROPERTIES)));

  const inlineProperties = (inlineStyle?.cssProperties ?? [])
    .filter((property) => property.name && property.value)
    .map((property) => `${property.name}: ${property.value}`);

  return { selector, matchedRuleCount: rules.length, rules, inlineStyle: inlineProperties, inheritedRuleCount: inherited?.length ?? 0, computed: applied };
}

// Custom properties are NOT enumerable through `getComputedStyle().length` /
// `computed[index]` — the spec limits that list to the longhand properties, so a
// loop over it returns zero variables even though `getPropertyValue('--x')`
// works. Names therefore have to be collected from stylesheet text first, then
// resolved against each element. (Tried and rejected: `CSS.registerProperty` is
// a no-op for enumeration, and `document.adoptedStyleSheets` does not include
// Vite's injected <style> nodes.)
const cssVariablesExpression = (selector) => `(() => {
  const element = document.querySelector(${JSON.stringify(selector)});
  if (!element) return JSON.stringify({ error: 'not found' });

  // Every custom property name declared anywhere in the document, including
  // antd's runtime-injected sheets and Vite's CSS Module output.
  // Split the declaration block on ';' and take the part before ':' instead of
  // using a regex: this whole expression is embedded in a template literal, so
  // every backslash has to survive two levels of escaping, and a silently
  // over-escaped pattern returns zero variables with no error at all.
  const declaredNames = new Set();
  const collectFrom = (rule) => {
    const cssText = rule.style ? rule.style.cssText : '';
    for (const declaration of cssText.split(';')) {
      const colon = declaration.indexOf(':');
      if (colon === -1) continue;
      const name = declaration.slice(0, colon).trim();
      if (name.startsWith('--')) declaredNames.add(name);
    }
  };
  for (const sheet of document.styleSheets) {
    let rules; try { rules = sheet.cssRules; } catch { continue; }
    const walk = (list) => {
      for (const rule of list) {
        // Collect first, recurse after: antd's CSS-in-JS emits rules that have
        // BOTH a style block and nested cssRules. Skipping when cssRules exists
        // (the obvious guard) discarded 6927 of 6928 rules and reported zero
        // variables with no error.
        collectFrom(rule);
        if (rule.cssRules) walk(rule.cssRules);
      }
    };
    walk(rules);
  }

  const resolveAll = (target) => {
    const computed = getComputedStyle(target);
    const resolved = {};
    for (const name of declaredNames) {
      const value = computed.getPropertyValue(name).trim();
      if (value) resolved[name] = value;
    }
    return resolved;
  };

  const onElement = resolveAll(element);
  const onRoot = resolveAll(document.documentElement);
  // Only variables whose resolved value differs from :root matter for "why does
  // this one element look different" — report those separately.
  const overridden = {};
  for (const [name, value] of Object.entries(onElement)) {
    if (onRoot[name] !== value) overridden[name] = { onRoot: onRoot[name] ?? null, onElement: value };
  }

  return JSON.stringify({ declaredVariableCount: declaredNames.size, onElement, onRoot, overridden });
})()`;

// Two boxes "collide" when they intersect but neither contains the other — the
// signature of an absolutely-positioned overlay or a negative margin bug.
// Container/child intersections are normal and would drown the report.
const overlapsExpression = (selector) => `(() => {
  const root = document.querySelector(${JSON.stringify(selector)});
  if (!root) return JSON.stringify({ error: 'not found' });
  const nodes = [root, ...root.querySelectorAll('*')];
  const boxes = nodes.map((node) => {
    const rect = node.getBoundingClientRect();
    const style = getComputedStyle(node);
    return {
      node,
      label: node.tagName.toLowerCase() + (node.id ? '#' + node.id : '') + (typeof node.className === 'string' && node.className ? '.' + node.className.trim().split(/\\s+/).slice(0, 2).join('.') : ''),
      rect,
      positioned: style.position !== 'static',
      zIndex: style.zIndex,
      visible: rect.width > 0 && rect.height > 0 && style.visibility !== 'hidden' && style.display !== 'none',
    };
  }).filter((entry) => entry.visible);

  const collides = (a, b) => {
    const overlapWidth = Math.min(a.rect.right, b.rect.right) - Math.max(a.rect.left, b.rect.left);
    const overlapHeight = Math.min(a.rect.bottom, b.rect.bottom) - Math.max(a.rect.top, b.rect.top);
    if (overlapWidth <= 1 || overlapHeight <= 1) return null;
    const aContainsB = a.rect.left <= b.rect.left && a.rect.top <= b.rect.top && a.rect.right >= b.rect.right && a.rect.bottom >= b.rect.bottom;
    const bContainsA = b.rect.left <= a.rect.left && b.rect.top <= a.rect.top && b.rect.right >= a.rect.right && b.rect.bottom >= a.rect.bottom;
    if (aContainsB || bContainsA) return null; // nesting, not a collision
    if (a.node.contains(b.node) || b.node.contains(a.node)) return null;
    return { overlapWidth: Math.round(overlapWidth), overlapHeight: Math.round(overlapHeight) };
  };

  const findings = [];
  for (let i = 0; i < boxes.length; i += 1) {
    for (let j = i + 1; j < boxes.length; j += 1) {
      const overlap = collides(boxes[i], boxes[j]);
      if (!overlap) continue;
      findings.push({
        a: boxes[i].label,
        b: boxes[j].label,
        positioned: boxes[i].positioned || boxes[j].positioned,
        ...overlap,
      });
    }
  }
  return JSON.stringify({ comparedElements: boxes.length, collisions: findings.slice(0, 40), totalCollisions: findings.length });
})()`;

const a11yExpression = (selector) => `(() => {
  const element = document.querySelector(${JSON.stringify(selector)});
  if (!element) return JSON.stringify({ error: 'not found' });
  const computed = getComputedStyle(element);
  const parseColor = (value) => {
    const match = value.match(/rgba?\\(([^)]+)\\)/);
    if (!match) return null;
    const parts = match[1].split(',').map((part) => parseFloat(part.trim()));
    return { r: parts[0], g: parts[1], b: parts[2], a: parts[3] ?? 1 };
  };
  const luminance = (color) => {
    const channel = (value) => {
      const scaled = value / 255;
      return scaled <= 0.03928 ? scaled / 12.92 : Math.pow((scaled + 0.055) / 1.055, 2.4);
    };
    return 0.2126 * channel(color.r) + 0.7152 * channel(color.g) + 0.0722 * channel(color.b);
  };
  // Walk up for the first opaque background: the element itself is usually
  // transparent, and comparing against it would report a meaningless ratio.
  let backgroundNode = element;
  let background = parseColor(getComputedStyle(backgroundNode).backgroundColor);
  while (backgroundNode && (!background || background.a === 0)) {
    backgroundNode = backgroundNode.parentElement;
    if (!backgroundNode) break;
    background = parseColor(getComputedStyle(backgroundNode).backgroundColor);
  }
  if (!background) background = { r: 255, g: 255, b: 255, a: 1 };
  const foreground = parseColor(computed.color);
  let contrastRatio = null;
  if (foreground) {
    const light = luminance(foreground);
    const dark = luminance(background);
    contrastRatio = Math.round(((Math.max(light, dark) + 0.05) / (Math.min(light, dark) + 0.05)) * 100) / 100;
  }
  return JSON.stringify({
    tag: element.tagName.toLowerCase(),
    role: element.getAttribute('role') ?? null,
    ariaLabel: element.getAttribute('aria-label') ?? null,
    text: (element.innerText ?? '').slice(0, 120),
    color: computed.color,
    backgroundColor: getComputedStyle(backgroundNode ?? element).backgroundColor,
    contrastRatio,
    fontSize: computed.fontSize,
    // WCAG AA: 4.5 for body text, 3.0 for large text (>=24px or >=18.66px bold).
    wcagAaPasses: contrastRatio === null ? null : (parseFloat(computed.fontSize) >= 24 ? contrastRatio >= 3 : contrastRatio >= 4.5),
    focusable: element.tabIndex >= 0 || /^(a|button|input|select|textarea)$/i.test(element.tagName),
    tabIndex: element.tabIndex,
    ariaHidden: element.getAttribute('aria-hidden') ?? null,
    pointerEvents: computed.pointerEvents,
  });
})()`;

async function ensureRendered(evaluate) {
  const state = JSON.parse(await evaluate(pageStateExpression));
  if (state.rootLength > 0) return state;
  if (state.href === 'about:blank') {
    // /json/list reports the navigation URL before the document switches, so a
    // reload here strands the page on about:blank with no error at all.
    throw new Error('Page is still about:blank. Wait for the window, then retry (do not Page.reload).');
  }
  throw new Error(`#root is empty (${state.href}). React failed to mount; check console output.`);
}

function formatRules(result) {
  const lines = [];
  for (const rule of result.rules) {
    const location = rule.line ? `${rule.source}:${rule.line}` : rule.source;
    lines.push(`  ${rule.selector}  <-  ${location}`);
    lines.push(`      ${rule.properties.join('; ')}`);
  }
  return lines.join('\n');
}

function formatStyleSection(title, values) {
  const entries = Object.entries(values).filter(([, value]) => value && value !== 'none' && value !== 'normal' && value !== 'auto' && value !== '0px' && value !== 'rgba(0, 0, 0, 0)');
  if (!entries.length) return `${title}: (all defaults)`;
  return `${title}:\n` + entries.map(([property, value]) => `  ${property}: ${value}`).join('\n');
}

const connection = await connect();
try {
  const state = await ensureRendered(connection.evaluate);

  switch (command) {
    case 'wait': {
      console.log(JSON.stringify(state, null, 2));
      break;
    }
    case 'snapshot': {
      const selector = positional[1] ?? '#root';
      console.log(`url: ${state.href}   theme: ${state.theme}`);
      console.log('\n=== element ===');
      console.log(JSON.stringify(JSON.parse(await connection.evaluate(describeElement(selector))), null, 2));
      console.log('\n=== computed style ===');
      const style = await matchedRules(connection, selector);
      console.log(formatStyleSection('applied (non-default only)', style.computed));
      console.log(`\n=== matched rules: ${style.matchedRuleCount} ===`);
      console.log(formatRules(style));
      if (style.inlineStyle.length) console.log(`\n=== inline style ===\n  ${style.inlineStyle.join('\n  ')}`);
      const box = JSON.parse(await connection.evaluate(boxModelExpression(selector)));
      console.log('\n=== box ===');
      console.log(JSON.stringify(box, null, 2));
      const html = await connection.evaluate(`document.querySelector(${JSON.stringify(selector)})?.outerHTML ?? '(not found)'`);
      console.log('\n=== html ===');
      console.log(html.slice(0, 4000));
      break;
    }
    case 'rules': {
      const selector = positional[1] ?? '#root';
      const result = await matchedRules(connection, selector);
      if (result.error) throw new Error(`selector not found: ${selector}`);
      console.log(`${result.matchedRuleCount} rules match ${selector}`);
      console.log(formatRules(result));
      if (result.inlineStyle.length) console.log(`\ninline style:\n  ${result.inlineStyle.join('\n  ')}`);
      break;
    }
    case 'computed': {
      const selector = positional[1] ?? '#root';
      const properties = positional[2] ? positional[2].split(',') : STYLE_PROPERTIES;
      console.log(JSON.stringify(JSON.parse(await connection.evaluate(computedStyleExpression(selector, properties))), null, 2));
      break;
    }
    case 'vars': {
      const selector = positional[1] ?? '#root';
      const result = JSON.parse(await connection.evaluate(cssVariablesExpression(selector)));
      console.log('=== variables on element ===');
      console.log(JSON.stringify(result.onElement, null, 2));
      console.log('\n=== variables on :root ===');
      console.log(JSON.stringify(result.onRoot, null, 2));
      break;
    }
    case 'box': {
      const selector = positional[1] ?? '#root';
      console.log(JSON.stringify(JSON.parse(await connection.evaluate(boxModelExpression(selector))), null, 2));
      break;
    }
    case 'overlaps': {
      const selector = positional[1] ?? 'body';
      const result = JSON.parse(await connection.evaluate(overlapsExpression(selector)));
      console.log(`compared ${result.comparedElements} elements, ${result.totalCollisions} collisions`);
      for (const collision of result.collisions) {
        console.log(`  ${collision.a}  <->  ${collision.b}   ${collision.overlapWidth}x${collision.overlapHeight}px${collision.positioned ? ' (positioned)' : ''}`);
      }
      break;
    }
    case 'a11y': {
      const selector = positional[1] ?? '#root';
      console.log(JSON.stringify(JSON.parse(await connection.evaluate(a11yExpression(selector))), null, 2));
      break;
    }
    case 'diff': {
      const [selectorA, selectorB] = positional.slice(1);
      if (!selectorA || !selectorB) throw new Error('diff needs two selectors');
      const styleA = JSON.parse(await connection.evaluate(computedStyleExpression(selectorA, STYLE_PROPERTIES)));
      const styleB = JSON.parse(await connection.evaluate(computedStyleExpression(selectorB, STYLE_PROPERTIES)));
      const differences = [];
      for (const property of STYLE_PROPERTIES) {
        if (styleA[property] !== styleB[property]) {
          differences.push(`${property}: ${styleA[property]}  ->  ${styleB[property]}`);
        }
      }
      console.log(`${differences.length} computed differences between ${selectorA} and ${selectorB}`);
      differences.forEach((line) => console.log('  ' + line));
      break;
    }
    case 'screenshot': {
      const selector = positional[1];
      let clip;
      if (selector) {
        const box = JSON.parse(await connection.evaluate(boxModelExpression(selector)));
        if (box.error) throw new Error(`selector not found: ${selector}`);
        clip = { x: box.viewportRect.x, y: box.viewportRect.y, width: box.viewportRect.width, height: box.viewportRect.height, scale: 1 };
      }
      const result = await connection.send('Page.captureScreenshot', {
        format: 'png',
        ...(clip ? { clip, captureBeyondViewport: true } : {}),
      });
      const outputPath = options.out ?? path.join(process.env.TEMP ?? '.', `ai-toolbox-${Date.now()}.png`);
      await writeFile(outputPath, Buffer.from(result.data, 'base64'));
      console.log(outputPath);
      break;
    }
    case 'eval': {
      const expression = positional[1];
      if (!expression) throw new Error('eval needs an expression');
      console.log(JSON.stringify(await connection.evaluate(expression), null, 2));
      break;
    }
    case 'console': {
      // Reload is intentionally NOT done here; it would discard the very
      // errors we came to read if the page is mid-navigation.
      await new Promise((resolve) => setTimeout(resolve, Number(positional[1] ?? 3000)));
      console.log(connection.consoleMessages.join('\n') || '(no console output captured)');
      break;
    }
    default: {
      console.error('Unknown command. See the usage block at the top of this file.');
      process.exitCode = 2;
    }
  }

  if (connection.consoleMessages.length && command !== 'console') {
    const errors = connection.consoleMessages.filter((line) => /^\[(error|exception)\]/.test(line));
    if (errors.length) {
      console.error(`\n(${errors.length} console error(s) during this run; use \`console\` to see all)`);
    }
  }
} finally {
  connection.socket.close();
}
