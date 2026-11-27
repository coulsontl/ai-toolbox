// AI Toolbox's page side of a custom plugin page's messages, compatible with
// Magpie's pagebridge.js. The page server serves this file beside every
// plugin page at `_magpie/bridge.js`; a page loads it with
// <script src="_magpie/bridge.js"></script>.
//
//   await magpie.ready            the host's theme, once it has answered
//   magpie.theme                  { dark, lang, tokens: { bg, fg, … } }
//   magpie.onTheme(fn)            fn(theme) on each change of it
//   await magpie.options.get()    the plugin's own options
//   await magpie.options.set(o)   sets them, and answers with them
//   await magpie.read(name, p)    a read its package.json's
//                                 magpie.permissions declares
//   await magpie.image.copy(png)  puts a picture on the clipboard
//   await magpie.image.save(png, name)
//                                 saves it in Downloads and answers { path }
//   await magpie.storage.get/set/delete/list/clear
//                                 the plugin's private key-value store
//                                 (needs the storage:kv permission)
//   await magpie.files.readText/writeText/readBytes/writeBytes/list/stat/delete
//                                 the plugin's private file tree
//                                 (needs the storage:files permission)
//   await magpie.openPlugin(name) opens the host's plugin sheet for a package
//   magpie.visible                false while the reader is on another page
//   magpie.onVisibility(fn)       fn(visible) as the reader leaves and returns
//   magpie.onAction(fn)           fn({ id, provider | agent: { id, name } })
//   magpie.context                where a panel is ({} for a page)
//   magpie.service                the page's local service, "" when none
//
// A page may never reach the network or the host's API itself: the page
// server's policy allows only the page's own files, and everything that needs
// the host goes through the parent frame, which answers only what the
// package declared. The alias `window.aiToolboxPlugin` mirrors the API for
// future host-specific extensions.
//
// What the page leaves unstyled takes the host's look: the bridge adds a
// stylesheet of no weight (:where()) for the font, colours, headings,
// buttons, fields and tables, and the theme's colours are the page's
// --magpie-<token> variables.
(() => {
  const connected = window.parent !== window;
  // Extract the per-frame nonce passed via URL fragment so an external page
  // navigated into the sandboxed iframe cannot spoof host bridge calls even
  // before the host's `load` event fires.
  let bridgeNonce = "";
  try {
    const rawHash = String(location.hash || "").replace(/^#/, "");
    if (rawHash) {
      const parts = rawHash.split("&");
      const kept = [];
      for (const part of parts) {
        if (part.startsWith("_magpie_nonce=")) {
          bridgeNonce = decodeURIComponent(part.slice("_magpie_nonce=".length));
        } else if (part) {
          kept.push(part);
        }
      }
      if (bridgeNonce) {
        try {
          const clean = kept.length ? "#" + kept.join("&") : "";
          history.replaceState(null, "", location.pathname + location.search + clean);
        } catch {
          /* sandboxed opaque origin may disallow replaceState */
        }
      }
    }
  } catch {
    /* ignore hash parsing failure */
  }
  const waiting = new Map();
  let next = 0;
  const themed = [];
  const watched = [];
  let acting = null;
  const acted = [];

  function report(error) {
    if (!connected) return;
    parent.postMessage(
      { magpie: 1, nonce: bridgeNonce, method: "error", params: { message: String(error?.message || error) } },
      "*"
    );
  }

  function call(method, params, timeoutMs = 15000) {
    if (!connected) {
      return Promise.reject(new Error(`AI Toolbox page bridge is not connected (${method})`));
    }
    const id = ++next;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        if (waiting.delete(id)) {
          reject(new Error(`The AI Toolbox host did not answer ${method}`));
        }
      }, timeoutMs);
      waiting.set(id, { resolve, reject, timer });
      parent.postMessage({ magpie: 1, nonce: bridgeNonce, id, method, params }, "*");
    });
  }

  function defaultTheme() {
    let dark = false;
    try {
      dark = matchMedia("(prefers-color-scheme: dark)").matches;
    } catch {
      /* no media query support */
    }
    const lang = String(document.documentElement.lang || navigator.language || "en").split("-")[0];
    return { dark, lang, tokens: {} };
  }

  function paint(theme) {
    const root = document.documentElement;
    root.dataset.theme = theme.dark ? "dark" : "light";
    // A light frame is light only, so a browser that repaints light pages dark
    // leaves the frame as the host left it.
    root.style.colorScheme = theme.dark ? "dark" : "light only";
    root.lang = theme.lang || "en";
    for (const [key, value] of Object.entries(theme.tokens || {})) {
      root.style.setProperty("--magpie-" + key, value);
    }
  }

  addEventListener("message", (event) => {
    if (event.source !== parent) return;
    const message = event.data;
    if (!message || message.magpie !== 1) return;

    if (message.event === "theme") {
      api.theme = message.data;
      paint(message.data);
      for (const fn of themed) {
        try {
          fn(message.data);
        } catch (error) {
          report(error);
        }
      }
      return;
    }
    if (message.event === "visibility") {
      api.visible = !!message.data?.visible;
      for (const fn of watched) {
        try {
          fn(api.visible);
        } catch (error) {
          report(error);
        }
      }
      return;
    }
    if (message.event === "action") {
      if (acting) act(message.data);
      else acted.push(message.data);
      return;
    }

    const waiter = waiting.get(message.id);
    if (!waiter) return;
    waiting.delete(message.id);
    clearTimeout(waiter.timer);
    if (message.error) waiter.reject(new Error(message.error));
    else waiter.resolve(message.result);
  });

  // What the page's own code did wrong is what breaks it. Two things that
  // reach its error handlers are not that: a browser extension's script, and
  // a read of storage or another frame the sandbox shuts out. Ported from
  // Magpie's pagebridge.js so a sandboxed page is not reported as broken.
  function foreign(error, file) {
    const where = String(file || "") + "\n" + String(error?.stack || "");
    if (/(?:chrome|moz|safari-web)-extension:\/\//i.test(where)) return true;
    const why = String(error?.message);
    if (error?.name === "SecurityError") {
      if (/The operation is insecure/.test(why)) return true;
      if (
        /(?:local|session)Storage|\bstorage\b|cookie/i.test(why) &&
        /sandboxed|allow-same-origin/i.test(why)
      ) {
        return true;
      }
      if (/Blocked a frame (?:with origin|at )/.test(why)) return true;
    }
    return false;
  }

  // A script's error reaches a sandboxed page only as "Script error." (its
  // files are another origin's to it); the host then shows its own "This page
  // stopped working" instead.
  addEventListener("error", (event) => {
    if (/ResizeObserver loop/.test(event.message)) {
      event.preventDefault();
      return;
    }
    if (foreign(event.error, event.filename)) {
      event.preventDefault();
      return;
    }
    report(event.error || (event.message === "Script error." ? "" : event.message));
  });
  addEventListener("unhandledrejection", (event) => {
    if (foreign(event.reason)) {
      event.preventDefault();
      return;
    }
    report(event.reason);
  });

  function act(data) {
    try {
      Promise.resolve(acting(data)).catch(report);
    } catch (error) {
      report(error);
    }
  }

  // A page hands a picture over as a canvas's toBlob (a Blob), an
  // ArrayBuffer/Uint8Array or an already encoded string. The host takes a
  // data URL or plain base64, so blobs and buffers are read as one.
  function imageData(png) {
    if (typeof png === "string") return Promise.resolve(png);
    let blob = png;
    if (png instanceof ArrayBuffer) blob = new Blob([png]);
    else if (ArrayBuffer.isView(png)) {
      blob = new Blob([png.buffer.slice(png.byteOffset, png.byteOffset + png.byteLength)]);
    }
    if (typeof Blob === "undefined" || !(blob instanceof Blob)) {
      return Promise.reject(
        new TypeError("magpie.image expects a Blob, ArrayBuffer, Uint8Array, base64 or data URL")
      );
    }
    return new Promise((resolve, reject) => {
      const reader = new FileReader();
      reader.onload = () => resolve(String(reader.result || ""));
      reader.onerror = () => reject(reader.error || new Error("The picture could not be read"));
      reader.readAsDataURL(blob);
    });
  }

  const api = {
    theme: null,
    ready: null,
    context: {},
    service: "",
    visible: true,
    onTheme(fn) {
      themed.push(fn);
    },
    onVisibility(fn) {
      watched.push(fn);
    },
    onAction(fn) {
      acting = fn;
      for (const data of acted.splice(0)) act(data);
    },
    options: {
      get: () => call("options.get", {}).catch(() => ({})),
      set: (options) => call("options.set", { options }),
    },
    read: (name, params) => call("read", { name, params }),
    openPlugin: (name) => call("plugin.open", { name }),
    image: {
      copy: (png) => imageData(png).then((data) => call("image.copy", { png: data })),
      save: (png, name) =>
        imageData(png).then((data) => call("image.save", { png: data, name })),
    },
    storage: {
      get: (key) => call("storage.get", { key }),
      set: (key, value) => call("storage.set", { key, value }),
      delete: (key) =>
        call("storage.delete", { key }).then((result) => !!result?.deleted),
      list: (prefix) => call("storage.list", { prefix }),
      clear: () => call("storage.clear", {}),
    },
    files: {
      readText: (path) =>
        call("files.readText", { path }).then((result) => result?.content ?? ""),
      writeText: (path, content) => call("files.writeText", { path, content }),
      readBytes: (path) =>
        call("files.readBytes", { path }).then((result) => result?.base64 ?? ""),
      writeBytes: (path, base64) => call("files.writeBytes", { path, content: base64 }),
      list: (path) => call("files.list", { path }),
      stat: (path) => call("files.stat", { path }),
      delete: (path) =>
        call("files.delete", { path }).then((result) => !!result?.deleted),
    },
    error: (error) => report(error),
  };

  api.theme = defaultTheme();
  paint(api.theme);

  // Opened as a standalone document there is no host to greet; fall back to
  // the default theme at once instead of waiting out the timeout.
  api.ready = !connected
    ? Promise.resolve(api.theme)
    : call("hello", undefined, 10000)
        .then(({ context, service, visible, ...theme }) => {
          api.theme = theme;
          if (visible === false) api.visible = false;
          api.context = context || {};
          api.service = "";
          try {
            if (service) api.service = new URL(service, location.href).href;
          } catch {
            /* no service */
          }
          paint(theme);
          return theme;
        })
        .catch(() => api.theme);

  // How tall the page draws, for a panel's frame; a page's frame fills its
  // view, and the host doesn't heed it there.
  let told = -1;
  const size = () => {
    const height = Math.ceil(document.documentElement.getBoundingClientRect().height);
    if (height === told) return;
    told = height;
    if (connected) parent.postMessage({ magpie: 1, method: "size", params: { height } }, "*");
  };
  const watch = () => {
    try {
      new ResizeObserver(() => size()).observe(document.documentElement);
    } catch {
      /* no ResizeObserver: the host simply doesn't learn the height */
    }
    size();
  };
  if (document.readyState === "loading") addEventListener("DOMContentLoaded", watch);
  else watch();

  // The host's look for what the page leaves unstyled: its font, colours,
  // headings, buttons, fields and tables. Every rule is :where(), of no
  // weight, so any rule of the page's wins.
  const base = document.createElement("style");
  base.dataset.magpie = "base";
  base.textContent = `
:where(html) { background: var(--magpie-bg, Canvas); color: var(--magpie-fg, CanvasText); font: 13px/1.45 var(--magpie-font, -apple-system, BlinkMacSystemFont, "Segoe UI", system-ui, sans-serif); -webkit-font-smoothing: antialiased; }
:where(body) { margin: 0; padding: 20px 24px; }
:where(h1, h2, h3, h4) { font-weight: 600; margin: 0 0 8px; letter-spacing: -.01em; }
:where(h1) { font-size: 20px; } :where(h2) { font-size: 15px; } :where(h3, h4) { font-size: 13px; }
:where(p) { margin: 0 0 8px; }
:where(small, .muted) { color: var(--magpie-muted, GrayText); }
:where(a) { color: var(--magpie-accent, LinkText); text-decoration: none; }
:where(a:hover) { text-decoration: underline; }
:where(button, input, textarea, select) { font: inherit; color: inherit; }
:where(button) { background: var(--magpie-card, ButtonFace); border: 1px solid var(--magpie-line, ButtonBorder); border-radius: 6px; padding: 3px 10px; }
:where(button:hover:not(:disabled)) { border-color: var(--magpie-muted, ButtonBorder); }
:where(button[aria-pressed="true"], button[aria-selected="true"]) { background: var(--magpie-accent-soft, Highlight); border-color: transparent; color: var(--magpie-accent, HighlightText); }
:where(button:disabled) { opacity: .5; }
:where(input, textarea, select) { background: var(--magpie-card, Field); border: 1px solid var(--magpie-line, ButtonBorder); border-radius: 6px; padding: 4px 8px; }
:where(:focus-visible) { outline: 2px solid var(--magpie-accent, Highlight); outline-offset: 1px; }
:where(table) { border-collapse: collapse; width: 100%; }
:where(th) { text-align: left; font-weight: 500; font-size: 12px; color: var(--magpie-muted, GrayText); }
:where(th, td) { padding: 6px 8px; border-bottom: 1px solid var(--magpie-line, ButtonBorder); }
:where(code, kbd, pre) { font-family: var(--magpie-code, ui-monospace, Menlo, monospace); font-size: 12px; }
:where(pre) { background: var(--magpie-card, Canvas); border: 1px solid var(--magpie-line, ButtonBorder); border-radius: 8px; padding: 8px 10px; overflow: auto; }
:where(hr) { border: 0; border-top: 1px solid var(--magpie-line, ButtonBorder); margin: 12px 0; }
:where(::selection) { background: var(--magpie-accent-soft, Highlight); }
`;
  (document.head || document.documentElement).prepend(base);

  window.magpie = api;
  window.aiToolboxPlugin = api;
})();
