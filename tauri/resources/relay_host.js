// 2API 插件宿主：在 Bun 里跑 OpenCode（v1）服务端插件，经 stdin/stdout 与
// Rust 侧一行一个 JSON 对话。
//
// ── 协议 ──────────────────────────────────────────────────────
// Rust → 宿主：{id, method, params}，宿主回 {id, result} 或 {id, error}。
// `fetch` 的回复是流式的：先 {id, event:"head", status, headers}，再若干
// {id, event:"chunk", data}（base64），最后 {id, result:null}。
// {method:"abort", params:{id}} 取消一个；{method:"credit", params:{id,n}} 把
// 读走的那 n 字节还给 fetch，让它能继续发——读取方慢，上游就跟着停，而不是
// 让宿主无限缓冲。
// 宿主 → Rust 的主动事件：{event:"log", level, message}、{event:"auth",
// provider, account, auth}（插件存了/删了登录）、{event:"toast", ...}。
//
// ── 凭证归 Rust 所有 ──────────────────────────────────────────
// init 时 Rust 把整份账号表灌进来，宿主只在内存里改；插件写登录时先改本地、
// 再发 {event:"auth"} 让 Rust 落库。不回环走 Rust 是因为 getAuth 在每个请求里
// 都要读，往返一次等于给每次请求加一跳。
//
// ── 为什么写队列只有一条 ──────────────────────────────────────
// 一帧写一半就等于把整个通道写坏，坏的不只是那一个调用。所有输出经同一个
// 队列，一行写完才写下一行。

import { AsyncLocalStorage } from "node:async_hooks"
import fs from "node:fs"
import path from "node:path"
import readline from "node:readline"
import { pathToFileURL } from "node:url"

// ---- 保护 stdout -------------------------------------------------------------

// 插件（或它拉起的程序）往 stdout 写一行，就等于往协议里插一行垃圾。插件的
// 一切输出都改道 stderr，由 Rust 收进日志。
const toErr = (...xs) =>
  process.stderr.write(xs.map((x) => (typeof x === "string" ? x : Bun.inspect(x))).join(" ") + "\n")

const rpcWrite = process.stdout.write.bind(process.stdout)
console.log = console.info = console.debug = console.warn = toErr
process.stdout.write = (chunk, enc, cb) => process.stderr.write(chunk, enc, cb)

// ---- 输出队列 ----------------------------------------------------------------

// 一个有序队列：流能接才发下一行，父进程不读时限制的是 Bun 的缓冲而不是让它
// 无限增长。写失败或流关闭时，仍在排队的每一行都以失败告终，而不是永远挂着。
const outq = [] // {line, settle}
let pumping = false
let outFail = null

function waitDrain() {
  return new Promise((resolve, reject) => {
    const done = () => { clear(); resolve() }
    const bad = (e) => { clear(); reject(e ?? new Error("宿主的标准输出已关闭")) }
    const clear = () => {
      process.stdout.removeListener("drain", done)
      process.stdout.removeListener("error", bad)
      process.stdout.removeListener("close", bad)
    }
    process.stdout.once("drain", done)
    process.stdout.once("error", bad)
    process.stdout.once("close", bad)
  })
}

function pump() {
  if (pumping) return
  pumping = true
  while (outq.length) {
    const item = outq[0]
    if (outFail) {
      outq.shift()
      item.settle(outFail)
      continue
    }
    // write 返回 false 只表示流满了，这一行已经被吃下：等 drain 再写下一行，
    // 绝不能重写。
    const ok = rpcWrite(item.line)
    outq.shift()
    item.settle(null)
    if (!ok) {
      waitDrain().then(
        () => { pumping = false; pump() },
        (e) => { outFail = e; pumping = false; pump() },
      )
      return
    }
  }
  pumping = false
}

function queueLine(line) {
  const item = { line, settle: () => {} }
  const promise = new Promise((resolve) => { item.settle = (err) => resolve(err ?? null) })
  outq.push(item)
  pump()
  return {
    promise,
    cancel: () => {
      const i = outq.indexOf(item)
      if (i >= 0) {
        outq.splice(i, 1)
        item.settle(null)
      }
    },
  }
}

const send = (msg) => queueLine(JSON.stringify(msg) + "\n").promise
const sendCancelable = (msg) => queueLine(typeof msg === "string" ? msg : JSON.stringify(msg) + "\n")

// chunkLine 直接拼出来而不是走 stringify：base64 里没有需要 JSON 转义的字符，
// 而 stringify 要把整块数据再扫一遍，那是流式路径上最贵的一步。
const chunkLine = (id, data) => '{"id":' + JSON.stringify(id) + ',"event":"chunk","data":"' + data + '"}\n'

// ---- 背压闸门 ----------------------------------------------------------------

// 一帧最多 256 KB。每一帧的开销按 FRAME_OVERHEAD 字节记（JSON 外壳 + base64
// 的 1/3 膨胀）。take 返回这一次能放多少字节，-1 表示这个 fetch 已被停止。
const MAX_FRAME = 256 << 10
const FRAME_OVERHEAD = 64

function makeGate(window) {
  let free = window
  let stopped = false
  let wake = null
  const fit = (want) => Math.min(want, free - FRAME_OVERHEAD)
  return {
    take(want) {
      if (stopped) return Promise.resolve(-1)
      const n = fit(want)
      if (n > 0) return Promise.resolve(n)
      return new Promise((resolve) => { wake = (ok) => resolve(ok ? fit(want) : -1) })
    },
    spend(cost) { free -= cost },
    add(n) {
      free += n
      if (wake && free > FRAME_OVERHEAD) {
        const w = wake
        wake = null
        w(true)
      }
    },
    stop() {
      stopped = true
      if (wake) {
        const w = wake
        wake = null
        w(false)
      }
    },
  }
}

// ---- 状态 --------------------------------------------------------------------

const hooks = []        // {spec, target, exportName, exportFile, hooks}
const loaded = []       // {spec, error?, exports: [exportName]}
const exportsSeen = []  // {spec, exportName, file}
const auth = {}         // key → OpenCode auth 形状
const config = { provider: {} }
let chosen = {}         // 用户在宿主侧设置的插件选项：{provider: {key: bool}}
let directory = process.cwd()
let listWait = 12000

const loaders = new Map() // key → 账号级 loader 结果
const inflight = new Map() // RPC id → {state, controller, gate, unconsumedBody}
const pendingSignIns = new Map() // session → {url, instructions, method, callback}

// ---- 账号作用域 --------------------------------------------------------------

// 一个 provider 可以登录多个账号：第一个按 OpenCode 的惯例留在 provider id
// 下，其余的放在 id#<slot>。请求里的一切都跑在它所属账号的作用域内，于是
// 插件的 getAuth 与 client.auth.set 只看见那一个账号。
const scope = new AsyncLocalStorage()

const providerOf = (key) => String(key).split("#")[0]
const accountsOf = (provider) => Object.keys(auth).filter((k) => providerOf(k) === provider)

function keyFor(id) {
  const current = scope.getStore()
  if (current && (current === id || providerOf(current) === id)) return current
  return accountsOf(id)[0] ?? id
}

function accountKey(provider, account) {
  if (!account) return accountsOf(provider)[0] ?? provider
  return providerOf(account) === provider ? account : providerOf(account) + "#" + account
}

function freshKey(provider) {
  if (!accountsOf(provider).includes(provider)) return provider
  for (let n = 2; ; n++) {
    const k = provider + "#" + n
    if (!(k in auth)) return k
  }
}

const inScope = (provider, key, fn) => scope.run(key ?? provider, fn)

function authCopy(key) {
  const v = auth[key]
  return v === undefined ? undefined : JSON.parse(JSON.stringify(v))
}

function forgetAccount(key) {
  loaders.delete(key)
}

function setAuth(key, value) {
  auth[key] = value
  forgetAccount(key)
  send({ event: "auth", provider: providerOf(key), account: key, auth: value })
}

function removeAuth(key) {
  delete auth[key]
  forgetAccount(key)
  send({ event: "auth", provider: providerOf(key), account: key, auth: null })
}

// ---- 插件交给我们的东西 ------------------------------------------------------

const stub = (name) =>
  new Proxy(async () => ({ data: undefined }), {
    get(_, key) {
      if (key === "then") return undefined
      return stub(name + "." + String(key))
    },
    apply() {
      return Promise.resolve({ data: undefined })
    },
  })

function makeClient() {
  const known = {
    auth: {
      // OpenCode 的 SDK 形状：auth.set({path: {id}, body})
      set: async (opts) => {
        const id = opts?.path?.id ?? opts?.id ?? opts?.providerID
        const body = opts?.body ?? opts?.auth
        if (typeof id === "string" && body && typeof body === "object") {
          // OpenCode 原样保留插件给的东西，刷新后的 token 覆盖旧的，不做合并。
          const key = keyFor(id)
          if (JSON.stringify(auth[key]) !== JSON.stringify(body)) setAuth(key, body)
        }
        return { data: true }
      },
      remove: async (opts) => {
        const id = opts?.path?.id ?? opts?.id
        if (typeof id === "string") removeAuth(keyFor(id))
        return { data: true }
      },
    },
    tui: {
      showToast: async (opts) => {
        const b = opts?.body ?? opts ?? {}
        send({ event: "toast", title: b.title ?? "", message: String(b.message ?? ""), variant: b.variant ?? "info" })
        return { data: true }
      },
    },
    app: {
      log: async (opts) => {
        const b = opts?.body ?? opts ?? {}
        send({ event: "log", level: b.level ?? "info", message: `[${b.service ?? "plugin"}] ${b.message ?? ""}` })
        return { data: true }
      },
    },
    config: { get: async () => ({ data: config }) },
  }
  const wrap = (obj, name) =>
    new Proxy(obj, {
      get(target, key) {
        if (key in target) {
          const v = target[key]
          return v && typeof v === "object" ? wrap(v, name + "." + String(key)) : v
        }
        if (key === "then") return undefined
        return stub(name + "." + String(key))
      },
    })
  return wrap(known, "client")
}

// ---- 加载 --------------------------------------------------------------------

const INDEX_FILES = ["index.ts", "index.tsx", "index.js", "index.mjs", "index.cjs"]

function readJSON(file) {
  try {
    return JSON.parse(fs.readFileSync(file, "utf8"))
  } catch {
    return undefined
  }
}

// entries 是一个插件可能加载的文件，按 OpenCode 的找法：package.json 的
// exports["./server"]、main、exports["."]，最后是 index 文件；参数是文件路径
// 时就是那个文件。
function entries(target) {
  const stat = fs.statSync(target, { throwIfNoEntry: false })
  if (!stat) throw new Error(`插件不存在：${target}`)
  if (!stat.isDirectory()) return [target]
  const out = []
  const add = (v) => typeof v === "string" && v.trim() && out.push(path.resolve(target, v.trim()))
  const pick = (x) => (typeof x === "string" ? x : x?.import ?? x?.default)
  const pkg = readJSON(path.join(target, "package.json"))
  if (pkg) {
    const ex = pkg.exports && typeof pkg.exports === "object" && !Array.isArray(pkg.exports) ? pkg.exports : undefined
    if (ex) add(pick(ex["./server"]))
    add(pkg.main)
    if (ex) add(pick(ex["."]))
    if (typeof pkg.exports === "string") add(pkg.exports)
  }
  for (const f of INDEX_FILES) {
    const p = path.join(target, f)
    if (fs.existsSync(p)) out.push(p)
  }
  if (out.length === 0) {
    throw Object.assign(
      new Error(`${pkg?.name || target} 不是 OpenCode 插件：package.json 里没有 main/exports，也没有 index 文件`),
      { plain: true },
    )
  }
  return [...new Set(out)]
}

// servers 是一个模块导出的全部插件函数。
//
// 一个包导出多个插件函数是常态（WorkBuddy 就导出两个站点），只取 default 或
// 首个导出会让一半能力消失。default 若是 v1 形状 {id?, server} 就取 server，
// 否则把每个函数（或 {server} 对象）都收进来，同一个函数只收一次。
function servers(mod) {
  const d = mod.default
  if (d && typeof d === "object" && ("server" in d || "id" in d || "tui" in d)) {
    return typeof d.server === "function" ? [{ fn: d.server, exportName: "default" }] : []
  }
  const seen = new Set()
  const out = []
  for (const [name, v] of Object.entries(mod)) {
    if (seen.has(v)) continue
    seen.add(v)
    if (typeof v === "function") out.push({ fn: v, exportName: name })
    else if (v && typeof v === "object" && typeof v.server === "function") out.push({ fn: v.server, exportName: name })
  }
  return out
}

function plainOf(e) {
  return e?.plain ? String(e.message) : String(e?.stack ?? e)
}

async function loadPlugins(list) {
  const client = makeClient()
  const one = async (p) => {
    try {
      const files = entries(p.target)
      const found = []
      for (const file of files) {
        const mod = await import(pathToFileURL(file).href)
        for (const { fn, exportName } of servers(mod)) found.push({ fn, exportName, file })
      }
      if (!found.length) throw Object.assign(new Error(`${p.spec} 没有导出任何插件函数`), { plain: true })
      for (const { fn, exportName, file } of found) {
        const input = {
          client,
          project: { id: "ai-toolbox", worktree: directory, vcs: undefined, time: { created: Date.now() } },
          directory,
          worktree: directory,
          serverUrl: new URL("http://127.0.0.1:4096"),
          $: Bun.$,
        }
        const got = await fn(input, p.options ?? {})
        if (got && typeof got === "object") hooks.push({ spec: p.spec, target: p.target, exportName, exportFile: file, hooks: got })
        exportsSeen.push({ spec: p.spec, exportName, file })
      }
      loaded.push({ spec: p.spec, exports: found.map((f) => f.exportName) })
    } catch (e) {
      loaded.push({ spec: p.spec, error: plainOf(e) })
      toErr("plugin:", p.spec, e)
    }
  }
  // 各插件并行加载：一个慢或卡死的插件不该拖住其余插件。
  await Promise.all(list.map(one))
  // config 钩子按加载顺序执行，后一个插件的配置覆盖前一个——与 OpenCode 一致。
  for (const h of hooks) {
    if (typeof h.hooks.config !== "function") continue
    try {
      await h.hooks.config(config)
    } catch (e) {
      toErr("config:", h.spec, e)
      send({ event: "log", level: "error", message: `${h.spec}: config: ${plainOf(e)}` })
    }
  }
}

// ---- provider 信息 -----------------------------------------------------------

// authOf 是某个 provider 的 auth 钩子。两个插件申报同一个 id 时，先加载的赢。
function authOf(provider) {
  for (const h of hooks) if (h.hooks.auth?.provider === provider) return h.hooks.auth
  return null
}

// ownerOf 是申报这个 provider 的那个导出。Rust 需要它把 provider 归到正确的
// 插件行上——一个包可以导出多个插件函数，按包名归会张冠李戴。
function ownerOf(provider) {
  for (const h of hooks) {
    if (h.hooks.auth?.provider === provider || h.hooks.provider?.id === provider) {
      return { spec: h.spec, exportName: h.exportName, file: h.exportFile }
    }
  }
  return { spec: "", exportName: "", file: "" }
}

// providerIds 是全部插件申报的 provider id，按加载顺序去重。
function providerIds() {
  const out = []
  for (const h of hooks) {
    const id = h.hooks.auth?.provider
    if (typeof id === "string" && id && !out.includes(id)) out.push(id)
  }
  return out
}

// providers 是每个插件申报的 provider 一行：登录方式、能力、模型。
async function providers({ providers: only } = {}) {
  const rows = []
  for (const id of providerIds()) {
    if (only && !only.includes(id)) continue
    const a = authOf(id)
    const keys = accountsOf(id)
    let info = { id, name: id, npm: "", api: "", models: {}, listError: "" }
    try {
      info = await infoFor(id, keys[0], listWait)
    } catch (e) {
      info = { ...info, listError: plainOf(e) }
    }
    rows.push({
      id,
      name: info.name || id,
      npm: info.npm ?? "",
      api: info.api ?? "",
      models: Object.keys(info.models ?? {}),
      listError: info.listError ?? "",
      methods: methodsOf(a),
      options: optionsOf(a),
      usage: typeof a?.usage === "function",
      checkin: typeof a?.checkin === "function",
      single: a?.single === true,
      signedIn: keys.length > 0,
      account: keys[0] ?? "",
      ...ownerOf(id),
    })
  }
  return rows
}

// infoFor 是 provider 的模型目录：config 钩子申报的模型打底，插件自己的
// provider.models 钩子可以覆盖或追加。
async function infoFor(id, key, limit) {
  const cfg = config.provider?.[id] ?? {}
  const out = {
    id,
    name: cfg.name ?? id,
    npm: cfg.npm ?? "",
    api: cfg.api ?? "",
    models: { ...(cfg.models ?? {}) },
    listError: "",
  }
  for (const h of hooks) {
    const ph = h.hooks.provider
    if (ph?.id !== id || typeof ph.models !== "function") continue
    const asked = key ?? accountsOf(id)[0] ?? id
    const given = JSON.parse(JSON.stringify(out))
    const next = await withTimeout(
      inScope(id, asked, () => ph.models(given, { auth: authCopy(asked) })),
      limit,
      "插件的模型列表",
    )
    out.models = Object.fromEntries(
      Object.entries(next ?? {}).map(([k, m]) => [k, { ...m, id: k, providerID: id }]),
    )
  }
  return out
}

// optionsOf 是插件声明的开关（magpie 的扩展，OpenCode 没有）。插件通过
// input.magpie.option(key) 读它。
function optionsOf(a) {
  const out = []
  if (!Array.isArray(a?.options)) return out
  for (const o of a.options) {
    if (!o || typeof o !== "object" || typeof o.key !== "string" || out.some((x) => x.key === o.key)) continue
    if (!shown(o.label, 60)) continue
    out.push({
      key: o.key,
      label: shown(o.label, 60),
      description: shown(o.description, 300),
      default: o.default === true,
    })
  }
  return out
}

function optionValues(provider) {
  const own = chosen?.[provider] ?? {}
  const out = {}
  for (const o of optionsOf(authOf(provider))) out[o.key] = typeof own[o.key] === "boolean" ? own[o.key] : o.default
  return out
}

// methodsOf 是 auth 钩子给出的登录方式。
function methodsOf(a) {
  return (a?.methods ?? []).map((m) => ({
    type: m.type,
    label: m.label,
    ...(m.type === "api" && m.keyless === true && typeof m.authorize === "function" ? { keyless: true } : {}),
    ...(m.type === "api" && shown(m.placeholder, 200) ? { placeholder: shown(m.placeholder, 200) } : {}),
  }))
}

function shown(v, max) {
  const s = typeof v === "string" ? v.trim() : ""
  return s.length > max ? "" : s
}

// ---- 超时 --------------------------------------------------------------------

class Late extends Error {}

function withTimeout(promise, ms, what) {
  if (!(ms > 0)) return promise
  let timer
  const late = new Promise((_, reject) => {
    timer = setTimeout(() => reject(new Late(`${what}超时（${ms}ms）`)), ms)
  })
  return Promise.race([promise, late]).finally(() => clearTimeout(timer))
}

// ---- 请求出口 ----------------------------------------------------------------

// loaderFor 是一个账号的 loader 结果，按账号缓存。插件没给 apiKey 而登录是
// api 类型时，用登录里的 key 兜底，与 OpenCode 一致。
async function loaderFor(provider, key) {
  if (loaders.has(key)) return loaders.get(key)
  const a = authOf(provider)
  const stored = auth[key]
  let opts = {}
  if (a?.loader && stored) {
    const p = await infoFor(provider, key, listWait)
    opts = (await inScope(provider, key, () => a.loader(async () => authCopy(key), JSON.parse(JSON.stringify(p))))) ?? {}
  }
  if (!opts.apiKey && stored?.type === "api") opts = { ...opts, apiKey: stored.key }
  loaders.set(key, opts)
  return opts
}

// sdkHeaders 是模型所在 npm 包会带上的鉴权头。插件自己的 fetch 通常会覆盖它，
// 只给了 baseURL 的插件则要靠这个。
function sdkHeaders(npm, key) {
  if (!key) return {}
  if (npm === "@ai-sdk/anthropic" || npm === "@ai-sdk/google-vertex/anthropic") return { "x-api-key": key }
  if (npm === "@ai-sdk/google") return { "x-goog-api-key": key }
  return { authorization: `Bearer ${key}` }
}

// bodyOf 把 Rust 传来的 base64 body 还原成插件收到的样子：能解成 UTF-8 文本
// 就给字符串（插件只 reshape 字符串 body），否则给字节。
function bodyOf(b64) {
  if (!b64) return undefined
  const b = Buffer.from(b64, "base64")
  try {
    return new TextDecoder("utf-8", { fatal: true, ignoreBOM: true }).decode(b)
  } catch {
    return b
  }
}

function checkFetch(record) {
  if (record.controller.signal.aborted) throw new DOMException("Aborted", "AbortError")
}

async function doFetch(id, record, params) {
  const key = accountKey(params.provider, params.account)
  return inScope(params.provider, key, () => fetchAs(id, record, key, params))
}

async function fetchAs(id, record, key, params) {
  const { controller: ctl, gate } = record
  const { provider, model, npm, url, method, headers, body, session } = params
  try {
    const o = await loaderFor(provider, key)
    checkFetch(record)

    const h = new Headers()
    for (const [k, v] of Object.entries(sdkHeaders(npm, o.apiKey))) h.set(k, v)
    for (const [k, v] of Object.entries(o.headers ?? {})) h.set(k, String(v))
    for (const [k, v] of Object.entries(headers ?? {})) h.set(k, v)

    // chat.headers：插件给这次请求加的头，插件的优先。
    for (const x of hooks) {
      if (typeof x.hooks["chat.headers"] !== "function") continue
      const out = { headers: {} }
      try {
        await x.hooks["chat.headers"](
          {
            sessionID: session ?? "",
            agent: "build",
            model: { id: model, providerID: provider, api: { id: model, npm } },
            provider: { source: "custom", info: { id: provider, npm, api: o.baseURL ?? "", models: {} }, options: o },
            message: {
              id: "",
              sessionID: session ?? "",
              role: "user",
              time: { created: Date.now() },
              agent: "build",
              model: { providerID: provider, modelID: model },
            },
          },
          out,
        )
      } catch (e) {
        send({ event: "log", level: "error", message: `${x.spec}: chat.headers: ${plainOf(e)}` })
      }
      checkFetch(record)
      for (const [k, v] of Object.entries(out.headers ?? {})) h.set(k, String(v))
    }

    const f = typeof o.fetch === "function" ? o.fetch : fetch
    checkFetch(record)
    const res = await f(url, { method: method ?? "POST", headers: h, body: bodyOf(body), signal: ctl.signal })
    record.unconsumedBody = res.body
    checkFetch(record)
    record.state = "streaming"

    const rh = {}
    res.headers.forEach((v, k) => (rh[k] = v))
    await sendCancelable({ id, event: "head", status: res.status, headers: rh }).promise
    checkFetch(record)

    if (res.body) {
      record.unconsumedBody = undefined
      // abort 会结束这一帧的等待，循环于是离开，而不是停在一个永不满的流上。
      for await (const chunk of res.body) {
        checkFetch(record)
        const bytes = Buffer.isBuffer(chunk)
          ? chunk
          : ArrayBuffer.isView(chunk)
            ? Buffer.from(chunk.buffer, chunk.byteOffset, chunk.byteLength)
            : Buffer.from(chunk)
        for (let off = 0; off < bytes.length; ) {
          // 一帧一帧地给，且不超过读取方 credit 过的额度：读取方停下来，
          // 上游就停在这里。
          const n = await gate.take(Math.min(MAX_FRAME, bytes.length - off))
          checkFetch(record)
          if (n <= 0) break
          const data = bytes.subarray(off, off + n).toString("base64")
          gate.spend(n + FRAME_OVERHEAD)
          await sendCancelable(chunkLine(id, data)).promise
          checkFetch(record)
          off += n
        }
      }
    }
    checkFetch(record)
    record.state = "finishing"
    await sendCancelable({ id, result: null }).promise
  } catch (e) {
    record.state = "finishing"
    await sendCancelable({ id, error: { message: plainOf(e) } }).promise
  }
}

// ---- 登录 --------------------------------------------------------------------

// nextPrompt 是登录方法的下一步输入。方法没声明 prompts 就一次问完。
async function nextPrompt(provider, index, inputs) {
  const m = authOf(provider)?.methods?.[index]
  if (!m) throw new Error(`${provider} 没有第 ${index} 种登录方式`)
  if (typeof m.ask === "function") return (await m.ask(inputs ?? {})) ?? null
  for (const p of m.prompts ?? []) {
    if (p.key in (inputs ?? {})) continue
    if (typeof p.when === "function" && !p.when(inputs ?? {})) continue
    return {
      type: p.type,
      key: p.key,
      message: p.message,
      placeholder: p.placeholder ?? "",
      options:
        p.type === "select" ? p.options.map((o) => ({ label: o.label, value: o.value, hint: o.hint ?? "" })) : undefined,
    }
  }
  return null
}

function validate(provider, index, key, value) {
  const p = (authOf(provider)?.methods?.[index]?.prompts ?? []).find((x) => x.key === key)
  if (!p || typeof p.validate !== "function") return null
  return p.validate(value) ?? null
}

// save 按 OpenCode CLI 的做法保存一次成功的登录。
function save(provider, key, result, inputs, apiKeyValue) {
  const id = result.provider ?? provider
  if (id !== provider) key = freshKey(id)
  if ("refresh" in result) {
    const { type: _t, provider: _p, refresh, access, expires, ...extra } = result
    setAuth(key, { type: "oauth", refresh, access, expires, ...extra })
  } else if ("key" in result || apiKeyValue) {
    const md = { ...(inputs && Object.keys(inputs).length ? inputs : {}), ...(result.metadata ?? {}) }
    setAuth(key, { type: "api", key: result.key ?? apiKeyValue, ...(Object.keys(md).length ? { metadata: md } : {}) })
  }
  return { provider: id, account: key }
}

function signInKey(provider, account) {
  if (authOf(provider)?.single === true) return accountsOf(provider)[0] ?? provider
  return account && account !== "new" && providerOf(account) === provider ? account : freshKey(provider)
}

const runSignIn = (provider, key, fn) => inScope(provider, key, () => fn(authCopy(key)))

async function authorize({ provider, method, inputs = {}, account }) {
  const m = authOf(provider)?.methods?.[method]
  if (typeof m?.authorize !== "function") throw new Error(`${provider} 的登录方式 ${method} 不能直接授权`)
  const key = signInKey(provider, account)
  const result = await runSignIn(provider, key, () => m.authorize(inputs))
  if (result?.url || result?.instructions || typeof result?.callback === "function") {
    // 交互式登录：把回调挂在 session 上，等 Rust 带着 code 回来。
    const session = crypto.randomUUID()
    pendingSignIns.set(session, { method, callback: result.callback ?? null, provider, key })
    return { session, url: result.url ?? "", instructions: result.instructions ?? "", method }
  }
  return save(provider, key, result ?? {}, inputs, result?.key)
}

async function callback({ session, code }) {
  const s = pendingSignIns.get(session)
  if (!s) throw new Error("这个登录会话已经过期")
  pendingSignIns.delete(session)
  if (!s.callback) throw new Error("这个登录会话没有回调")
  const result = await runSignIn(s.provider, s.key, () => s.callback(code ?? ""))
  return save(s.provider, s.key, result ?? {}, {}, result?.key)
}

async function apiKey({ provider, method, inputs = {}, key: apiKeyValue, account }) {
  const m = authOf(provider)?.methods?.[method]
  if (!m) throw new Error(`${provider} 没有第 ${method} 种登录方式`)
  const key = signInKey(provider, account)
  const result =
    typeof m.authorize === "function"
      ? await runSignIn(provider, key, () => m.authorize({ ...inputs, key: apiKeyValue }))
      : { key: apiKeyValue }
  return save(provider, key, result ?? {}, inputs, apiKeyValue)
}

// ---- 用量与签到 --------------------------------------------------------------

async function checkin({ provider, account }) {
  const a = authOf(provider)
  if (typeof a?.checkin !== "function") throw new Error(`${provider} 的插件不支持签到`)
  const key = accountKey(provider, account)
  if (!auth[key]) throw new Error("账号未登录")
  const p = await infoFor(provider, key, listWait)
  const r = (await inScope(provider, key, () => a.checkin(async () => authCopy(key), JSON.parse(JSON.stringify(p))))) ?? {}
  // 六种 outcome 都是终态，只有 failed 会在当天重试。captcha 永不代解。
  const outcomes = ["claimed", "done", "ineligible", "inactive", "captcha", "failed"]
  const num = (v) => (typeof v === "number" && isFinite(v) ? Math.max(0, v) : 0)
  const message = typeof r.message === "string" ? r.message : ""
  const t = typeof r.until === "number" ? r.until : typeof r.until === "string" ? Date.parse(r.until) : NaN
  const until = Number.isFinite(t) && t > 0 ? new Date(t).toISOString() : ""
  if (!outcomes.includes(r.outcome)) {
    const said = r.outcome === undefined || r.outcome === null ? "没有 outcome" : JSON.stringify(r.outcome)
    return {
      outcome: "failed",
      credit: 0,
      streak: 0,
      message: message ? `插件回了 ${said}：${message}` : `插件回了 ${said}`,
      until,
    }
  }
  return { outcome: r.outcome, credit: num(r.credit), streak: Math.round(num(r.streak)), message, until }
}

async function usage({ provider, account }) {
  const a = authOf(provider)
  if (typeof a?.usage !== "function") throw new Error(`${provider} 的插件不报用量`)
  const key = accountKey(provider, account)
  if (!auth[key]) throw new Error("账号未登录")
  const p = await infoFor(provider, key, listWait)
  return (await inScope(provider, key, () => a.usage(async () => authCopy(key), JSON.parse(JSON.stringify(p))))) ?? {}
}

async function models({ provider, account }) {
  return infoFor(provider, accountKey(provider, account), listWait)
}

// listAuth 是某个 provider 当前的全部登录，供 Rust 落库。
async function listAuth({ provider }) {
  const out = {}
  for (const key of Object.keys(auth)) {
    if (!provider || providerOf(key) === provider) out[key] = authCopy(key)
  }
  return out
}

// ---- 主循环 ------------------------------------------------------------------

const handlers = {
  async init(p) {
    directory = p.directory ?? directory
    if (p.listMillis > 0) listWait = p.listMillis
    chosen = p.options ?? {}
    // Rust 是凭证的唯一来源：init 时整份灌进来。
    for (const k of Object.keys(auth)) delete auth[k]
    Object.assign(auth, p.auth ?? {})
    await loadPlugins(p.plugins ?? [])
    return { plugins: loaded, exports: exportsSeen, providers: await providers() }
  },
  providers,
  models,
  load: async ({ provider, account }) => loaderFor(provider, accountKey(provider, account)),
  prompt: async (p) => ({ prompt: await nextPrompt(p.provider, p.method, p.inputs ?? {}) }),
  validate: (p) => ({ error: validate(p.provider, p.method, p.key, p.value) }),
  authorize,
  callback,
  apiKey,
  checkin,
  usage,
  listAuth,
  // setAuth 让 Rust 把一个账号的凭证推到宿主（用户改了 key、在别处登录等）。
  setAuth: (p) => {
    if (typeof p.key !== "string") throw new Error("setAuth 需要 key")
    if (p.auth === null) removeAuth(p.key)
    else setAuth(p.key, p.auth)
    return null
  },
  setProviderOptions(p) {
    chosen = p?.options ?? {}
    return null
  },
  // options 是插件声明的开关当前的值，供 UI 回显。
  providerOptions: (p) => optionValues(p.provider),
}

let pending = 0
let closed = false
const done = () => {
  if (--pending === 0 && closed) process.exit(0)
}

// ready 在 init 之前一直悬着：Rust 保证 init 是第一条消息，其余方法都等它。
let readyResolve
const ready = new Promise((resolve) => { readyResolve = resolve })

const rl = readline.createInterface({ input: process.stdin, terminal: false })
rl.on("line", (line) => {
  if (!line.trim()) return
  let msg
  try {
    msg = JSON.parse(line)
  } catch {
    return
  }
  if (msg.method === "abort") {
    const record = inflight.get(msg.params?.id)
    if (record) {
      record.state = "cancelled"
      record.gate.stop()
      record.controller.abort()
    }
    return
  }
  if (msg.method === "credit") {
    const record = inflight.get(msg.params?.id)
    if (record?.state === "streaming") record.gate.add(msg.params?.n ?? 0)
    return
  }
  pending++
  if (msg.method === "init") {
    handlers.init(msg.params ?? {}).then(
      (result) => {
        readyResolve()
        return send({ id: msg.id, result })
      },
      (e) => {
        readyResolve()
        return send({ id: msg.id, error: { message: plainOf(e) } })
      },
    ).finally(done)
    return
  }
  if (msg.method === "fetch") {
    const id = msg.id
    // fetch 在读到消息的这一刻就被接管，早于下面任何 await：在碰到上游之前
    // 到达的 abort 同样能停下它，等在 await 里到达的也不会漏掉。
    const record = {
      state: "setup",
      controller: new AbortController(),
      gate: makeGate(msg.params?.window > 0 ? msg.params.window : 512 << 10),
      unconsumedBody: undefined,
    }
    inflight.set(id, record)
    ready
      .then(() => {
        checkFetch(record)
        return doFetch(id, record, msg.params ?? {})
      })
      .catch((e) => {
        record.state = "finishing"
        return sendCancelable({ id, error: { message: plainOf(e) } }).promise
      })
      .finally(() => {
        record.state = "finished"
        record.unconsumedBody = undefined
        inflight.delete(id)
        done()
      })
    return
  }
  const fn = handlers[msg.method]
  if (!fn) {
    send({ id: msg.id, error: { message: `没有方法 ${msg.method}` } })
    done()
    return
  }
  ready
    .then(() => fn(msg.params ?? {}))
    .then(
      (result) => send({ id: msg.id, result: result ?? null }),
      (e) => send({ id: msg.id, error: { message: plainOf(e) } }),
    )
    .finally(done)
})
rl.on("close", () => {
  closed = true
  if (pending === 0) process.exit(0)
})
