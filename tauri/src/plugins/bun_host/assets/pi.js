// pi's extensions and packages (pi coding agent, @earendil-works) run as
// magpie's plugins: host.js loads this module when a plugin it is given is
// one, and turns each provider the extensions register into the hooks an
// OpenCode plugin gives.
//
// The extensions are loaded by pi's own loader (discoverAndLoadExtensions),
// so they import pi's packages as in pi, and their providers are composed
// by pi's ModelRuntime, as pi composes them: its sign-in (OAuth or a key),
// its renewal, its models and its requests are pi's. The runtime keeps
// its sign-ins in magpie's plugin-auth.json, each account as host.js keeps
// an OpenCode plugin's (an OAuth one as {type: "oauth", refresh, access,
// expires, …}, a key as {type: "api", key}), read in the scope of the
// request's account. What else an extension registers (commands, tools,
// renderers, event handlers) is for pi's own sessions and is left unused.
//
// A request reaches a pi provider as Anthropic's Messages (the models are
// said to be @ai-sdk/anthropic's): the provider's loader gives a fetch
// that reads it into pi's context, streams it with pi, and answers with
// Anthropic's events.

import { AsyncLocalStorage } from "node:async_hooks"
import fs from "node:fs"
import path from "node:path"
import { pathToFileURL } from "node:url"

const CODING_AGENT = "@earendil-works/pi-coding-agent"
const PI_AI = "@earendil-works/pi-ai"
// what pi's packages were called before @earendil-works
const OLD_SCOPE = "@mariozechner/pi-"

function readJSON(file) {
  try {
    return JSON.parse(fs.readFileSync(file, "utf8"))
  } catch {
    return undefined
  }
}

// isPi is whether the plugin at target is pi's rather than OpenCode's: a
// package with a pi manifest, the pi-package keyword or pi among what it
// depends on, or a file that imports pi.
export function isPi(target) {
  const stat = fs.statSync(target, { throwIfNoEntry: false })
  if (!stat) return false
  if (stat.isDirectory()) {
    const pkg = readJSON(path.join(target, "package.json"))
    if (!pkg) return fs.existsSync(path.join(target, "extensions")) && !fs.existsSync(path.join(target, "index.ts")) && !fs.existsSync(path.join(target, "index.js"))
    if (pkg.pi && typeof pkg.pi === "object") return true
    if (Array.isArray(pkg.keywords) && pkg.keywords.includes("pi-package")) return true
    const deps = { ...pkg.peerDependencies, ...pkg.dependencies }
    return Object.keys(deps).some((d) => d === CODING_AGENT || d === OLD_SCOPE + "coding-agent")
  }
  if (!/\.[cm]?[jt]sx?$/.test(target)) return false
  try {
    const src = fs.readFileSync(target, "utf8")
    return src.includes(`"${CODING_AGENT}"`) || src.includes(`'${CODING_AGENT}'`) || src.includes(`"${OLD_SCOPE}coding-agent"`) || src.includes(`'${OLD_SCOPE}coding-agent'`)
  } catch {
    return false
  }
}

// resolve finds pi's package name from the first of dirs that has it.
function resolve(name, dirs) {
  for (const d of dirs) {
    try {
      return Bun.resolveSync(name, d)
    } catch {}
  }
  throw new Error(`${name} isn't installed: add it as a plugin's dependency, or reinstall the pi plugin`)
}

// ---- sign-ins ------------------------------------------------------------------

// toPi is an account as pi keeps its credential; fromPi the other way,
// over what the account had.
function toPi(a) {
  if (!a || typeof a !== "object") return undefined
  if (a.type === "oauth") return { ...a, type: "oauth" }
  if (a.type === "api" && typeof a.key === "string") return { type: "api_key", key: a.key, ...(a.metadata?.env ? { env: a.metadata.env } : {}) }
  return undefined
}

function fromPi(c, was) {
  if (c.type === "api_key") return { type: "api", key: c.key, ...(c.env ? { metadata: { ...(was?.metadata ?? {}), env: c.env } } : was?.metadata ? { metadata: was.metadata } : {}) }
  const { type: _t, ...rest } = c
  return { type: "oauth", ...rest }
}

// ---- the context pi is asked with ------------------------------------------------

// the signatures Gemini gives its tool calls (thoughtSignature), which
// Anthropic's tool_use has no field for: kept here by the call's id, to be
// given back with the conversation.
const callSigs = new Map()
const SIGS_KEPT = 4000
function keepSig(id, sig) {
  if (!id || !sig) return
  callSigs.delete(id)
  callSigs.set(id, sig)
  if (callSigs.size > SIGS_KEPT) callSigs.delete(callSigs.keys().next().value)
}

const zeroUsage = () => ({ input: 0, output: 0, cacheRead: 0, cacheWrite: 0, totalTokens: 0, cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 } })

async function imageOf(b) {
  const s = b.source ?? {}
  if (s.type === "base64") return { type: "image", data: s.data, mimeType: s.media_type ?? "image/png" }
  if (s.type === "url" && typeof s.url === "string") {
    const m = /^data:([^;,]+);base64,(.*)$/s.exec(s.url)
    if (m) return { type: "image", data: m[2], mimeType: m[1] }
    const r = await fetch(s.url)
    if (!r.ok) throw new Error(`couldn't fetch the image ${s.url}: ${r.status}`)
    return { type: "image", data: Buffer.from(await r.arrayBuffer()).toString("base64"), mimeType: r.headers.get("content-type")?.split(";")[0] || "image/png" }
  }
  return undefined
}

function textOf(b) {
  if (b.type === "text") return b.text ?? ""
  if (b.type === "document") {
    const s = b.source ?? {}
    if (s.type === "text") return s.data ?? ""
    if (s.type === "content" && Array.isArray(s.content)) return s.content.map((x) => x.text ?? "").join("\n")
  }
  return undefined
}

async function contentOf(c) {
  const blocks = typeof c === "string" ? [{ type: "text", text: c }] : Array.isArray(c) ? c : []
  const out = []
  for (const b of blocks) {
    if (b?.type === "image") {
      const im = await imageOf(b)
      if (im) out.push(im)
      continue
    }
    const t = textOf(b ?? {})
    if (t !== undefined) out.push({ type: "text", text: t })
  }
  return out
}

// contextOf is an Anthropic Messages request as pi's context, its
// assistant turns said to be the model's own so their signatures go back.
export async function contextOf(body, model) {
  const now = Date.now()
  const system = typeof body.system === "string" ? body.system : Array.isArray(body.system) ? body.system.map((b) => b?.text ?? "").filter(Boolean).join("\n\n") : ""
  const names = new Map()
  const messages = []
  for (const m of body.messages ?? []) {
    const blocks = typeof m.content === "string" ? [{ type: "text", text: m.content }] : Array.isArray(m.content) ? m.content : []
    if (m.role === "assistant") {
      const content = []
      for (const b of blocks) {
        if (b.type === "text") content.push({ type: "text", text: b.text ?? "" })
        else if (b.type === "thinking") content.push({ type: "thinking", thinking: b.thinking ?? "", ...(b.signature ? { thinkingSignature: b.signature } : {}) })
        else if (b.type === "redacted_thinking") content.push({ type: "thinking", thinking: "", thinkingSignature: b.data, redacted: true })
        else if (b.type === "tool_use") {
          names.set(b.id, b.name)
          const sig = callSigs.get(b.id)
          content.push({ type: "toolCall", id: b.id, name: b.name, arguments: b.input ?? {}, ...(sig ? { thoughtSignature: sig } : {}) })
        }
      }
      if (!content.length) continue
      messages.push({
        role: "assistant",
        content,
        api: model.api,
        provider: model.provider,
        model: model.id,
        usage: zeroUsage(),
        stopReason: content.some((c) => c.type === "toolCall") ? "toolUse" : "stop",
        timestamp: now,
      })
      continue
    }
    let user = []
    const flush = () => {
      if (user.length) messages.push({ role: "user", content: user, timestamp: now })
      user = []
    }
    for (const b of blocks) {
      if (b.type === "tool_result") {
        flush()
        messages.push({
          role: "toolResult",
          toolCallId: b.tool_use_id,
          toolName: names.get(b.tool_use_id) ?? "",
          content: await contentOf(b.content),
          isError: !!b.is_error,
          timestamp: now,
        })
      } else if (b.type === "image") {
        const im = await imageOf(b)
        if (im) user.push(im)
      } else {
        const t = textOf(b)
        if (t !== undefined) user.push({ type: "text", text: t })
      }
    }
    flush()
  }
  const tools = (body.tools ?? [])
    .filter((t) => t && t.name && (t.input_schema || !t.type || t.type === "custom"))
    .map((t) => ({ name: t.name, description: t.description ?? "", parameters: t.input_schema ?? { type: "object", properties: {} } }))
  return { ...(system ? { systemPrompt: system } : {}), messages, ...(tools.length ? { tools } : {}) }
}

// levelOf is how hard the request asks the model to think, as pi's
// levels say it: output_config.effort, else thinking's budget; none when
// thinking is off or the model doesn't think. A model that can't be told
// not to think (pi's thinkingLevelMap has off null: gpt-oss and Claude on
// Antigravity, whose vendor turns a request without a level away) thinks
// as little as it can, as pi asks it.
export function levelOf(body, model, clamp) {
  if (!model.reasoning) return undefined
  const th = body.thinking
  if (!th || th.type === "disabled") {
    if (model.thinkingLevelMap?.off !== null || !clamp) return undefined
    const l = clamp(model, "minimal")
    return l === "off" ? undefined : l
  }
  let level = body.output_config?.effort
  if (level === "minimal" || level === "low" || level === "medium" || level === "high" || level === "xhigh" || level === "max") {
  } else if (th.type === "enabled" && th.budget_tokens > 0) {
    const n = th.budget_tokens
    level = n <= 4096 ? "low" : n <= 12000 ? "medium" : n <= 24000 ? "high" : "xhigh"
  } else {
    level = "high"
  }
  const l = clamp ? clamp(model, level) : level
  return l === "off" ? undefined : l
}

// ---- answering as Anthropic ----------------------------------------------------

const STOP = { stop: "end_turn", length: "max_tokens", toolUse: "tool_use" }

function usageOf(u) {
  return {
    input_tokens: u?.input ?? 0,
    output_tokens: u?.output ?? 0,
    cache_read_input_tokens: u?.cacheRead ?? 0,
    cache_creation_input_tokens: u?.cacheWrite ?? 0,
  }
}

// startUsage is the usage message_start tells, as Anthropic's does: the
// vendor's prompt (input, cache read and write), which Claude Code's
// context meter reads before message_delta comes (#1887), and the output
// pi's start named. pi's start event names the prompt, or, as pi-ai's
// anthropic-messages pushes start before the vendor's message_start is
// read, the message the events after it share does. That message is
// pi's own and goes on being written, so only its prompt is taken from
// it: the output it holds may already be the turn's last.
function startUsage(events) {
  const start = events.find((e) => e.type === "start")?.partial?.usage
  const prompt = events.map((e) => e.partial?.usage ?? e.message?.usage).find((u) => u && (u.input || u.cacheRead || u.cacheWrite))
  return usageOf({ input: prompt?.input, cacheRead: prompt?.cacheRead, cacheWrite: prompt?.cacheWrite, output: start?.output })
}

function sse(type, data) {
  return `event: ${type}\ndata: ${JSON.stringify({ type, ...data })}\n\n`
}

// statusOf is the status a failure is answered with: the vendor's, else
// one its message names, else 502.
function statusOf(res, msg) {
  if (res?.status >= 400) return res.status
  if (/prompt is too long|context (length|window)|too many tokens|maximum context/i.test(msg)) return 400
  const m = /\b(4\d\d|5\d\d)\b/.exec(msg)
  if (m) return Number(m[1])
  if (/not configured|no api key|unauthori[sz]ed|sign in|log ?in again|invalid_grant/i.test(msg)) return 401
  if (/quota|rate.?limit|too many requests|usage limit/i.test(msg)) return 429
  return 502
}

function errorType(status) {
  return (
    { 400: "invalid_request_error", 401: "authentication_error", 403: "permission_error", 404: "not_found_error", 413: "request_too_large", 429: "rate_limit_error", 529: "overloaded_error" }[status] ??
    "api_error"
  )
}

function failure(status, message, headers = {}) {
  return new Response(JSON.stringify({ type: "error", error: { type: errorType(status), message } }), {
    status,
    headers: { "content-type": "application/json", ...headers },
  })
}

// kept are the vendor's headers worth passing on with a failure.
function kept(res) {
  const out = {}
  for (const [k, v] of Object.entries(res?.headers ?? {})) if (/^(retry-after|x-ratelimit-|anthropic-ratelimit-)/i.test(k)) out[k] = String(v)
  return out
}

// emptyOf is whether a message says nothing at all.
function emptyOf(m) {
  return !(m?.content ?? []).some((c) => (c.type === "text" && c.text) || (c.type === "thinking" && (c.thinking || c.thinkingSignature)) || c.type === "toolCall")
}

// said is what a request's own fetches were last answered with, when that
// wasn't a stream: a package that reads only streams (pi-zcode) passes a
// vendor's refusal sent as JSON over in silence, and its answer is empty.
const said = new AsyncLocalStorage()
let watching = false
function watchFetch() {
  if (watching) return
  watching = true
  const was = globalThis.fetch
  globalThis.fetch = Object.assign(function fetch(input, init) {
    const s = said.getStore()
    const p = was(input, init)
    if (!s) return p
    return p.then((res) => {
      if (/json|text\/plain/i.test(res.headers.get("content-type") ?? "") && Number(res.headers.get("content-length") ?? 0) <= 65536 && res.body)
        s.body = res
          .clone()
          .text()
          .catch(() => "")
      return res
    })
  }, was)
}

// whyOf is what a vendor's JSON body said went wrong, its code after it.
async function whyOf(s) {
  if (!s?.body) return ""
  const text = await Promise.race([s.body, new Promise((ok) => setTimeout(() => ok(""), 1000))])
  let v
  try {
    v = JSON.parse(text)
  } catch {
    return ""
  }
  const e = v?.error ?? v
  const msg = typeof e === "string" ? e : e?.message ?? e?.msg ?? v?.msg ?? v?.message ?? v?.detail
  if (typeof msg !== "string" || !msg) return ""
  const code = (typeof e === "object" ? e?.code : undefined) ?? v?.code
  return code === undefined || code === null || code === "" ? msg : `${msg} (${code})`
}

// answer turns pi's events into Anthropic's: an error before anything was
// said as an HTTP failure, else a stream (or, unstreamed, one message).
async function answer(events, { model, stream, res, said }) {
  const it = events[Symbol.asyncIterator]()
  const first = []
  // wait for the first thing said, so a failure can still be a status
  for (;;) {
    const r = await it.next()
    if (r.done) break
    first.push(r.value)
    const t = r.value.type
    if (t === "error") {
      const msg = r.value.error?.errorMessage || "the request failed"
      if (r.value.reason === "aborted") return failure(499, msg)
      const st = statusOf(res(), msg)
      return failure(st, st === 400 && !/prompt is too long/i.test(msg) && /context|too long|too many tokens/i.test(msg) ? "prompt is too long: " + msg : msg, kept(res()))
    }
    if (t !== "start") break
  }
  // nothing said, while the vendor said why in a body the package didn't
  // read (pi-zcode's {"code":1005,"msg":"exceed quota limit"})
  const last = first.at(-1)
  if (last?.type === "done" && emptyOf(last.message)) {
    const why = await whyOf(said)
    if (why) return failure(statusOf(res(), why), why, kept(res()))
  }
  const id = "msg_pi_" + Math.random().toString(36).slice(2, 14)
  async function* all() {
    yield* first
    for (;;) {
      const r = await it.next()
      if (r.done) return
      yield r.value
    }
  }
  if (!stream) {
    const content = []
    let stop = "end_turn"
    let usage = usageOf()
    for await (const e of all()) {
      if (e.type === "error") {
        const msg = e.error?.errorMessage || "the request failed"
        return failure(statusOf(res(), msg), msg, kept(res()))
      }
      if (e.type !== "done") continue
      for (const c of e.message.content ?? []) {
        if (c.type === "text") c.text && content.push({ type: "text", text: c.text })
        else if (c.type === "thinking") {
          if (c.redacted) content.push({ type: "redacted_thinking", data: c.thinkingSignature ?? "" })
          else if (c.thinking || c.thinkingSignature) content.push({ type: "thinking", thinking: c.thinking ?? "", signature: c.thinkingSignature ?? "" })
        } else if (c.type === "toolCall") {
          keepSig(c.id, c.thoughtSignature)
          content.push({ type: "tool_use", id: c.id, name: c.name, input: c.arguments ?? {} })
        }
      }
      stop = STOP[e.reason] ?? "end_turn"
      usage = usageOf(e.message.usage)
    }
    return new Response(JSON.stringify({ id, type: "message", role: "assistant", model, content, stop_reason: stop, stop_sequence: null, usage }), {
      status: 200,
      headers: { "content-type": "application/json" },
    })
  }
  const enc = new TextEncoder()
  const body = new ReadableStream({
    async start(ctl) {
      const put = (type, data) => ctl.enqueue(enc.encode(sse(type, data)))
      put("message_start", { message: { id, type: "message", role: "assistant", model, content: [], stop_reason: null, stop_sequence: null, usage: startUsage(first) } })
      let index = -1
      const open = new Map() // pi's content index → ours
      const begin = (ci, block) => {
        index++
        open.set(ci, index)
        put("content_block_start", { index, content_block: block })
        return index
      }
      const end = (ci) => {
        if (!open.has(ci)) return
        put("content_block_stop", { index: open.get(ci) })
        open.delete(ci)
      }
      try {
        for await (const e of all()) {
          switch (e.type) {
            // a block is begun with what it says first: an empty one (a
            // vendor's text with nothing in it) would be sent back, and
            // Anthropic's Messages turn an empty text block away
            case "text_start":
            case "thinking_start":
              break
            case "text_delta":
              if (!e.delta) break
              if (!open.has(e.contentIndex)) begin(e.contentIndex, { type: "text", text: "" })
              put("content_block_delta", { index: open.get(e.contentIndex), delta: { type: "text_delta", text: e.delta } })
              break
            case "text_end":
              end(e.contentIndex)
              break
            case "thinking_delta":
              if (!e.delta) break
              if (!open.has(e.contentIndex)) begin(e.contentIndex, { type: "thinking", thinking: "" })
              put("content_block_delta", { index: open.get(e.contentIndex), delta: { type: "thinking_delta", thinking: e.delta } })
              break
            case "thinking_end": {
              const c = e.partial?.content?.[e.contentIndex]
              if (c?.redacted) {
                end(e.contentIndex)
                if (c.thinkingSignature) begin(e.contentIndex, { type: "redacted_thinking", data: c.thinkingSignature })
                end(e.contentIndex)
                break
              }
              if (!open.has(e.contentIndex) && c?.thinkingSignature) begin(e.contentIndex, { type: "thinking", thinking: "" })
              if (open.has(e.contentIndex) && c?.thinkingSignature) put("content_block_delta", { index: open.get(e.contentIndex), delta: { type: "signature_delta", signature: c.thinkingSignature } })
              end(e.contentIndex)
              break
            }
            case "toolcall_end": {
              // a call is told whole, as its arguments are only JSON at its end
              const c = e.toolCall
              keepSig(c.id, c.thoughtSignature)
              const i = begin(e.contentIndex, { type: "tool_use", id: c.id, name: c.name, input: {} })
              put("content_block_delta", { index: i, delta: { type: "input_json_delta", partial_json: JSON.stringify(c.arguments ?? {}) } })
              end(e.contentIndex)
              break
            }
            case "done":
              for (const ci of [...open.keys()]) end(ci)
              put("message_delta", { delta: { stop_reason: STOP[e.reason] ?? "end_turn", stop_sequence: null }, usage: usageOf(e.message?.usage) })
              put("message_stop", {})
              break
            case "error": {
              for (const ci of [...open.keys()]) end(ci)
              const msg = e.error?.errorMessage || "the request failed"
              const st = statusOf(res(), msg)
              put("error", { error: { type: errorType(st), message: msg } })
              break
            }
          }
        }
      } catch (e) {
        put("error", { error: { type: "api_error", message: String(e?.message ?? e) } })
      }
      ctl.close()
    },
  })
  return new Response(body, { status: 200, headers: { "content-type": "text/event-stream", "cache-control": "no-cache" } })
}

// ---- loading -------------------------------------------------------------------

// load loads the pi plugins in list ({spec, target}) and gives, for each,
// the hooks of the providers it registers, or why it couldn't load.
// h is what host.js lends: readAuth, changeAuth, keyFor, send, directory.
export async function load(h, list) {
  const home = path.join(h.directory, "pi")
  // pi keeps its settings and caches here, not in the user's ~/.pi. This
  // overrides any PI_CODING_AGENT_DIR from the user's shell: with that one,
  // the auth.json written below would be the user's own pi sign-ins, and
  // magpie would delete every sign-in it doesn't hold itself.
  process.env.PI_CODING_AGENT_DIR = home
  // pi makes it on start; packages write their files into it (pi-antigravity
  // its accounts) and fail with ENOENT when it isn't there
  fs.mkdirSync(process.env.PI_CODING_AGENT_DIR, { recursive: true })
  watchFetch()
  const none = path.join(home, "none") // no project's or user's extensions come along
  const dirs = [...list.map((p) => (fs.statSync(p.target, { throwIfNoEntry: false })?.isDirectory() ? p.target : path.dirname(p.target))), path.join(h.directory, "plugins")]
  let pi, ai
  try {
    const entry = resolve(CODING_AGENT, dirs)
    pi = await import(pathToFileURL(entry).href)
    ai = await import(pathToFileURL(resolve(PI_AI, [path.dirname(entry), ...dirs])).href)
  } catch (e) {
    return list.map((p) => ({ spec: p.spec, error: String(e?.message ?? e) }))
  }

  // pi's runtime, its sign-ins those of the account a call is made for
  const chains = new Map()
  const credentials = {
    async read(id) {
      return toPi(h.readAuth()[h.keyFor(id)])
    },
    async list() {
      return Object.entries(h.readAuth())
        .filter(([k, a]) => !k.includes("#") && toPi(a))
        .map(([k, a]) => ({ providerId: k, type: toPi(a).type }))
    },
    // one change at a time to an account, read afresh
    modify(id, fn) {
      const key = h.keyFor(id)
      const prev = chains.get(key) ?? Promise.resolve()
      const run = prev.catch(() => {}).then(async () => {
        const was = h.readAuth()[key]
        const next = await fn(toPi(was))
        if (next === undefined) return toPi(was)
        await h.changeAuth((all) => {
          all[key] = fromPi(next, all[key])
        })
        h.send({ event: "auth", provider: key.split("#")[0], account: key })
        mirror()
        return next
      })
      chains.set(key, run)
      return run
    },
    async delete(id) {
      const key = h.keyFor(id)
      await h.changeAuth((all) => {
        if (!(key in all)) return false
        delete all[key]
      })
      mirror()
    },
  }
  const mr = await pi.ModelRuntime.create({ credentials, modelsPath: null, allowModelNetwork: false, refreshOnCreate: false })

  // pi's auth.json, kept as pi keeps it: each provider's sign-in (its first
  // account's). Packages read it back themselves rather than be given it
  // (readStoredCredential: pi-zcode its plans, pi-cursor its token), and
  // are told of it as pi tells them, with session_start: each as the host
  // starts, and a provider again once newly signed in (pi-zcode registers
  // its plans' models then).
  const runners = new Map() // provider id → its package's runner
  const mirrored = new Map()
  const authFile = path.join(process.env.PI_CODING_AGENT_DIR, "auth.json")
  let started = false
  function mirror() {
    const all = h.readAuth()
    const fresh = new Set()
    let file
    for (const [id, runner] of runners) {
      const c = toPi(all[id])
      const now = c ? JSON.stringify(c) : ""
      // a session starts for each package as pi starts, signed in or not
      // (pi-provider-kiro keeps the UI its sign-in asks on from it)
      if (!started && runner) fresh.add(runner)
      if (mirrored.get(id) === now) continue
      if (c && !mirrored.get(id) && runner) fresh.add(runner)
      mirrored.set(id, now)
      file ??= readJSON(authFile) ?? {}
      if (c) file[id] = c
      else delete file[id]
    }
    if (file) {
      try {
        const tmp = `${authFile}.${process.pid}.tmp`
        fs.writeFileSync(tmp, JSON.stringify(file, null, 2), { mode: 0o600 })
        fs.renameSync(tmp, authFile)
      } catch (e) {
        h.send({ event: "log", level: "error", message: `pi: auth.json: ${e?.message ?? e}` })
      }
    }
    const reason = started ? "new" : "startup"
    for (const runner of fresh) {
      const before = JSON.stringify([...runners.keys()].map((id) => (mr.getModels(id) ?? []).map((m) => m.id)))
      runner
        .emit({ type: "session_start", reason })
        .catch(() => {})
        .then(() => {
          // magpie asks the models again when they changed
          if (JSON.stringify([...runners.keys()].map((id) => (mr.getModels(id) ?? []).map((m) => m.id))) !== before) h.send({ event: "auth" })
        })
    }
  }
  const hp = { ...h, mirror }

  const out = []
  for (const p of list) {
    try {
      const r = await pi.discoverAndLoadExtensions(await entriesOf(pi, p.target, none), none, none, pi.createEventBus())
      const ids = [
        ...(r.runtime?.pendingProviderRegistrations ?? []).map((x) => x.name),
        ...(r.runtime?.pendingNativeProviderRegistrations ?? []).map((x) => x.provider.id),
      ]
      if (r.errors?.length && !r.extensions?.length) throw new Error(r.errors.map((e) => e.error).join("; "))
      for (const e of r.errors ?? []) h.send({ event: "log", level: "error", message: `${p.spec}: ${e.error}` })
      if (!ids.length && !r.extensions?.length) throw new Error("has no pi extension")
      const runner = bind(pi, r, mr, none, h, p.spec)
      for (const id of ids) runners.set(id, runner)
      const hooks = [...new Set(ids)].map((id) => ({ spec: p.spec, target: p.target, hooks: hooksOf(hp, mr, ai, id) }))
      out.push({ spec: p.spec, hooks })
    } catch (e) {
      out.push({ spec: p.spec, error: String(e?.stack ?? e) })
    }
  }
  mirror()
  started = true
  return out
}

// bind gives a package's extensions to the runtime as pi's session gives
// them: the providers they registered while loading are registered, one
// they register later (pi-zcode its plans, once signed in) at once, and
// their events (session_start) are given pi's context, with no session
// and no tools of its own, and a UI only for a sign-in's questions.
// Without pi's runner the providers are registered as they were loaded,
// and nothing later reaches them.
function bind(pi, r, mr, cwd, h, spec) {
  const flush = () => {
    for (const x of r.runtime?.pendingProviderRegistrations ?? []) mr.registerProvider(x.name, x.config)
    for (const x of r.runtime?.pendingNativeProviderRegistrations ?? []) mr.registerNativeProvider(x.provider)
    return null
  }
  if (!pi.ExtensionRunner || !pi.ModelRegistry || !pi.SessionManager?.inMemory || !r.runtime) return flush()
  const nothing = () => {}
  let runner
  try {
    runner = new pi.ExtensionRunner(r.extensions ?? [], r.runtime, cwd, pi.SessionManager.inMemory(cwd), new pi.ModelRegistry(mr))
    runner.bindCore(
      {
        sendMessage: nothing,
        sendUserMessage: nothing,
        appendEntry: nothing,
        setSessionName: nothing,
        getSessionName: () => undefined,
        setLabel: nothing,
        getActiveTools: () => [],
        getAllTools: () => [],
        getSettings: () => ({}),
        setActiveTools: nothing,
        refreshTools: nothing,
        getCommands: () => [],
        setModel: async () => false,
        getThinkingLevel: () => "off",
        setThinkingLevel: nothing,
      },
      {
        getModel: () => undefined,
        getScopedModels: () => [],
        isIdle: () => true,
        isProjectTrusted: () => false,
        getSignal: () => undefined,
        abort: nothing,
        hasPendingMessages: () => false,
        shutdown: nothing,
        getContextUsage: () => undefined,
        compact: nothing,
        getSystemPrompt: () => "",
        executeTool: async () => {
          throw new Error("magpie runs no tools of pi's")
        },
        getCallableTools: () => [],
      },
      {
        registerProvider: (name, config) => mr.registerProvider(name, config),
        registerNativeProvider: (provider) => mr.registerNativeProvider(provider),
        unregisterProvider: (name) => mr.unregisterProvider(name),
        registerVirtualModel: (d) => mr.registerVirtualModel?.(d),
        unregisterVirtualModel: (provider, id) => mr.unregisterVirtualModel?.(provider, id),
      },
    )
  } catch (e) {
    h.send({ event: "log", level: "error", message: `${spec}: pi's runner: ${e?.message ?? e}` })
    return flush()
  }
  runner.onError((e) => h.send({ event: "log", level: "error", message: `${spec}: ${e.event}: ${e.error}` }))
  try {
    runner.setUIContext?.(uiOf(runner.getUIContext()), "rpc")
  } catch (e) {
    h.send({ event: "log", level: "error", message: `${spec}: pi's UI: ${e?.message ?? e}` })
  }
  return runner
}

// entriesOf is what pi loads of a package: the files its manifest names, a
// directory among them read as pi's package manager reads one, for its
// files (pi-xai-oauth's "./extensions") — pi's loader, given the package,
// would import the directory itself and fail.
async function entriesOf(pi, target, none) {
  if (!fs.statSync(target, { throwIfNoEntry: false })?.isDirectory() || !pi.DefaultPackageManager || !pi.SettingsManager?.inMemory) return [target]
  try {
    const pm = new pi.DefaultPackageManager({ cwd: none, agentDir: none, settingsManager: pi.SettingsManager.inMemory() })
    const r = await pm.resolveExtensionSources([target], { temporary: true })
    const files = (r?.extensions ?? []).filter((e) => e.enabled).map((e) => e.path)
    if (files.length) return files
  } catch {}
  return [target]
}

// ---- a package's own UI ----------------------------------------------------------

// asking is the sign-in a package's UI is shown in: pi's dialogs and its
// components (ctx.ui) are asked as the sign-in's questions, as magpie asks
// them, and outside one they answer as pi's do with no UI.
const asking = new AsyncLocalStorage()

// plain stands for pi's theme and keybindings: a colour or a style gives
// back the text it is given, and no key is bound.
const plain = new Proxy({}, { get: (_, k) => (k === "then" ? undefined : (...a) => [...a].reverse().find((x) => typeof x === "string") ?? "") })

function uiOf(base) {
  const ask = (q) => asking.getStore().prompt(q)
  return {
    ...base,
    get theme() {
      return plain
    },
    async select(title, options) {
      if (!asking.getStore() || !options?.length) return undefined
      return ask({ type: "select", message: title, options: options.map((o) => ({ id: o, label: o })) })
    },
    async confirm(title, message) {
      if (!asking.getStore()) return false
      const yes = await ask({ type: "select", message: [title, message].filter(Boolean).join("\n"), options: [{ id: "yes", label: "Yes" }, { id: "no", label: "No" }] })
      return yes === "yes"
    },
    async input(title, placeholder) {
      if (!asking.getStore()) return undefined
      return ask({ type: "text", message: title, placeholder: placeholder ?? "" })
    },
    async editor(title, prefill) {
      if (!asking.getStore()) return undefined
      return ask({ type: "text", message: title, placeholder: prefill ?? "" })
    },
    notify(message) {
      asking.getStore()?.notify({ type: "info", message: String(message) })
    },
    async custom(factory) {
      const i = asking.getStore()
      return i ? shown(i, factory) : undefined
    },
  }
}

// shown runs a component of pi's terminal UI (ctx.ui.custom: Kiro's choice
// of sign-in) with no terminal: the list it shows is asked as a choice and
// the field as text, each answer given to it as its keys would, until it
// is done; one with nothing to answer (a sign-in's waiting screen) is
// waited on.
async function shown(i, factory) {
  let result
  let over = false
  let finish
  const done = new Promise((ok) => (finish = ok))
  const tui = new Proxy({ terminal: { columns: 100, rows: 40 } }, { get: (t, k) => (k in t ? t[k] : k === "then" ? undefined : () => {}) })
  const c = await factory(tui, plain, plain, (v) => {
    if (over) return
    over = true
    result = v
    finish()
  })
  try {
    while (!over) {
      // what it does on an answer (its next screen) comes first
      await new Promise((ok) => setTimeout(ok, 0))
      if (over) break
      const ctl = controlOf(c)
      if (!ctl) {
        await done
        break
      }
      if (ctl.list) {
        const items = ctl.list.filteredItems ?? ctl.list.items
        const v = await i.prompt({ type: "select", message: ctl.message, options: items.map((x) => ({ id: String(x.value), label: x.label ?? String(x.value), description: x.description ?? "" })) })
        const item = items.find((x) => String(x.value) === String(v))
        if (item) ctl.list.onSelect?.(item)
        else ctl.list.onCancel?.()
      } else {
        const v = String((await i.prompt({ type: "text", message: ctl.message, placeholder: "" })) ?? "")
        ctl.input.setValue?.(v)
        ctl.input.onSubmit?.(v)
      }
    }
  } finally {
    c?.dispose?.()
  }
  return result
}

const strip = (l) => String(l).replace(/\x1b\[[0-9;]*m/g, "").trim()

// controlOf is the first list or field a component shows, with the text
// shown above it, rules left out, as its question. A component may keep
// its parts to itself (Kiro's gives only render and handleInput), and each
// package loads its own copy of pi-tui, so its classes can't be told
// apart; but drawing walks the parts in order, each container going
// through its children, so the arrays walked while it draws are what it
// shows.
function controlOf(root) {
  const seen = []
  const walk = Array.prototype[Symbol.iterator]
  Array.prototype[Symbol.iterator] = function () {
    for (let i = 0; i < this.length; i++) {
      const x = this[i]
      if (x && typeof x === "object" && typeof x.render === "function" && !seen.includes(x)) seen.push(x)
    }
    return walk.call(this)
  }
  try {
    root?.render?.(100)
  } catch {
  } finally {
    Array.prototype[Symbol.iterator] = walk
  }
  const at = seen.findIndex((c) => (Array.isArray(c.items) && typeof c.onSelect === "function") || typeof c.onSubmit === "function")
  if (at < 0) return undefined
  const c = seen[at]
  const above = seen
    .slice(0, at)
    .filter((t) => typeof t.text === "string")
    .map((t) => strip(t.text))
    .filter((l) => l && !/^[\s─━│┃┌┐└┘├┤┬┴┼╭╮╯╰═║-]*$/.test(l))
  return { [typeof c.onSelect === "function" ? "list" : "input"]: c, message: above.join("\n") }
}

// ---- one provider's hooks --------------------------------------------------------

const LOGIN_WAIT = 10 * 60 * 1000

function loopbackRedirect(url) {
  try {
    const r = new URL(new URL(url).searchParams.get("redirect_uri") ?? "")
    return ["localhost", "127.0.0.1", "[::1]"].includes(r.hostname)
  } catch {
    return false
  }
}

// a sign-in under way: pi's login runs, and what it asks or shows comes
// out as events, one at a time
function startLogin(oauth) {
  const ctl = new AbortController()
  const timer = setTimeout(() => ctl.abort(new Error("the sign-in timed out")), LOGIN_WAIT)
  const run = { ctl, n: 0, asked: null, url: null, shown: [], waiters: [], result: null, done: false }
  const wake = () => {
    for (const w of run.waiters.splice(0)) w()
  }
  run.next = () => new Promise((ok) => (run.waiters.push(ok), run.asked || run.url || run.done ? wake() : null))
  const interaction = {
    signal: ctl.signal,
    prompt(q) {
      return new Promise((ok, no) => {
        run.asked = { q, ok, key: "pi" + run.n++ }
        ctl.signal.addEventListener("abort", () => no(ctl.signal.reason ?? new Error("Login cancelled")), { once: true })
        wake()
      })
    },
    notify(e) {
      if (e?.type === "auth_url" && e.url) run.url ??= { url: e.url, instructions: e.instructions ?? "" }
      else if (e?.type === "device_code") run.url ??= { url: e.verificationUriComplete ?? e.verificationUri ?? e.verificationUrl ?? "", instructions: `Enter the code ${e.userCode}` }
      else if (e?.type === "info" && e.message) run.shown.push(String(e.message))
      wake()
    },
  }
  run.promise = Promise.resolve()
    .then(() => asking.run(interaction, () => oauth.login(interaction)))
    .then(
      (c) => (run.result = { ...c, type: "success" }),
      (e) => (run.result = { type: "failed", error: String(e?.message ?? e) }),
    )
    .finally(() => {
      clearTimeout(timer)
      run.done = true
      wake()
    })
  return run
}

function promptOf(run) {
  const { q, key } = run.asked
  const select = q.type === "select" && Array.isArray(q.options) && q.options.length
  return {
    type: select ? "select" : "text",
    key,
    message: q.message ?? "",
    placeholder: q.placeholder ?? "",
    // pi's options are { id, label, description } and the login is answered
    // with the id (pi-zcode's "bigmodel"), never with what the reader saw
    options: select ? q.options.map((o) => (typeof o === "string" ? { label: o, value: o, hint: "" } : { label: o.label ?? String(o.id ?? o.value), value: String(o.id ?? o.value ?? o.label), hint: o.hint ?? o.description ?? "" })) : undefined,
  }
}

function hooksOf(h, mr, ai, id) {
  const prov = () => mr.getProvider(id)
  const runs = new Map() // method index → its sign-in under way

  function methods() {
    const a = prov()?.auth ?? {}
    const out = []
    if (a.oauth) {
      const m = {
        type: "oauth",
        label: a.oauth.loginLabel ?? `Sign in with ${a.oauth.name ?? prov()?.name ?? id}`,
      }
      const index = out.length
      // what the login asks before it shows its page, asked one at a time
      // as magpie asks a method's prompts (magpie's own hook)
      m.ask = async (inputs = {}) => {
        let run = runs.get(index)
        const answered = !!run?.asked && run.asked.key in inputs
        // asked with nothing answered: a sign-in begun afresh
        if (!run || run.done || (!answered && !Object.keys(inputs).length)) {
          run?.ctl.abort(new Error("Login cancelled"))
          run = startLogin(a.oauth)
          runs.set(index, run)
        }
        if (answered) {
          const { ok, key } = run.asked
          run.asked = null
          ok(String(inputs[key] ?? ""))
        }
        await run.next()
        return run.asked && !run.url ? promptOf(run) : null
      }
      m.authorize = async () => {
        let run = runs.get(index)
        if (!run || (run.done && run.result?.type !== "success" && !run.url)) {
          run = startLogin(a.oauth)
          runs.set(index, run)
        }
        while (!run.url && !run.done) {
          // a question left after the page (none expected): told nothing
          if (run.asked) {
            const { ok } = run.asked
            run.asked = null
            ok("")
          }
          await run.next()
        }
        if (run.done && run.result?.type !== "success" && !run.url) {
          runs.delete(index)
          throw new Error(run.result?.error ?? "the sign-in failed")
        }
        const url = run.url?.url ?? ""
        // a page that comes back to a server the login runs here finishes
        // by itself; one that doesn't asks for what it shows to be pasted
        await Promise.race([run.next(), new Promise((ok) => setTimeout(ok, 300))])
        const code = !!url && !loopbackRedirect(url) && !!run.asked && !run.done
        const instructions = [run.url?.instructions, ...run.shown].filter(Boolean).join("\n")
        return {
          url,
          instructions: code && !instructions ? run.asked.q.message ?? "" : instructions,
          method: code ? "code" : "auto",
          callback: async (pasted) => {
            runs.delete(index)
            if (pasted && run.asked) {
              const { ok } = run.asked
              run.asked = null
              ok(String(pasted))
            }
            await run.promise
            return run.result
          },
        }
      }
      out.push(m)
    }
    if (a.apiKey || !a.oauth) out.push({ type: "api", label: "API key" })
    return out
  }

  const auth = {
    provider: id,
    get methods() {
      return methods()
    },
    async loader() {
      return { apiKey: "pi", baseURL: "http://pi.magpie/v1", fetch: (url, init) => piFetch(url, init) }
    },
    // renewed by magpie ahead of its expiry, as pi renews it
    async refresh(stored) {
      const oauth = prov()?.auth?.oauth
      if (!oauth?.refresh) return undefined
      const { type: _t, ...c } = await oauth.refresh({ ...stored, type: "oauth" })
      return c
    },
  }

  async function piFetch(url, init) {
    let body
    try {
      body = JSON.parse(typeof init?.body === "string" ? init.body : new TextDecoder().decode(init?.body ?? new Uint8Array()))
    } catch {
      return failure(400, "pi's providers take Anthropic's Messages requests")
    }
    const model = mr.getModel(id, body.model)
    if (!model) return failure(404, `${prov()?.name ?? id} has no model ${body.model}`)
    let context
    try {
      context = await contextOf(body, model)
    } catch (e) {
      return failure(400, String(e?.message ?? e))
    }
    let res
    const seen = {}
    const max = Number(body.max_tokens) > 0 ? Number(body.max_tokens) : undefined
    const opts = {
      signal: init?.signal,
      maxTokens: max && model.maxTokens > 0 ? Math.min(max, model.maxTokens) : max ?? (model.maxTokens || undefined),
      reasoning: levelOf(body, model, ai.clampThinkingLevel),
      ...(typeof body.temperature === "number" ? { temperature: body.temperature } : {}),
      ...(body.tool_choice?.type === "none" ? { toolChoice: "none" } : body.tools?.length ? { toolChoice: "auto" } : {}),
      onResponse: (r) => void (res = r),
    }
    let events
    try {
      events = said.run(seen, () => mr.streamSimple(model, context, opts))
    } catch (e) {
      const msg = String(e?.message ?? e)
      return failure(statusOf(undefined, msg), msg)
    }
    return answer(events, { model: body.model, stream: body.stream !== false, res: () => res, said: seen })
  }

  return {
    auth,
    provider: {
      id,
      async models(_given, { auth: stored }) {
        h.mirror()
        if (stored && prov()?.refreshModels) {
          const r = await mr.refresh({ providers: [id], allowNetwork: true })
          const e = r?.errors?.get?.(id)
          if (e) h.send({ event: "log", level: "error", message: `${id}: models: ${e.message ?? e}` })
        }
        let list = mr.getModels(id) ?? []
        if (stored) {
          try {
            const av = await mr.getAvailable(id)
            if (av?.length) list = av
          } catch {}
        }
        const levels = (m) => {
          try {
            return ai.getSupportedThinkingLevels(m).filter((l) => l !== "off")
          } catch {
            return m.reasoning ? ["low", "medium", "high"] : []
          }
        }
        return Object.fromEntries(
          list
            .filter((m) => !m.type || m.type === "chat")
            .map((m) => [
              m.id,
              {
                id: m.id,
                providerID: id,
                name: m.name ?? m.id,
                api: { id: m.id, url: "", npm: "@ai-sdk/anthropic" },
                status: "active",
                headers: {},
                options: {},
                cost: { input: m.cost?.input ?? 0, output: m.cost?.output ?? 0, cache: { read: m.cost?.cacheRead ?? 0, write: m.cost?.cacheWrite ?? 0 } },
                limit: { context: m.contextWindow ?? 0, output: m.maxTokens ?? 0 },
                capabilities: {
                  temperature: true,
                  reasoning: !!m.reasoning,
                  attachment: (m.input ?? []).includes("image"),
                  toolcall: true,
                  input: { text: true, image: (m.input ?? []).includes("image"), audio: false, video: false, pdf: false },
                  output: { text: true, image: false, audio: false, video: false, pdf: false },
                  interleaved: false,
                },
                release_date: "",
                variants: m.reasoning ? Object.fromEntries(levels(m).map((l) => [l, {}])) : {},
              },
            ]),
        )
      },
    },
    config(c) {
      c.provider ??= {}
      c.provider[id] ??= {}
      c.provider[id].name ??= prov()?.name ?? id
    },
  }
}
