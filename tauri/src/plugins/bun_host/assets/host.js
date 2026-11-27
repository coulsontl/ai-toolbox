// magpie's plugin host: runs OpenCode (v1) server plugins under Bun and
// answers magpie over stdin/stdout, one JSON message a line.
//
// magpie → host: {id, method, params}; the host answers {id, result} or
// {id, error}. A fetch streams its reply first: {id, event: "head", status,
// headers}, then {id, event: "chunk", data} (base64) as the body comes, then
// {id, result: null}. {method: "abort", params: {id}} cancels one, and
// {method: "credit", params: {id, n}} gives a fetch back the n charged bytes
// its reader took (the payload it read, and a frame's overhead once the frame
// is done), so it may send that much more. A fetch starts with the window
// magpie gave it and sends no more than that ahead of its reader, so a slow
// reader pauses its upstream instead of the host buffering without bound.
// host → magpie, unasked: {event: "log", level, message} and {event:
// "toast", ...} (what a plugin logs or shows), {event: "auth", provider}
// (a sign-in the host saved or refreshed).
//
// The plugins get what OpenCode hands them (PluginInput): a client whose
// auth.set saves to magpie's plugin-auth.json in OpenCode's auth.json shape,
// tui.showToast and app.log, config.get; Bun's $; the folder magpie keeps
// its files in. What else a plugin asks the client for answers {data:
// undefined}.
//
// A provider may be signed in to more than once: the first account is kept
// under the provider's id, as OpenCode keeps it, the others under
// id#<slot>. Everything a request does runs in its account's scope, so the
// plugin's getAuth, client.auth.set and loader see that account alone.

import { AsyncLocalStorage } from "node:async_hooks"
import { createHash, randomUUID } from "node:crypto"
import dns from "node:dns"
import fs from "node:fs"
import http2 from "node:http2"
import net from "node:net"
import os from "node:os"
import path from "node:path"
import readline from "node:readline"
import { Duplex } from "node:stream"
import tls from "node:tls"
import { pathToFileURL } from "node:url"

const rpcWrite = process.stdout.write.bind(process.stdout)

// Everything the child writes goes through one ordered queue: a line goes out
// only when the stream can take it, so a parent that stops reading bounds what
// Bun buffers instead of it growing without bound. A caller may cancel its own
// wait, which drops its line if it has not been written yet, so an aborted
// request leaves no listener or closure behind. A write failure or a closed
// stream fails every line still waiting rather than leaving them pending.
const outq = [] // {line, settle}
let pumping = false
let outFail = null

function waitDrain() {
  return new Promise((resolve, reject) => {
    const done = () => { clear(); resolve() }
    const bad = (e) => { clear(); reject(e ?? new Error("the host's stdout closed")) }
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
    // write returns false when the stream is full, but the line was still
    // taken: wait for it to drain before the next one, never write it twice
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

// queueLine queues one line, giving a promise that settles with null once it is
// written or with the error that stopped it, and a way to give the wait up.
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

// send queues one message, ignoring whether it was written.
const send = (msg) => queueLine(JSON.stringify(msg) + "\n").promise

// sendCancelable is send, with a way to give the wait up. A string is a
// line made already (chunkLine).
const sendCancelable = (msg) => queueLine(typeof msg === "string" ? msg : JSON.stringify(msg) + "\n")

// chunkLine is JSON.stringify({id, event: "chunk", data}) + "\n", made by
// joining: base64 has nothing JSON escapes, and stringify scanning every
// byte of it for something to escape cost more than the base64 itself.
const chunkLine = (id, data) => '{"id":' + JSON.stringify(id) + ',"event":"chunk","data":"' + data + '"}\n'

// A host cancellation suppresses the answer; a local abort still owes Go
// its error. The record, rather than the signal, owns pending wire output.
function transitionFetch(record, state) {
  if (record.state === "finished") return
  if (record.state !== "cancelled") record.state = state
  if (state === "streaming") return
  record.gate.stop()
  const body = record.unconsumedBody
  record.unconsumedBody = undefined
  try { Promise.resolve(body?.cancel?.()).catch(() => {}) } catch {}
  if (state === "finishing") return
  record.pendingSend?.cancel()
  record.readyCleanup?.()
  record.readyCleanup = null
  if (state === "cancelled") record.controller.abort()
  else record.state = "finished"
}

function checkFetch(record) {
  if (record.state === "cancelled" || record.state === "finished") throw new DOMException("Aborted", "AbortError")
  record.controller.signal.throwIfAborted()
}

async function sendForFetch(record, msg) {
  if (record.state === "cancelled" || record.state === "finished") return
  const ctl = record.controller
  const pendingSend = sendCancelable(msg)
  record.pendingSend = pendingSend
  const onAbort = () => pendingSend.cancel()
  ctl.signal.addEventListener("abort", onAbort, { once: true })
  try {
    const err = await pendingSend.promise
    if (err) throw err
  } finally {
    record.pendingSend = null
    ctl.signal.removeEventListener("abort", onAbort)
  }
}

// The largest decoded bytes one frame carries: the reader credits in larger
// steps than this, so frames stay small enough that a credit round trip is
// cheap without one frame dwarfing the window.
const MAX_FRAME = 64 << 10

// FRAME_OVERHEAD is what one queued frame costs the window besides its bytes,
// as Go counts it too: the message struct and the decoded slice behind it. It
// keeps a producer of tiny frames from filling a window with structs.
const FRAME_OVERHEAD = 2048

// HEAD_MAX and HEAD_ENTRIES are magpie's own head-envelope policy, not HTTP's:
// a fetch's head carries at most HEAD_MAX header bytes (the UTF-8 bytes of
// every name and value, plus HEAD_ENTRY_COST an entry) and HEAD_ENTRIES
// entries, counted the same by Go before it is queued, so a vendor's unbounded
// header set is not a free 0-charge line.
const HEAD_MAX = 64 << 10
const HEAD_ENTRIES = 256
const HEAD_ENTRY_COST = 4

// headCost is a head's header cost, as Go counts it too: the UTF-8 bytes of
// every name and value, plus HEAD_ENTRY_COST an entry.
function headCost(h) {
  let n = 0
  for (const [k, v] of Object.entries(h)) n += Buffer.byteLength(k) + Buffer.byteLength(v) + HEAD_ENTRY_COST
  const entries = Object.keys(h).length
  return { n, entries }
}

// makeGate is one fetch's credit: the child may have `window` decoded bytes
// outstanding, each frame costing its bytes plus FRAME_OVERHEAD. take returns
// how many bytes of a frame may go now (-1 when the fetch was stopped), spend
// takes the frame's cost, add is the reader giving credit back.
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
      if (wake && free > FRAME_OVERHEAD) { const w = wake; wake = null; w(true) }
    },
    stop() {
      stopped = true
      if (wake) { const w = wake; wake = null; w(false) }
    },
  }
}

// A plugin writing to stdout would break the protocol: everything it
// prints goes to stderr, which magpie logs.
const toErr = (...xs) => process.stderr.write(xs.map((x) => (typeof x === "string" ? x : Bun.inspect(x))).join(" ") + "\n")
console.log = console.info = console.debug = console.warn = toErr
process.stdout.write = (chunk, enc, cb) => process.stderr.write(chunk, enc, cb)

// Nor may a program a plugin starts have them: one given the host's stdin
// reads magpie's messages away (pi-devin-plus runs `devin auth login` with
// stdio "inherit", which took the next requests as its answer and gave up,
// "Login canceled"), and its stdout is the host's answers. It reads
// nothing instead and writes to stderr. node:child_process starts every
// program through Bun.spawn and Bun.spawnSync, so these see all of them.
//
// And it has the proxy the plugin's own fetches take, as a built-in's CLI
// had magpie's (netproxy.Env): the host has its *_PROXY only as
// MAGPIE_*_PROXY (see below), so `grok login`, which the Grok plugin runs,
// went out with none and, where x.ai is reached only through one, printed
// no link to open (𝕏 on Discord). One the plugin set itself is kept.
// Node's fetch goes past *_PROXY unless NODE_USE_ENV_PROXY=1 says to take
// them, so a plugin downloading in a Node program it starts went out with
// none and failed where the vendor is reached only through one (larchsis
// on 𝕏, Windows 11, a proxy set in magpie only). It is told to, unless
// the plugin said otherwise.
const own = (v, fd) => v === "inherit" || v === fd || v === (fd ? process.stdout : process.stdin)
const PROXY_VARS = ["HTTPS_PROXY", "HTTP_PROXY", "ALL_PROXY"]
function proxied(env) {
  env = { ...(env ?? process.env) }
  if (Object.keys(env).some((k) => PROXY_VARS.includes(k.toUpperCase()) && env[k])) return env
  const mine = via.getStore()
  if (mine === "direct") return env
  const set = mine ? { HTTPS_PROXY: mine, HTTP_PROXY: mine } : { HTTPS_PROXY: globalProxy.https, HTTP_PROXY: globalProxy.http, NO_PROXY: proxyVar("NO_PROXY") }
  if (set.HTTPS_PROXY || set.HTTP_PROXY) set.NODE_USE_ENV_PROXY = "1"
  for (const [k, v] of Object.entries(set)) {
    if (!v || Object.keys(env).some((e) => e.toUpperCase() === k)) continue
    env[k] = v
    // Windows' names are one whatever their case
    if (process.platform !== "win32" && k.endsWith("_PROXY") && k !== "NODE_USE_ENV_PROXY") env[k.toLowerCase()] = v
  }
  return env
}
// And a program started with no folder of its own runs in workDir, not
// the host's: the host runs in magpie's config folder, beside
// settings.json and providers.json, so whatever a program saved by a
// relative name went there, and a tool told to write "providers.json"
// would write over magpie's. The chat page plugin (magpie-chat) starts
// the user's MCP servers this way and runs their tools for a model,
// whose fetched pages and saved files (jiakun_zhao on 𝕏 asked whose
// page_<hash>.html and magpie_*.md files they had found) belong in a
// folder of their own. A folder the plugin gives is kept.
const workDir = path.join(process.cwd(), "plugin-work")
let workReady = false
function workCwd() {
  if (!workReady) {
    try {
      fs.mkdirSync(workDir, { recursive: true })
      workReady = true
    } catch {
      return undefined
    }
  }
  return workDir
}
const guard = (o) => {
  if (o != null && typeof o !== "object") return o
  o = { ...o }
  if (o.cwd == null || o.cwd === "") {
    const d = workCwd()
    if (d) {
      o.cwd = d
      // and PWD with it, which a shell script reads for its folder
      if (process.platform !== "win32") o.env = { ...(o.env ?? process.env), PWD: d }
    }
  }
  o.env = proxied(o.env)
  if (Array.isArray(o.stdio)) {
    o.stdio = [...o.stdio]
    if (own(o.stdio[0], 0)) o.stdio[0] = "ignore"
    if (own(o.stdio[1], 1)) o.stdio[1] = 2
  }
  if (own(o.stdin, 0)) o.stdin = "ignore"
  if (own(o.stdout, 1)) o.stdout = 2
  return o
}
for (const name of ["spawn", "spawnSync"]) {
  const run = Bun[name]
  Bun[name] = function (a, b) {
    return Array.isArray(a) ? run.call(this, a, guard(b)) : run.call(this, guard(a))
  }
}

let authPath = ""
let modelsDevPath = ""
let piPath = "" // pi.js, which loads pi's extensions
let directory = process.cwd()
let userConfig = {}
// prefer is the plugin the user picked for a provider id more than one
// plugin signs in to: id → its spec (plugins.json's prefer)
let prefer = {}
// chosen are the user's own values of the plugins' options (auth.options),
// provider id → key → true/false (settings.json's pluginOptions, #2038)
let chosen = {}
const hooks = [] // {spec, hooks}
const gatewayHooks = [] // {spec, options, onRequest}
const loaded = [] // {spec, id, error}
const loaders = new Map() // account → options the auth loader returned
// infos are info's answers a request is given, by account: built once, not
// for every fetch, which asked the plugin's models hook (its vendor, once
// its own list's time was up) before each request went. One goes with its
// account's sign-in (forgetAccount), and all with the options the user set
// (setProviderOptions) or a new models.dev catalog; a listing (providers)
// puts its own in.
const infos = new Map() // account → {gen, md, info}
let infoGen = 0

// forgetAccount drops what was made of the sign-in at key: its loader's
// options and its info, which the next request makes again.
function forgetAccount(key) {
  loaders.delete(key)
  infos.delete(key)
  infoGen++
}
const sessions = new Map() // oauth sign-in in progress → its authorize result
const inflight = new Map() // fetch id → its lifecycle and owned resources
const renewing = new Map() // account → its sign-in's renewal under way
const unrenewed = new Map() // account → its last renewal that failed: {at, secret, gone, error, n}
let config = { provider: {} } // what the plugins' config hooks made of it

// loadWait bounds one plugin's load (its import and its plugin function)
// and one config hook; listWait bounds one provider's models hook in a
// listing. A plugin stuck on its vendor (a sign-in answered 401 and tried
// again, a vendor that never answers) is then given up on alone: the
// plugins load side by side and every other one is listed meanwhile,
// rather than all waiting behind it until magpie's own deadline fails the
// whole host or the whole list. magpie sets both at init.
let loadWait = 20 * 1000
let listWait = 20 * 1000

// watchHeavy says once when this host holds more than heavy.mb MiB, a
// full collection run first so garbage not yet collected doesn't count:
// magpie then starts a new host for the calls to come and stops this one
// once its own are answered (host.go's heavyMB and recycle). It checks
// every heavy.everyMillis, from heavy.afterMillis after the host started.
function watchHeavy(heavy) {
  const mb = Number(heavy?.mb)
  if (!(mb > 0)) return
  const every = Math.max(Number(heavy.everyMillis) || 60_000, 10)
  const after = Math.max(Number(heavy.afterMillis) || 0, 0)
  const rss = () => process.memoryUsage().rss / 1048576
  const timer = setInterval(() => {
    if (process.uptime() * 1000 < after || rss() <= mb) return
    Bun.gc(true)
    const held = rss()
    if (held <= mb) return
    clearInterval(timer)
    send({ event: "heavy", count: Math.round(held) })
  }, every)
  timer.unref?.()
}

// late is what a step that took longer than ms gives instead of its own
// result: an error saying so. The step goes on by itself; what it gives
// later is dropped. A step handed a signal of its own (a model hook's) has
// its controller aborted here, so the I/O a deadline gives up on stops too
// rather than waiting for a caller that has moved on (#4678).
class Late extends Error {}
function inTime(step, ms, what, ctl) {
  let timer
  const late = new Promise((_, no) => {
    timer = setTimeout(() => {
      ctl?.abort()
      no(Object.assign(new Late(`${what} didn't finish in ${Math.round(ms / 1000)}s`), { plain: true }))
    }, ms)
  })
  return Promise.race([Promise.resolve().then(step), late]).finally(() => clearTimeout(timer))
}

// ---- a provider's own proxy ---------------------------------------------------

// A request made for a provider or an account with a proxy of its own
// (#237) goes through it, and so does every fetch the plugin makes for
// it; one for a provider set to "direct" goes through none. Bun reads
// *_PROXY once, and a fetch given no proxy option can't be told to skip
// them, so the host is started with them as MAGPIE_*_PROXY and each
// fetch is given magpie's own here when it has none of its own.
// Loopback never goes through one.
const via = new AsyncLocalStorage() // the proxy: "", "direct" or its URL
const bunFetch = globalThis.fetch

function proxyVar(k) {
  return process.env["MAGPIE_" + k] || process.env["MAGPIE_" + k.toLowerCase()] || ""
}

const globalProxy = {
  https: proxyVar("HTTPS_PROXY") || proxyVar("ALL_PROXY"),
  http: proxyVar("HTTP_PROXY") || proxyVar("ALL_PROXY"),
  no: proxyVar("NO_PROXY")
    .split(",")
    .map((s) => s.trim().toLowerCase().replace(/^\*?\./, ""))
    .filter(Boolean),
}

function hostOf(input) {
  try {
    const u = new URL(typeof input === "string" || input instanceof URL ? input : input.url)
    return { protocol: u.protocol, host: u.hostname.toLowerCase().replace(/^\[|\]$/g, "") }
  } catch {
    return null
  }
}

function loopback(host) {
  return host === "localhost" || host.endsWith(".localhost") || host === "::1" || /^127\./.test(host)
}

// A host on the user's own network — another computer's magpie, an LLM
// relay on the NAS — is reached directly, whatever proxy the host was
// started with, as magpie's own requests reach it (netproxy.onLAN, lan.go,
// #982, #1572): a proxy is for reaching vendors; one elsewhere can't reach
// the user's network, and one here only goes there directly itself. A
// provider's own proxy (via) is still taken as it is. The rule is lan.go's:
// keep the two the same.

// lanWait is how long a name is looked up for before it is taken as not on
// the network; lanKeep is how long what it resolved to is kept.
const lanWait = 400
const lanKeep = 60_000
const lanSeen = new Map() // host → {lan: Promise<boolean>, at}

// ipBytes is the 16 bytes of the IPv6 address s (an IPv4 one as
// ::ffff:a.b.c.d), null when s is no address.
function ipBytes(s) {
  if (net.isIPv4(s)) return [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff, ...s.split(".").map(Number)]
  if (!net.isIPv6(s)) return null
  s = s.replace(/%.*$/, "")
  const v4 = s.match(/(\d+\.\d+\.\d+\.\d+)$/)
  if (v4) {
    const [a, b, c, d] = v4[1].split(".").map(Number)
    s = s.slice(0, -v4[1].length) + ((a << 8) | b).toString(16) + ":" + ((c << 8) | d).toString(16)
  }
  const [head, tail] = s.includes("::") ? s.split("::") : [s, null]
  const words = (x) => (x ? x.split(":").map((w) => parseInt(w, 16)) : [])
  const h = words(head)
  const t = words(tail)
  const all = tail === null ? h : [...h, ...Array(8 - h.length - t.length).fill(0), ...t]
  return all.flatMap((w) => [w >> 8, w & 0xff])
}

// lanIP is netproxy.lanIP: a private (10/8, 172.16/12, 192.168/16,
// fc00::/7), link-local (169.254/16, fe80::/10), loopback or carrier-grade
// NAT (Tailscale's 100.64/10) address.
function lanIP(s) {
  const b = ipBytes(s)
  if (!b) return false
  if (b.slice(0, 10).every((x) => x === 0) && b[10] === 0xff && b[11] === 0xff) {
    const [p, q] = b.slice(12)
    return p === 10 || (p === 172 && (q & 0xf0) === 16) || (p === 192 && q === 168) || (p === 169 && q === 254) || p === 127 || (p === 100 && (q & 0xc0) === 64)
  }
  return (b[0] & 0xfe) === 0xfc || (b[0] === 0xfe && (b[1] & 0xc0) === 0x80) || b.every((x, i) => x === (i === 15 ? 1 : 0))
}

// onLAN is netproxy.onLAN: whether host is on the user's own network — an
// address lanIP takes, a .local name, or a name every address of which is
// one of those. A name that doesn't resolve here (in lanWait) is not: the
// proxy is what reaches it.
function onLAN(host) {
  host = host.toLowerCase().replace(/\.$/, "")
  if (!host) return Promise.resolve(false)
  if (net.isIP(host)) return Promise.resolve(lanIP(host))
  if (host.endsWith(".local")) return Promise.resolve(true)
  const seen = lanSeen.get(host)
  if (seen && Date.now() - seen.at < lanKeep) return seen.lan
  let timer
  const lan = Promise.race([
    dns.promises.lookup(host, { all: true, verbatim: true }).then(
      (as) => as.length > 0 && as.every((a) => lanIP(a.address)),
      () => false,
    ),
    new Promise((r) => (timer = setTimeout(() => r(false), lanWait))),
  ]).finally(() => clearTimeout(timer))
  if (lanSeen.size >= 256) lanSeen.clear()
  lanSeen.set(host, { lan, at: Date.now() })
  return lan
}

// proxyOf is the proxy a request to input goes through, "" for none, and
// whether it is the global one, which a host on the user's own network
// goes around (proxyFor).
function proxyOf(input) {
  const u = hostOf(input)
  if (!u || loopback(u.host)) return { p: "" }
  const own = via.getStore()
  if (own === "direct") return { p: "" }
  if (own) return { p: own }
  if (globalProxy.no.some((n) => n === "*" || u.host === n || u.host.endsWith("." + n))) return { p: "" }
  const p = u.protocol === "https:" ? globalProxy.https : u.protocol === "http:" ? globalProxy.http : ""
  return { p, global: !!p, host: u.host }
}

// proxyFor is the proxy a fetch of input goes through, "" for none.
async function proxyFor(input) {
  const { p, global, host } = proxyOf(input)
  return global && (await onLAN(host)) ? "" : p
}

// reach is what a check's fetches came to: whether one got an answer, and
// the first that got none
const reach = new AsyncLocalStorage()

// sent is a fetch as the plugin asked it, through each redirect it may
// follow (followed); hop is one request of it, through its proxy.
function sent(input, init) {
  const redirect = init?.redirect ?? (input instanceof Request ? input.redirect : undefined) ?? "follow"
  if (redirect !== "follow") return hop(input, init)
  return followed(input, init)
}

// A plugin's request is followed through a redirect with its headers only
// to its own origin (the scheme, host and port it was sent to), or from
// http to https on the same host: provider.KeepsCredentials' rule, which
// magpie's own requests take (redirect.go, #5508). Bun follows any
// redirect anywhere and drops only Authorization and Cookie on a hop to
// another origin, so x-api-key, x-goog-api-key and a vendor's own headers
// went to wherever the vendor's Location named. A redirect elsewhere is
// given back to the plugin as it came, Location and all, unless the
// request carries nothing of its own (bare: no body, and only the headers
// any client sends), as a download handed on to a CDN is. A fetch the
// plugin asked with redirect "manual" or "error" is left as it asked.
const REDIRECTS = new Set([301, 302, 303, 307, 308])
const BARE_HEADERS = new Set(["accept", "accept-encoding", "accept-language", "user-agent", "cache-control", "pragma", "range", "if-range", "if-none-match", "if-modified-since"])
const BODY_HEADERS = ["content-type", "content-length", "content-encoding", "content-language", "content-location"]

// sameOrigin is provider.SameOrigin, for URLs: a URL leaves out its
// scheme's own port.
function sameOrigin(from, to) {
  const port = (u) => u.port || (u.protocol === "http:" ? "80" : u.protocol === "https:" ? "443" : "")
  if (!from.hostname || from.hostname.toLowerCase() !== to.hostname.toLowerCase()) return false
  if (from.protocol === to.protocol) return port(from) === port(to)
  // an upgrade: the request went over http already, to this same host
  return from.protocol === "http:" && to.protocol === "https:" && port(from) === "80" && port(to) === "443"
}

async function followed(input, init) {
  let url, opts
  if (input instanceof Request) {
    const body = init && "body" in init ? init.body : input.body ? await input.arrayBuffer() : undefined
    opts = { method: input.method, headers: input.headers, signal: input.signal, ...init, body }
    url = input.url
  } else {
    url = String(input instanceof URL ? input.href : input)
    opts = { ...init }
  }
  const first = new URL(url)
  let method = (opts.method ?? "GET").toUpperCase()
  let headers = new Headers(opts.headers)
  let body = opts.body
  // a stream can be sent once: a redirect that would send it again is the answer
  const replayable = body == null || !(body instanceof ReadableStream)
  const bare = () => body == null && [...headers.keys()].every((k) => BARE_HEADERS.has(k))
  for (let n = 0; ; n++) {
    const res = await hop(url, { ...opts, method, headers, body, redirect: "manual" })
    const at = res.headers.get("location")
    if (!REDIRECTS.has(res.status) || at == null) return n ? said(res, url) : res
    let next
    try {
      next = new URL(at, url)
    } catch {
      return n ? said(res, url) : res
    }
    if (next.protocol !== "http:" && next.protocol !== "https:") return n ? said(res, url) : res
    if (!sameOrigin(first, next) && !bare()) return n ? said(res, url) : res
    const drops = res.status === 303 ? method !== "HEAD" : (res.status === 301 || res.status === 302) && method === "POST"
    if (!drops && body != null && !replayable) return n ? said(res, url) : res
    if (n + 1 >= 10) {
      res.body?.cancel().catch(() => {})
      throw new TypeError("stopped after 10 redirects")
    }
    res.body?.cancel().catch(() => {})
    if (drops) {
      if (method !== "HEAD") method = "GET"
      body = undefined
      headers = new Headers(headers)
      for (const k of BODY_HEADERS) headers.delete(k)
    }
    url = next.href
  }
}

// said is a reply reached through a redirect as fetch says one: where it
// came from, and that it was redirected.
function said(res, url) {
  try {
    Object.defineProperty(res, "redirected", { value: true })
    Object.defineProperty(res, "url", { value: url })
  } catch {}
  return res
}

async function hop(input, init) {
  if (init && "proxy" in init) return bunFetch(input, init).catch((e) => throwTLS(e, input))
  const p = await proxyFor(input)
  if (!p) return bunFetch(input, init).catch((e) => throwTLS(e, input))
  return bunFetch(input, { ...init, proxy: p }).catch(async (e) => {
    if (e?.name === "AbortError" || init?.signal?.aborted) throw e
    const why = await proxyRefuses(p, input)
    if (why) throw Object.assign(new Error(`proxyconnect tcp: ${why}`), { code: e?.code, cause: e })
    throwTLS(e, input)
  })
}

// The codes Bun gives a fetch whose server's certificate it didn't take.
const tlsCodes = new Set([
  "UNABLE_TO_GET_ISSUER_CERT",
  "UNABLE_TO_GET_ISSUER_CERT_LOCALLY",
  "UNABLE_TO_VERIFY_LEAF_SIGNATURE",
  "SELF_SIGNED_CERT_IN_CHAIN",
  "DEPTH_ZERO_SELF_SIGNED_CERT",
  "CERT_HAS_EXPIRED",
  "CERT_NOT_YET_VALID",
  "CERT_UNTRUSTED",
  "ERR_TLS_CERT_ALTNAME_INVALID",
])

// throwTLS throws e, its message saying which host's certificate the
// plugin host refused and with what code, when that is why it failed:
// Bun says only "unable to get issuer certificate", which read like the
// vendor's own fault, for a chain that verified outside the host
// (youyo.p_10001). The message Bun gave stays first, and the error is
// the same one, its code and cause kept.
function throwTLS(e, input) {
  if (e && tlsCodes.has(e.code) && typeof e.message === "string" && !e.message.includes("plugin host's TLS")) {
    const h = hostOf(input)?.host
    try {
      e.message = `${e.message} (${e.code}: the plugin host's TLS didn't verify ${h ? h + "'s" : "the server's"} certificate chain against the roots it trusts)`
    } catch {}
  }
  throw e
}

// proxyRefuses is why the proxy p won't carry a request to input, "" when
// it will: Bun says a proxy that is down, or that turns the tunnel away,
// only as a connection that failed, which a vendor's own failure is too.
// The tunnel is asked for again, as Go's transport asks for it, so a
// failure is said in its words ("proxyconnect …") and the gateway takes it
// for the proxy's, asking the next account rather than resting this one.
function proxyRefuses(p, input) {
  let pu, target
  try {
    pu = new URL(p)
    const u = new URL(typeof input === "string" || input instanceof URL ? input : input.url)
    target = `${u.hostname.includes(":") ? `[${u.hostname.replace(/^\[|\]$/g, "")}]` : u.hostname}:${u.port || (u.protocol === "http:" ? 80 : 443)}`
  } catch {
    return Promise.resolve("")
  }
  const host = pu.hostname.replace(/^\[|\]$/g, "")
  const port = Number(pu.port || (pu.protocol === "https:" ? 443 : 80))
  return new Promise((resolve) => {
    let got = ""
    let refused
    const done = (why) => {
      sock.destroy()
      resolve(why)
    }
    const opts = { host, port, servername: net.isIP(host) ? undefined : host }
    const sock = pu.protocol === "https:" ? tls.connect(opts) : net.connect(opts)
    sock.setTimeout(10_000, () => done(`dial tcp ${host}:${port}: i/o timeout`))
    sock.on("error", (e) => done(`dial tcp ${host}:${port}: ${e?.message ?? e}`))
    sock.once(pu.protocol === "https:" ? "secureConnect" : "connect", () => {
      let req = `CONNECT ${target} HTTP/1.1\r\nHost: ${target}\r\n`
      if (pu.username) {
        const cred = `${decodeURIComponent(pu.username)}:${decodeURIComponent(pu.password)}`
        req += `Proxy-Authorization: Basic ${Buffer.from(cred).toString("base64")}\r\n`
      }
      sock.write(req + "\r\n")
    })
    sock.on("data", (d) => {
      got += d.toString("latin1")
      const end = got.indexOf("\r\n\r\n")
      if (end < 0 && got.length < 8192) return
      const status = got.slice(0, got.indexOf("\r\n")).replace(/^HTTP\/\d(\.\d)?\s+/, "")
      if (/^2\d\d/.test(status)) return done("")
      // a bridge to a SOCKS proxy says why after its status
      refused ??= () => {
        const body = end < 0 ? "" : got.slice(end + 4, end + 304).trim()
        return body ? `${status}: ${body}` : status
      }
      setTimeout(() => done(refused()), 50)
    })
    sock.on("end", () => done(refused ? refused() : got ? "" : `${host}:${port} closed the connection`))
  })
}

// listing is what a models hook's fetches came to: whether it asked any,
// and whether the last it asked answered
const listing = new AsyncLocalStorage()

function asked(input, init) {
  const l = listing.getStore()
  if (!l) return sent(input, init)
  l.tried = true
  return sent(input, init).then(
    (res) => ((l.lastOk = res.ok), (l.said = res.ok ? "" : `HTTP ${res.status}`), res),
    (e) => {
      l.lastOk = false
      l.said = e?.message ?? String(e)
      throw e
    },
  )
}

// MAGPIE_ONLY are the headers magpie tells a plugin's fetch where the agent
// that asked works (WorkDirHeader and WorkGitHeader in internal/provider),
// that a queue needn't be waited out (FallbackHeader), and the session and
// subagent the agent named (SessionToPluginHeader, SubagentToPluginHeader)
// and the effort asked of a model with variants (EffortHeader), and the
// agent the request came from (CallerAgentHeader and the two beside it):
// for the plugin to read, never for its vendor, so a plugin that sends on
// the headers it was given sends them nowhere.
const MAGPIE_ONLY = ["x-magpie-work-dir", "x-magpie-work-git", "x-magpie-fallback", "x-magpie-session", "x-magpie-subagent", "x-magpie-effort", "x-magpie-caller-agent", "x-magpie-caller-name", "x-magpie-caller-via"]

function unsaid(input, init) {
  if (input instanceof Request && MAGPIE_ONLY.some((k) => input.headers.has(k))) {
    const h = new Headers(input.headers)
    for (const k of MAGPIE_ONLY) h.delete(k)
    input = new Request(input, { headers: h })
  }
  if (init?.headers) {
    const h = new Headers(init.headers)
    if (MAGPIE_ONLY.some((k) => h.has(k))) {
      for (const k of MAGPIE_ONLY) h.delete(k)
      init = { ...init, headers: h }
    }
  }
  return [input, init]
}

// signInSignal is the signal of the sign-in a plugin's step runs for: a
// fetch made for it fails once magpie has given the sign-in up.
const signInSignal = new AsyncLocalStorage()

// whileSigningIn gives a fetch made for a sign-in that sign-in's signal
// beside its own, so a sign-in magpie gave up on stops polling its vendor
// (#5016).
function whileSigningIn(input, init) {
  const signal = signInSignal.getStore()
  if (!signal) return [input, init]
  const own = init?.signal ?? (input instanceof Request ? input.signal : undefined)
  return [input, { ...init, signal: own ? AbortSignal.any([own, signal]) : signal }]
}

globalThis.fetch = Object.assign(function fetch(input, init) {
  ;[input, init] = unsaid(input, init)
  ;[input, init] = whileSigningIn(input, init)
  const r = reach.getStore()
  if (!r) return asked(input, init)
  return asked(input, init).then(
    (res) => ((r.reached = true), res),
    (e) => {
      r.failed ??= e
      throw e
    },
  )
}, bunFetch)

// An HTTP/2 session a plugin opens itself goes through the proxy its
// fetches take, as a built-in's requests on Go's transport did. Cursor's
// plugin runs agent.v1.AgentService/Run on node:http2, which Bun connects
// directly, *_PROXY or not: where Cursor is reached only through magpie's
// proxy (the system's, or one set in magpie), its sign-in, models and
// usage came through and every chat went out without it (#1035).
const h2connect = http2.connect
http2.connect = function connect(authority, options, listener) {
  if (typeof options === "function") [listener, options] = [options, undefined]
  let u
  try {
    u = typeof authority === "string" ? new URL(authority) : authority instanceof URL ? authority : new URL(`${authority.protocol ?? "https:"}//${authority.host ?? authority.hostname}`)
  } catch {
    return h2connect.call(this, authority, options, listener)
  }
  const { p, global } = options?.createConnection ? { p: "" } : proxyOf(u.href)
  if (!p) return h2connect.call(this, authority, options, listener)
  const host = u.hostname.replace(/^\[|\]$/g, "")
  const port = Number(u.port || (u.protocol === "http:" ? 80 : 443))
  // the session is returned now, and its connection made once it is known
  // whether host is on the user's own network (direct) or not (the proxy)
  const raw = tunnel(global ? onLAN(host).then((lan) => (lan ? "" : p)) : p, host, port)
  const createConnection = () =>
    u.protocol === "http:" ? raw : tls.connect({ ...options, socket: raw, servername: options?.servername || (net.isIP(host) ? undefined : host), ALPNProtocols: ["h2"] })
  const session = h2connect.call(this, authority, { ...options, createConnection }, listener)
  // the tunnel's failure is the session's: Bun's TLS over a stream leaves
  // the stream's errors unheard (one unheard ends the host), and says a
  // tunnel that never came as "h2 is not supported"
  raw.on("error", (e) => session.destroy(e))
  return session
}

// tunnel is a connection to host:port through the HTTP proxy which names
// (a proxy, or a promise of one), or straight to it when that is "" (a host
// on the user's own network, onLAN). A SOCKS proxy is handed to the host as
// an HTTP bridge to it, netproxy.ForBun: its CONNECT is asked first, and
// what is written waits for the tunnel. A proxy that turns it away fails
// it in Go's words ("proxyconnect …").
function tunnel(which, host, port) {
  const target = `${host.includes(":") ? `[${host}]` : host}:${port}`
  let ready = false
  let sock = null
  let ended = false
  let gone = false
  const queued = []
  const conn = new Duplex({
    read() {},
    write(chunk, enc, cb) {
      if (ready) sock.write(chunk, cb)
      else queued.push([chunk, cb])
    },
    final(cb) {
      if (ready) sock.end()
      else ended = true
      cb()
    },
    destroy(err, cb) {
      gone = true
      sock?.destroy()
      cb(err)
    },
  })
  const open = () => {
    ready = true
    sock.on("data", (d) => conn.push(d))
    for (const [c, cb] of queued.splice(0)) sock.write(c, cb)
    if (ended) sock.end()
  }
  Promise.resolve(which).then((p) => {
    if (gone) return
    if (!p) {
      sock = net.connect({ host, port })
      sock.once("connect", open)
      sock.on("end", () => conn.push(null))
      sock.on("error", (e) => conn.destroy(e))
      return
    }
    const pu = new URL(p)
    const ph = pu.hostname.replace(/^\[|\]$/g, "")
    const pp = Number(pu.port || (pu.protocol === "https:" ? 443 : 80))
    sock = pu.protocol === "https:" ? tls.connect({ host: ph, port: pp, servername: net.isIP(ph) ? undefined : ph }) : net.connect({ host: ph, port: pp })
    sock.once(pu.protocol === "https:" ? "secureConnect" : "connect", () => {
      let req = `CONNECT ${target} HTTP/1.1\r\nHost: ${target}\r\n`
      if (pu.username) {
        const cred = `${decodeURIComponent(pu.username)}:${decodeURIComponent(pu.password)}`
        req += `Proxy-Authorization: Basic ${Buffer.from(cred).toString("base64")}\r\n`
      }
      sock.write(req + "\r\n")
    })
    let got = Buffer.alloc(0)
    const head = (chunk) => {
      got = Buffer.concat([got, chunk])
      const end = got.indexOf("\r\n\r\n")
      if (end < 0) {
        if (got.length > 8192) conn.destroy(new Error(`proxyconnect tcp: ${ph}:${pp} answered CONNECT with no HTTP`))
        return
      }
      sock.off("data", head)
      const status = got.subarray(0, got.indexOf("\r\n")).toString("latin1").replace(/^HTTP\/\d(\.\d)?\s+/, "")
      if (!/^2\d\d/.test(status)) {
        const body = got.subarray(end + 4, end + 304).toString("utf8").trim()
        return conn.destroy(new Error(`proxyconnect tcp: ${body ? `${status}: ${body}` : status}`))
      }
      if (got.length > end + 4) conn.push(got.subarray(end + 4))
      open()
    }
    sock.on("data", head)
    sock.on("end", () => conn.push(null))
    sock.on("error", (e) => conn.destroy(ready ? e : new Error(`proxyconnect tcp: dial tcp ${ph}:${pp}: ${e?.message || e?.code || e}`)))
  }).catch((e) => conn.destroy(e))
  return conn
}

// ---- auth.json ---------------------------------------------------------------

// A crash can leave plugin-auth.json at its full length with every byte
// zero (#1505). Such a file is unknown, never "no sign-ins": it is read
// from its last good generation, plugin-auth.json.bak, which each write
// keeps first; a write over one that doesn't read copies it aside
// (plugin-auth.json.bad-<time>) first, and never writes none over it. As
// internal/lastgood does for magpie's own files.

// parseAuth is the sign-ins in text, null when it isn't one JSON object.
function parseAuth(text) {
  try {
    const v = JSON.parse(text)
    return v && typeof v === "object" && !Array.isArray(v) ? v : null
  } catch {
    return null
  }
}

// readFileSteady reads p; a read the OS refuses for a moment (Windows,
// while the file is being renamed over) is tried again at once and then
// thrown, never read as none: the next setAuth would write the others'
// accounts away. It never waits: the event loop is every plugin's (#5112),
// so a caller that can wait uses readFileSteadyAsync.
function readFileSteady(p) {
  for (let i = 0; ; i++) {
    try {
      return fs.readFileSync(p)
    } catch (e) {
      if (e?.code === "ENOENT") return null
      if (i >= 2) throw e
    }
  }
}

async function readFileSteadyAsync(p) {
  for (let i = 0; ; i++) {
    try {
      return fs.readFileSync(p)
    } catch (e) {
      if (e?.code === "ENOENT") return null
      if (i >= 50) throw e
      await sleep(10)
    }
  }
}

// describeBad says what is wrong with a file that doesn't read.
function describeBad(buf) {
  if (buf.length > 0 && buf.every((b) => b === 0)) return `${buf.length} bytes, every one zero`
  if (buf.toString("utf8").trim() === "") return "it is empty"
  return "it doesn't parse"
}

let authRecovered = false
let authUnread = false

// readAuthState is the sign-ins and whether they are known: a file not
// there is none, known; one that doesn't read is its .bak's, known, or
// none, unknown.
function readAuthState(buf = readFileSteady(authPath)) {
  return authStateOf(buf)
}

async function readAuthStateAsync() {
  return authStateOf(await readFileSteadyAsync(authPath))
}

function authStateOf(buf) {
  if (buf === null) return { all: {}, known: true }
  const v = parseAuth(buf.toString("utf8"))
  if (v) return { all: v, known: true, own: true }
  const why = describeBad(buf)
  let bak = null
  try {
    bak = parseAuth(fs.readFileSync(authPath + ".bak", "utf8"))
  } catch {}
  if (bak) {
    if (!authRecovered) {
      authRecovered = true
      toErr(`plugin-auth.json can't be read (${why}); the sign-ins in plugin-auth.json.bak are used`)
      send({ event: "recovered", said: why })
    }
    return { all: bak, known: true }
  }
  if (!authUnread) {
    authUnread = true
    toErr(`plugin-auth.json can't be read (${why}) and has no good backup`)
  }
  return { all: {}, known: false }
}

// authKnown is each sign-in as this host last read or wrote it: key → its
// JSON. Another magpie writes plugin-auth.json too: the window while the
// gateway runs as a service, or the other way round. A sign-in it saved
// again over an account this host had a loader for (settle: the same
// Google account signed in again with agy-cli's "Add another Google
// account", its home now in its metadata) was served by the loader made
// from the old one until the host restarted (mayne_wong on X: a gateway
// run as a LaunchDaemon kept answering 401 for an account the window had
// put right). So a sign-in that reads otherwise than this host left it is
// forgotten as one changed here is (forgetAccount), and magpie is told of
// an account added or removed (an "auth" event), which it lists again.
let authKnown = null // Map key → JSON

// noteAuth takes all, the sign-ins as just read (own false) or as this
// host is writing them (own true).
function noteAuth(all, own = false) {
  const now = new Map(Object.entries(all).map(([k, v]) => [k, JSON.stringify(v)]))
  const was = authKnown
  authKnown = now
  if (own || !was) return
  for (const k of new Set([...was.keys(), ...now.keys()])) {
    if (was.get(k) === now.get(k)) continue
    forgetAccount(k)
    if (!was.has(k) || !now.has(k)) send({ event: "auth", provider: providerOf(k), account: k })
  }
}

// authSeen is plugin-auth.json as last parsed, kept while the file has the
// same time, size and inode: every request read and parsed it two or three
// times (its account, its renewal, the plugin's getAuth), ~0.6 ms each for
// a dozen sign-ins. While the file's time is too recent for a second write
// to stamp a later one (Linux's clock is a tick coarse), its bytes are kept
// and compared too, as Go's filememo does.
let authSeen = null // {mtimeMs, size, ino, raw, all}
const AUTH_SETTLED = 2000

// readAuth is the sign-ins, shared: the caller must not change them, and
// hands a plugin a copy (authCopy).
function readAuth() {
  let st = null
  try {
    st = fs.statSync(authPath)
  } catch {}
  const e = authSeen
  const same = st && e && e.mtimeMs === st.mtimeMs && e.size === st.size && e.ino === st.ino
  if (same && !e.raw) return e.all
  let buf
  try {
    buf = readFileSteady(authPath)
  } catch (err) {
    // refused for a moment: the last good read stands, not none
    if (e) return e.all
    throw err
  }
  if (same && buf && buf.equals(e.raw)) {
    if (Date.now() - e.mtimeMs > AUTH_SETTLED) e.raw = null
    return e.all
  }
  const s = readAuthState(buf)
  if (s.known) noteAuth(s.all)
  authSeen = st && s.own ? { mtimeMs: st.mtimeMs, size: st.size, ino: st.ino, raw: Date.now() - st.mtimeMs > AUTH_SETTLED ? null : buf, all: s.all } : null
  return s.all
}

// authCopy is readAuth's sign-ins, or the one at key, the plugin's own to
// change.
function authCopy(key) {
  const all = readAuth()
  const v = key === undefined ? all : all[key]
  return v === undefined ? undefined : structuredClone(v)
}

// writeDurable replaces p with data through a temp file flushed to the
// disk before it is renamed over p: a rename reaches the disk on its own
// schedule, the data behind it later, and a machine that went down between
// the two left plugin-auth.json all zero (#1505). A write or flush that
// fails leaves p as it was.
async function writeDurable(p, data) {
  // named past the pid: another pod's host on the volume can have this one's
  const tmp = p + ".tmp-" + process.pid + "-" + Math.random().toString(36).slice(2, 10)
  try {
    const f = fs.openSync(tmp, "w", 0o600)
    try {
      for (let off = 0; off < data.length; ) off += fs.writeSync(f, data, off, data.length - off)
      fs.fsyncSync(f)
    } finally {
      fs.closeSync(f)
    }
    // and a file held open by a reader refuses to be renamed over
    for (let i = 0; ; i++) {
      try {
        return fs.renameSync(tmp, p)
      } catch (e) {
        if (i >= 50 || !["EPERM", "EACCES", "EBUSY"].includes(e?.code)) throw e
        await sleep(10)
      }
    }
  } catch (e) {
    try {
      fs.unlinkSync(tmp)
    } catch {}
    throw e
  }
}

// badStamp is the time a bad file is kept under, as Go's lastgood names it.
function badStamp(d = new Date()) {
  const p = (n) => String(n).padStart(2, "0")
  return `${d.getFullYear()}${p(d.getMonth() + 1)}${p(d.getDate())}-${p(d.getHours())}${p(d.getMinutes())}${p(d.getSeconds())}`
}

// keepBad copies buf, a plugin-auth.json that doesn't read, aside: two
// within one second are both kept (-2, …), the same bytes kept already
// not again.
function keepBad(buf) {
  const stamp = authPath + ".bad-" + badStamp()
  for (let i = 1; i < 1000; i++) {
    const name = i > 1 ? `${stamp}-${i}` : stamp
    let f
    try {
      f = fs.openSync(name, "wx", 0o600)
    } catch (e) {
      if (e?.code !== "EEXIST") throw e
      let old = null
      try {
        old = fs.readFileSync(name)
      } catch {}
      if (old && old.equals(buf)) return name
      continue
    }
    try {
      for (let off = 0; off < buf.length; ) off += fs.writeSync(f, buf, off, buf.length - off)
      fs.fsyncSync(f)
    } catch (e) {
      fs.closeSync(f)
      try {
        fs.unlinkSync(name)
      } catch {}
      throw e
    }
    fs.closeSync(f)
    toErr(`plugin-auth.json can't be read (${describeBad(buf)}); it is kept as ${path.basename(name)}`)
    return name
  }
  throw new Error("plugin-auth.json can't be read and there is no free name to keep it under")
}

// keepAuth is called before plugin-auth.json is replaced: the file there
// now becomes plugin-auth.json.bak when it reads, or is copied aside when
// it doesn't. A failure stops the write.
async function keepAuth() {
  const buf = await readFileSteadyAsync(authPath)
  if (buf === null) return
  if (!parseAuth(buf.toString("utf8"))) {
    keepBad(buf)
    return
  }
  const bak = authPath + ".bak"
  let old = null
  try {
    old = fs.readFileSync(bak)
  } catch {}
  if (!old || !old.equals(buf)) await writeDurable(bak, buf)
}

async function writeAuth(all) {
  fs.mkdirSync(path.dirname(authPath), { recursive: true })
  await keepAuth()
  await writeDurable(authPath, Buffer.from(JSON.stringify(all, null, 2) + "\n"))
}

// sleep waits ms without holding the event loop. Nothing in the host waits
// any other way: Atomics.wait here froze every plugin's requests, model
// lists and check-ins for as long as one of them spun on the lock (#5112).
const sleep = (ms) => new Promise((r) => setTimeout(r, ms))

// Two hosts run at once while one is restarted (the old one finishing its
// calls), and magpie writes the file when no host runs: each change to it
// is made under plugin-auth.json.lock, read afresh and written back with
// only its own accounts changed, so a host never writes another's newer
// token away with what it read before. The change itself is synchronous;
// the lock is waited for, and the write made, without holding the event
// loop. A lock held longer than AUTH_LOCK_STALE was left by a host that
// died. Changes within this host take the file lock one at a time
// (authQueue), so they don't poll against each other.
const AUTH_LOCK_STALE = 10 * 1000

async function lockAuth() {
  const lock = authPath + ".lock"
  fs.mkdirSync(path.dirname(authPath), { recursive: true })
  let tries = 0
  for (const start = Date.now(); ; ) {
    try {
      fs.closeSync(fs.openSync(lock, "wx", 0o600))
      return () => {
        try {
          fs.unlinkSync(lock)
        } catch {}
      }
    } catch (e) {
      if (e?.code !== "EEXIST" && e?.code !== "EPERM" && e?.code !== "EACCES") throw e
    }
    let old = false
    try {
      old = Date.now() - fs.statSync(lock).mtimeMs > AUTH_LOCK_STALE
    } catch {}
    if (old || Date.now() - start > 2 * AUTH_LOCK_STALE) {
      try {
        fs.unlinkSync(lock)
      } catch {}
    }
    await sleep(Math.min(50, 5 + 5 * tries++))
  }
}

// changeAuth runs change on the file as it is now, under the lock, and
// writes it back when change says it changed it.
let authQueue = Promise.resolve()
function changeAuth(change) {
  const run = authQueue.then(async () => {
    const unlock = await lockAuth()
    try {
      const { all, known } = await readAuthStateAsync()
      // what another magpie saved since this host's last read
      if (known) noteAuth(all)
      const out = change(all)
      // sign-ins that don't read are never written over with none: a
      // sign-out of one of them waits for the file to be put right
      if (out !== false && (known || Object.keys(all).length > 0)) {
        await writeAuth(all)
        noteAuth(all, true)
      }
      return out
    } finally {
      unlock()
    }
  })
  authQueue = run.then(
    () => {},
    () => {},
  )
  return run
}

async function setAuth(key, info) {
  await changeAuth((all) => {
    all[key] = info
  })
  forgetAccount(key)
  send({ event: "auth", provider: providerOf(key), account: key })
}

async function removeAuth(key) {
  await changeAuth((all) => {
    delete all[key]
  })
  forgetAccount(key)
  send({ event: "auth", provider: providerOf(key), account: key })
}

// ---- accounts ----------------------------------------------------------------

// scope is the account a request is for: {provider, key}.
const scope = new AsyncLocalStorage()

const providerOf = (key) => key.split("#")[0]

// accountsOf are the keys provider's accounts are kept under, the one under
// its own id first.
function accountsOf(all, provider) {
  return Object.keys(all)
    .filter((k) => k === provider || k.startsWith(provider + "#"))
    .sort((a, b) => (a === provider ? -1 : b === provider ? 1 : a < b ? -1 : a > b ? 1 : 0))
}

// keyFor is where the plugin's id is kept in this scope: the scope's
// account when the plugin names the scope's provider, else its own id.
function keyFor(id) {
  const s = scope.getStore()
  return s && s.provider === id ? s.key : id
}

// accountKey is the account a request names, the provider's first when it
// names none.
function accountKey(provider, account) {
  if (account && providerOf(account) === provider) return account
  return accountsOf(readAuth(), provider)[0] ?? provider
}

// freshKey is where a new sign-in to provider goes: its own id while that
// is free.
function freshKey(provider) {
  const all = readAuth()
  if (!(provider in all)) return provider
  for (;;) {
    const k = provider + "#" + Math.random().toString(36).slice(2, 8)
    if (!(k in all)) return k
  }
}

// inScope runs fn for provider's account key. had is whether the account
// was signed in as fn began: a hook's auth.set for one signed out while it
// ran (a models list spending a rotating token) doesn't sign it in again.
// A sign-in's steps (signIn) save whatever they are given.
function inScope(provider, key, fn, signIn = false) {
  let had = false
  if (!signIn) {
    try {
      had = key in readAuth()
    } catch {}
  }
  return scope.run({ provider, key, had }, fn)
}
// opens.js names the plugin whose program it refused by this
globalThis[Symbol.for("magpie.spawnedBy")] = () => scope.getStore()?.provider ?? ""

function whoOf(a) {
  return a?.accountId ?? a?.metadata?.email ?? a?.email ?? ""
}

// hintOf tells an account with no id from another: the end of its key.
function hintOf(a) {
  return a?.type === "api" && typeof a.key === "string" && a.key.length >= 12 ? a.key.slice(-4) : ""
}

function secretOf(a) {
  return a?.type === "oauth" ? a.refresh ?? a.access ?? "" : a?.key ?? ""
}

// uidOf is the vendor's id a plugin keeps beside an account's name
// (WorkBuddy's uid): two accounts of one name are told apart by it (#413).
function uidOf(a) {
  return typeof a?.uid === "string" ? a.uid : ""
}

// renamedBy tells a sign-in to an account a plugin once named another way:
// the same uid, and one of the two named by that uid, so the uid is the
// account's own id (#4473: officeace-auth 0.1.14 named an account by its
// masked phone, uid its Huawei domain id; the CodeArts plugin names it by
// the domain id). A uid alone is not enough: Kiro keeps how an account was
// signed in to as its uid, "google" for every Google sign-in.
function renamedBy(now, was) {
  const uid = uidOf(now)
  return !!uid && uidOf(was) === uid && whoOf(now) !== whoOf(was) && (whoOf(now) === uid || whoOf(was) === uid)
}

// settle keeps a sign-in just saved at key once: one to an account already
// signed in (the same account id and, where both have one, the same uid;
// the same uid that one of them is named by; else the same secret)
// replaces that one's and goes. It gives where it is kept. An account
// named another way before keeps its account id, which the user's name
// for it, its proxy and gateway keys are kept by.
async function settle(provider, key) {
  const k = await changeAuth((all) => {
    const now = all[key]
    if (!now) return false
    const who = whoOf(now)
    const secret = secretOf(now)
    for (const k of accountsOf(all, provider)) {
      if (k === key) continue
      const was = all[k]
      const other = uidOf(now) && uidOf(was) && uidOf(now) !== uidOf(was)
      const renamed = renamedBy(now, was)
      if ((who && whoOf(was) === who && !other) || renamed || (!who && !whoOf(was) && secret && secretOf(was) === secret)) {
        all[k] = renamed && typeof was.accountId === "string" && was.accountId ? { ...now, accountId: was.accountId } : now
        delete all[key]
        return k
      }
    }
    return false
  })
  if (k) {
    forgetAccount(k)
    forgetAccount(key)
    send({ event: "auth", provider, account: k })
    return k
  }
  return key
}

// ---- the client plugins are given --------------------------------------------

function stub(name) {
  return new Proxy(async () => ({ data: undefined }), {
    get(_, key) {
      if (key === "then") return undefined
      return stub(name + "." + String(key))
    },
    apply() {
      return Promise.resolve({ data: undefined })
    },
  })
}

function makeClient() {
  const known = {
    auth: {
      // OpenCode's SDK: auth.set({path: {id}, body})
      set: async (opts) => {
        const id = opts?.path?.id ?? opts?.id ?? opts?.providerID
        const body = opts?.body ?? opts?.auth
        if (typeof id === "string" && body && typeof body === "object") {
          // OpenCode keeps what the plugin gives it, saving refreshed
          // tokens over the old; nothing is merged. It goes to the
          // account the request is for.
          const key = keyFor(id)
          const s = scope.getStore()
          const had = s?.provider === id && s.had
          const kept = await changeAuth((all) => {
            // signed out while the hook ran: it stays signed out
            if (had && !(key in all)) return false
            if (JSON.stringify(all[key]) === JSON.stringify(body)) return false
            all[key] = body
          })
          if (kept !== false) {
            forgetAccount(key)
            send({ event: "auth", provider: providerOf(key), account: key })
          }
        }
        return { data: true }
      },
      remove: async (opts) => {
        const id = opts?.path?.id ?? opts?.id
        if (typeof id === "string") await removeAuth(keyFor(id))
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
    config: {
      get: async () => ({ data: config }),
    },
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

// ---- loading plugins ---------------------------------------------------------

const INDEX_FILES = ["index.ts", "index.tsx", "index.js", "index.mjs", "index.cjs"]

function readJSON(file) {
  try {
    return JSON.parse(fs.readFileSync(file, "utf8"))
  } catch {
    return undefined
  }
}

// entries are the files a plugin's server side may load from, as
// OpenCode looks for them: the package's exports["./server"], its main,
// exports["."], else an index file; a path to a file is that file. A
// package whose ./server is written for OpenCode's next plugin API
// (Plugin.define, opencode-gemini-auth 2) keeps its v1 plugin at main.
function entries(target) {
  const stat = fs.statSync(target, { throwIfNoEntry: false })
  if (!stat) throw new Error(`no such plugin: ${target}`)
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
    // a command alone (bin), an MCP server, is no plugin at all (#1327)
    const cmd = pkg?.bin ? ", only a command (bin). If it is an MCP server, add it under Library › MCP servers instead" : ""
    // said without a stack: there is no fault in the host to trace
    throw Object.assign(new Error(`${pkg?.name || target} isn't an OpenCode or magpie plugin: its package names no file to load (no main or exports in package.json, no index file)${cmd}`), { plain: true })
  }
  return [...new Set(out)]
}

// servers are the plugin functions a module exports: default {id?,
// server} (v1), else every function it exports, or {server} objects
// (legacy), each once.
function servers(mod) {
  const d = mod.default
  if (d && typeof d === "object" && ("server" in d || "id" in d || "tui" in d)) {
    return typeof d.server === "function" ? [d.server] : []
  }
  const seen = new Set()
  const out = []
  for (const v of Object.values(mod)) {
    if (seen.has(v)) continue
    seen.add(v)
    if (typeof v === "function") out.push(v)
    else if (v && typeof v === "object" && typeof v.server === "function") out.push(v.server)
  }
  return out
}

async function loadPlugins(list) {
  const input = {
    client: makeClient(),
    project: { id: "magpie", worktree: directory, vcs: undefined, time: { created: Date.now() } },
    directory,
    worktree: directory,
    experimental_workspace: { register() {} },
    serverUrl: new URL("http://127.0.0.1:4096"),
    $: Bun.$,
    // magpie's own (OpenCode has none): the plugin's options as the user
    // set them in magpie, read when wanted, so a change reaches the
    // plugin without a restart (#2038)
    magpie: { option: optionValue, options: optionValues, load: reportLoad, usage: reportUsage, account: keyFor },
  }
  // pi's packages and extensions are loaded as pi loads them (pi.js)
  let pi
  if (piPath) {
    try {
      pi = await import(pathToFileURL(piPath).href)
    } catch (e) {
      toErr("pi.js:", e)
    }
  }
  const pis = pi ? list.filter((p) => pi.isPi(p.target)) : []
  if (pis.length) {
    const h = { readAuth: authCopy, changeAuth, keyFor, send, directory }
    const got = await pi.load(h, pis).catch((e) => pis.map((p) => ({ spec: p.spec, error: String(e?.stack ?? e) })))
    for (const r of got) {
      for (const x of r.hooks ?? []) hooks.push(x)
      loaded.push(r.error ? { spec: r.spec, error: r.error } : { spec: r.spec })
    }
  }
  // each plugin loads by itself, side by side, so one that is slow or
  // stuck holds up none of the others; what they add is kept in the
  // list's order all the same, as a later plugin's hooks win
  const one = async (p, isPi) => {
    let gateway
    if (p.gateway) {
      gateway = await import(pathToFileURL(p.gateway).href)
      // a hook that only listens for sign-ins needs no onRequest
      if (typeof gateway.onRequest !== "function" && typeof gateway.onSignIn !== "function") throw new Error("magpie.hooks exports no onRequest")
    }
    let fns = []
    // A gateway-only package needs no OpenCode entry. Mixed packages
    // still load their provider hooks through the usual entry rules.
    const pkg = readJSON(path.join(p.target, "package.json"))
    const onlyGateway = isPi || (p.gateway && (p.gateway === p.target || (!pkg?.main && !pkg?.exports && !INDEX_FILES.some((f) => fs.existsSync(path.join(p.target, f))))))
    for (const file of onlyGateway ? [] : entries(p.target)) {
      fns = servers(await import(pathToFileURL(file).href))
      if (fns.length) break
    }
    if (fns.length === 0 && !gateway) throw new Error("exports no OpenCode plugin function")
    const got = []
    for (const fn of fns) got.push({ spec: p.spec, target: p.target, hooks: (await fn(input, p.options)) ?? {} })
    return { got, gateway }
  }
  const results = await Promise.all(list.map((p) => {
    const isPi = pis.includes(p)
    if (isPi && !p.gateway) return null
    return inTime(() => one(p, isPi), loadWait, "the plugin's load").then((r) => ({ p, isPi, ...r }), (e) => ({ p, isPi, e }))
  }))
  for (const r of results) {
    if (!r) continue
    const { p, isPi } = r
    if (r.e) {
      const e = r.e
      const error = e?.plain ? e.message : String(e?.stack ?? e)
      if (isPi) loaded.find((x) => x.spec === p.spec).error = error
      else loaded.push({ spec: p.spec, error })
      continue
    }
    for (const x of r.got) hooks.push(x)
    if (r.gateway) gatewayHooks.push({ spec: p.spec, options: p.options ?? {}, onRequest: r.gateway.onRequest,
      onResponse: r.gateway.onResponse, onSignIn: r.gateway.onSignIn,
      state: { calls: 0, failures: 0, totalMillis: 0, lastOutcome: "idle", lastBytes: 0 } })
    if (!isPi) loaded.push({ spec: p.spec })
  }
  config = { provider: {}, ...structuredClone(userConfig) }
  ownConfig = new Map()
  for (const h of hooks) {
    if (typeof h.hooks.config !== "function") continue
    // a provider the plugin signs in to is built from the user's own
    // entry, or what this plugin made of it before, never from another
    // plugin's that signs in to it too: a third party's that set it
    // first (models: {}) left the other's `??=` nothing to add, and the
    // provider no models (豆包工作, two plugins of doubao-work)
    const ids = signsIn(h.spec)
    for (const id of ids) {
      const own = ownConfig.get(ownKey(h.spec, id)) ?? structuredClone(userConfig.provider?.[id])
      if (own === undefined) delete config.provider[id]
      else config.provider[id] = own
    }
    try {
      // one at a time, as each may build on the last, but none for long
      await inTime(() => h.hooks.config(config), loadWait, "config hook")
    } catch (e) {
      send({ event: "log", level: "error", message: `${h.spec}: config hook: ${e?.message ?? e}` })
    }
    for (const id of ids) ownConfig.set(ownKey(h.spec, id), config.provider?.[id])
  }
}

// ownConfig is each provider's config entry as the config hook of each
// plugin that signs in to it made it, by ownKey(spec, id). The plugin
// serving the provider (auths) is the one whose entry counts.
let ownConfig = new Map()
const ownKey = (spec, id) => spec + "\0" + id

// providerConfig is the config entry of provider id: the serving plugin's
// own, or what the config hooks made of it when no plugin signs in to it.
function providerConfig(id) {
  const server = auths().get(id)?.spec
  const k = server && ownKey(server, id)
  return k && ownConfig.has(k) ? ownConfig.get(k) : config.provider?.[id]
}

// A bounded await detaches its listener even when the plugin ignores the
// signal. The rejected promise is observed, so late failures cannot escape.
function bounded(promise, signal) {
  return new Promise((resolve, reject) => {
    const finish = (fn, value) => { signal.removeEventListener("abort", abort); fn(value) }
    const abort = () => finish(reject, new DOMException("Aborted", "AbortError"))
    // Observe the task even if cancellation won before the await began.
    Promise.resolve(promise).then((value) => finish(resolve, value), (error) => finish(reject, error))
    if (signal.aborted) abort()
    else signal.addEventListener("abort", abort, { once: true })
  })
}

// A run owns the private state of its participating plugins. Its token prevents
// a stale or malformed RPC from selecting another request's tool handlers.
const gatewayRuns = new Map()
const GATEWAY_RUN_LIMIT = 128
const GATEWAY_RUN_TTL = 15 * 60 * 1000
// A hook's own timeout: gatewayTimeoutMs, an integer clamped to 100–30000
// ms (a slow deep recall may take 20s, #5890), or 1000 when it isn't one.
// Keep gatewayTimeout in gateway.go the same.
const GATEWAY_HOOK_LEAST = 100, GATEWAY_HOOK_MOST = 30000
const gatewayTimeout = (h) => Number.isInteger(h.options.gatewayTimeoutMs) ? Math.min(GATEWAY_HOOK_MOST, Math.max(GATEWAY_HOOK_LEAST, h.options.gatewayTimeoutMs)) : 1000
// The chain's budget: the hooks' timeouts summed within 2–10s, as before,
// or the longest one's own timeout when that is longer, so a hook given
// more than 10s gets it (at most 30s). gatewayBudget in gateway.go mirrors it.
const gatewayBudget = () => Math.max(Math.min(10000, Math.max(2000, gatewayHooks.reduce((n, h) => n + gatewayTimeout(h), 0))), ...gatewayHooks.map(gatewayTimeout))

function closeGateway(run) {
  if (!run || run.closed) return
  run.closed = true
  gatewayRuns.delete(run.id)
  clearTimeout(run.timer)
  run.controller.abort()
  run.plugins.length = 0
}

async function gatewayBegin(p, signal) {
  if (gatewayRuns.size >= GATEWAY_RUN_LIMIT) return null
  const run = { id: randomUUID(), token: randomUUID(), plugins: [], controller: new AbortController(), closed: false, finished: false }
  gatewayRuns.set(run.id, run)
  run.timer = setTimeout(() => closeGateway(run), GATEWAY_RUN_TTL)
  run.timer.unref()
  let body = p.body
  // The total budget bounds many installed hooks as well as one slow hook.
  const total = AbortSignal.any([signal, run.controller.signal, AbortSignal.timeout(gatewayBudget())])
  for (const h of gatewayHooks) {
    if (total.aborted) break
    if (typeof h.onRequest !== "function") continue
    const ctl = new AbortController()
    const timer = setTimeout(() => ctl.abort(), gatewayTimeout(h))
    const ownSignal = AbortSignal.any([total, ctl.signal])
    const started = performance.now()
    let outcome = "skipped", bytes = 0
    try {
      // Diagnostics accept fixed codes and byte counts only. Never retain
      // exception messages, queries, scope IDs, credentials or recalled text.
      const report = (code, count = 0) => {
        if (ownSignal.aborted || !["ready", "empty", "unbound", "unauthorized", "unavailable", "invalid", "skipped"].includes(code)) return
        outcome = code
        bytes = Number.isSafeInteger(count) && count >= 0 && count <= 32768 ? count : 0
      }
      // each hook its own copy: ctx.caller is an object, and one hook's
      // change to it reaches no other hook
      const ctx = { ...structuredClone(p.context), options: structuredClone(h.options), state: {}, signal: ownSignal, report }
      const result = await bounded(Promise.resolve().then(() => h.onRequest(structuredClone(body), ctx)), ownSignal)
      let next = body
      if (result !== undefined) {
        if (!result || typeof result !== "object" || Array.isArray(result)) throw new Error("invalid body")
        // Validate serialization before accepting a plugin's result.
        next = JSON.parse(JSON.stringify(result))
        if (!next || typeof next !== "object" || Array.isArray(next)) throw new Error("invalid serialized body")
        if (outcome === "skipped") outcome = "ready"
      }
      // Context hooks cannot register or change client tools or tool choice,
      // nor the model or streaming: the gateway checked the caller's key
      // against the model, and the client reads the reply it asked for.
      for (const key of ["tools", "tool_choice", "function_call", "functions", "toolConfig", "model", "stream"]) {
        if (JSON.stringify(next[key]) !== JSON.stringify(body[key])) throw new Error("context hook changed client tools, model or streaming")
      }
      if (JSON.stringify(next.config?.tools) !== JSON.stringify(body.config?.tools) || JSON.stringify(next.config?.toolConfig) !== JSON.stringify(body.config?.toolConfig)) throw new Error("context hook changed client tools")
      body = next
      run.plugins.push({ h, ctx })
    } catch {
      if (ownSignal.aborted) outcome = signal.aborted ? "cancelled" : "timeout"
      else if (!["unbound", "unauthorized", "unavailable", "invalid"].includes(outcome)) outcome = "error"
      // Exception text may contain tokens or recalled private material.
      if (!signal.aborted) send({ event: "log", level: "warn", message: `${h.spec}: gateway request hook failed or timed out; keeping request` })
    } finally {
      h.state.calls++
      if (["timeout", "error", "unauthorized", "unavailable", "invalid"].includes(outcome)) h.state.failures++
      h.state.totalMillis += performance.now() - started
      h.state.lastOutcome = outcome
      h.state.lastBytes = outcome === "ready" ? bytes : 0
      clearTimeout(timer)
      ctl.abort()
    }
  }
  if (signal.aborted || run.closed) {
    closeGateway(run)
    signal.throwIfAborted()
    return null
  }
  return { id: run.id, token: run.token, body }
}

async function gatewayFinish(run, p, signal) {
  if (run.finished) return null
  run.finished = true
  const total = AbortSignal.any([signal, run.controller.signal, AbortSignal.timeout(gatewayBudget())])
  try {
    for (const { h, ctx } of run.plugins) {
      if (total.aborted) break
      if (typeof h.onResponse !== "function") continue
      const ctl = new AbortController()
      const timer = setTimeout(() => ctl.abort(), gatewayTimeout(h))
      ctx.signal = AbortSignal.any([total, ctl.signal])
      ctx.success = p.success === true
      if (p.request !== undefined) ctx.request = structuredClone(p.request)
      try {
        await bounded(Promise.resolve().then(() => h.onResponse(structuredClone(p.reply), ctx)), ctx.signal)
      } catch {
        if (!signal.aborted) send({ event: "log", level: "warn", message: `${h.spec}: gateway response hook failed or timed out` })
      } finally {
        clearTimeout(timer)
        ctl.abort()
      }
    }
  } finally {
    closeGateway(run)
  }
  return null
}

// An account that has to be signed in again, or whose sign-in ends soon,
// told to each hook's onSignIn once, as magpie's own alert is. The event
// holds the provider, the account's label and the state, never a token.
async function gatewaySignIn(p, signal) {
  const ev = p.event
  if (!ev || typeof ev !== "object") return null
  const total = AbortSignal.any([signal, AbortSignal.timeout(gatewayBudget())])
  for (const h of gatewayHooks) {
    if (total.aborted) break
    if (typeof h.onSignIn !== "function") continue
    const ctl = new AbortController()
    const timer = setTimeout(() => ctl.abort(), gatewayTimeout(h))
    const ctx = { options: structuredClone(h.options), signal: AbortSignal.any([total, ctl.signal]) }
    try {
      await bounded(Promise.resolve().then(() => h.onSignIn(structuredClone(ev), ctx)), ctx.signal)
    } catch {
      if (!signal.aborted) send({ event: "log", level: "warn", message: `${h.spec}: gateway sign-in hook failed or timed out` })
    } finally {
      clearTimeout(timer)
      ctl.abort()
    }
  }
  return null
}

async function gatewayRequest(p, signal) {
  if (p.operation === "begin") return gatewayBegin(p, signal)
  if (p.operation === "signIn") return gatewaySignIn(p, signal)
  const run = gatewayRuns.get(p.id)
  if (!run || run.closed || run.token !== p.token) return null
  if (p.operation === "close") { closeGateway(run); return null }
  if (p.operation === "finish") return gatewayFinish(run, p, signal)
  return null
}

// auths are the auth hooks by provider, the last plugin to name one
// winning, as in OpenCode, unless the user picked one of them for it
// (prefer): a plugin of their own beside a third party's that signs in to
// the same provider id is then theirs to choose, not the list's order.
function auths() {
  const m = new Map()
  for (const h of hooks) if (h.hooks.auth?.provider) m.set(h.hooks.auth.provider, { spec: h.spec, target: h.target, auth: h.hooks.auth })
  for (const h of hooks) {
    const id = h.hooks.auth?.provider
    if (id && prefer[id] === h.spec) m.set(id, { spec: h.spec, target: h.target, auth: h.hooks.auth })
  }
  return m
}

// signsIn are the provider ids the plugin spec's auth hooks name.
const signsIn = (spec) => [...new Set(hooks.filter((h) => h.spec === spec && h.hooks.auth?.provider).map((h) => h.hooks.auth.provider))]

// withProviders is a loaded plugin as init tells magpie of it: the provider
// ids it signs in to, and for each one another plugin serves (two plugins
// naming one id: the host runs one), the plugin that does. magpie says so
// on the plugin's row rather than leave its provider missing. And the
// tools it gives (toolList).
function withProviders(l) {
  const tools = l.error ? [] : toolList(l.spec)
  if (tools.length) l = { ...l, tools }
  const searches = l.error ? [] : searchList(l.spec)
  if (searches.length) l = { ...l, searches }
  const ids = l.error ? [] : signsIn(l.spec)
  if (!ids.length) return l
  const a = auths()
  const servedBy = Object.fromEntries(ids.filter((id) => a.get(id) && a.get(id).spec !== l.spec).map((id) => [id, a.get(id).spec]))
  return { ...l, provides: ids, ...(Object.keys(servedBy).length ? { servedBy } : {}) }
}

// ---- tools ----------------------------------------------------------------
//
// A plugin's tools are OpenCode's tool hook: {tool: {name: {description,
// args, execute(args, ctx)}}}, args a zod shape (OpenCode's tool.schema is
// zod) or a JSON Schema. magpie offers them to every agent through its
// plugin tools MCP server (internal/gateway/tools_mcp.go), so each is told
// to magpie by its name, description and a JSON Schema of its arguments.

const TOOL_NAME = /^[A-Za-z0-9_-]{1,48}$/

// toolsOf are the tools the plugin spec's hooks give, the last of a name
// winning, as in OpenCode.
function toolsOf(spec) {
  const m = new Map()
  for (const h of hooks) {
    if (h.spec !== spec) continue
    const t = h.hooks.tool
    if (!t || typeof t !== "object") continue
    for (const [name, def] of Object.entries(t)) {
      if (TOOL_NAME.test(name) && def && typeof def.execute === "function") m.set(name, def)
    }
  }
  return m
}

// jsonSchema is a zod schema (v3 or v4) as JSON Schema, or a JSON Schema
// as it is. What it can't read is {}: any value.
function jsonSchema(z, depth = 0) {
  if (!z || typeof z !== "object" || depth > 16) return {}
  const desc = (o) => {
    const d = typeof z.description === "string" ? z.description : z._def?.description
    if (d) o.description = d
    return o
  }
  const v4 = z._zod?.def
  const v3 = z._def
  if (!v4 && !v3) return z
  const d = v4 ?? v3
  const kind = v4 ? d.type : String(d.typeName ?? "").replace(/^Zod/, "").toLowerCase()
  const inner = (x) => jsonSchema(x, depth + 1)
  switch (kind) {
    case "string": return desc({ type: "string" })
    case "number": return desc({ type: d.format === "safeint" || d.checks?.some?.((c) => c.kind === "int" || c._zod?.def?.format === "safeint") ? "integer" : "number" })
    case "int": case "bigint": return desc({ type: "integer" })
    case "boolean": return desc({ type: "boolean" })
    case "null": return desc({ type: "null" })
    case "enum": return desc({ type: "string", enum: v4 ? Object.values(d.entries ?? {}) : [...(d.values ?? [])] })
    case "literal": {
      const vals = v4 ? d.values ?? [] : [d.value]
      return desc(vals.length === 1 ? { const: vals[0] } : { enum: vals })
    }
    case "array": return desc({ type: "array", items: inner(v4 ? d.element : d.type) })
    case "object": return desc(objectSchema(typeof d.shape === "function" ? d.shape() : d.shape, depth + 1))
    case "optional": case "nullable": case "default": case "readonly": case "catch": case "prefault": {
      const o = { ...inner(d.innerType), ...desc({}) }
      if (kind === "default") try { o.default = typeof d.defaultValue === "function" ? d.defaultValue() : d.defaultValue } catch {}
      return o
    }
    case "union": return desc({ anyOf: (d.options ?? []).map(inner) })
    case "record": return desc({ type: "object", additionalProperties: inner(d.valueType) })
    default: return desc({})
  }
}

// optionalIn is whether a zod field may be left out.
function optionalIn(z) {
  const t = z?._zod?.def?.type ?? String(z?._def?.typeName ?? "").replace(/^Zod/, "").toLowerCase()
  if (z?._zod?.optin === "optional") return true
  return t === "optional" || t === "default"
}

function objectSchema(shape, depth = 0) {
  const properties = {}
  const required = []
  for (const [k, v] of Object.entries(shape ?? {})) {
    properties[k] = jsonSchema(v, depth)
    if (!optionalIn(v)) required.push(k)
  }
  return { type: "object", properties, ...(required.length ? { required } : {}) }
}

// argsSchema is a tool's arguments as JSON Schema: its args a zod shape, a
// zod object, or a JSON Schema object.
function argsSchema(args) {
  if (!args || typeof args !== "object") return { type: "object", properties: {} }
  if (args._zod || args._def) return jsonSchema(args)
  if (typeof args.type === "string" || args.properties) return args
  return objectSchema(args)
}

function toolList(spec) {
  const out = []
  for (const [name, def] of toolsOf(spec)) {
    let schema
    try {
      schema = argsSchema(def.args)
    } catch {
      schema = { type: "object", properties: {} }
    }
    out.push({ name, description: shown(def.description, 2000), schema })
  }
  return out
}

// parsedArgs are args as the tool's zod schema parses them, as OpenCode
// does before execute: defaults filled in, a wrong argument refused. A
// JSON Schema is the tool's own to check.
function parsedArgs(name, schema, args) {
  args = args && typeof args === "object" && !Array.isArray(args) ? args : {}
  const fail = (r) => {
    const issues = r.error?.issues ?? []
    const why = issues.map((i) => `${(i.path ?? []).join(".") || "arguments"}: ${i.message}`).join("; ")
    throw new Error(`${name}'s arguments are wrong: ${why || r.error?.message || "refused"}`)
  }
  if (!schema || typeof schema !== "object") return args
  if (typeof schema.safeParse === "function") {
    const r = schema.safeParse(args)
    if (!r.success) fail(r)
    return r.data
  }
  if (schema._zod || schema._def || typeof schema.type === "string" || schema.properties) return args
  const out = { ...args }
  for (const [k, z] of Object.entries(schema)) {
    if (typeof z?.safeParse !== "function") continue
    const r = z.safeParse(args[k])
    if (!r.success) fail({ error: { issues: (r.error?.issues ?? [{ message: r.error?.message }]).map((i) => ({ ...i, path: [k, ...(i.path ?? [])] })) } })
    if (r.data === undefined) delete out[k]
    else out[k] = r.data
  }
  return out
}

// runTool runs the plugin spec's tool name with args, within ms: its
// answer as text (a string as it is, else JSON).
// took is ms as Go prints a duration: 300ms, 55s.
const took = (ms) => (ms % 1000 ? `${ms}ms` : `${ms / 1000}s`)

async function runTool({ spec, name, args, ms, agent }) {
  const def = toolsOf(spec).get(name)
  if (!def) throw new Error(`${spec} has no tool ${name}`)
  args = parsedArgs(name, def.args, args)
  const abort = AbortSignal.timeout(ms > 0 ? ms : 60_000)
  const ctx = {
    sessionID: "", messageID: "", callID: "", agent: agent ?? "",
    directory, worktree: directory, abort,
    metadata() {},
  }
  const out = await Promise.race([
    Promise.resolve().then(() => def.execute(args, ctx)),
    new Promise((_, reject) => abort.addEventListener("abort", () => reject(new Error(`${name} took longer than ${took(ms || 60_000)}`)), { once: true })),
  ])
  if (typeof out === "string") return { text: out }
  if (out && typeof out === "object" && typeof out.output === "string") return { text: out.output }
  return { text: out === undefined ? "" : JSON.stringify(out) }
}

// ---- web search APIs --------------------------------------------------------
//
// A plugin's search hook adds web search APIs to magpie's own list (Tavily,
// Brave, …): {search: {<id>: {name, keysURL?, keyOptional?, url?, execute}}}.
// execute({query, count, key, url}, ctx) returns the pages found:
// [{title, url, text?, age?}] (or {results: [...]}). A throw with a numeric
// status (401, 402, 403, 429…) is the API refusing the key, which magpie
// rests as it does a built-in's; retryAfter is its Retry-After.
// magpie keeps the keys, order and limits, and masks the query before it.
//
// An entry may name auth: "<provider id>", one this plugin's auth hook
// signs in to (a model provider, or an auth hook with no models of its
// own). It then takes no key: magpie tries the provider's accounts that
// are on, in their order, one per call, and the hook reads the one chosen
// through ctx.getAuth(), renewed first if due (the wait counts within the
// search's time). A refusal rests the account for search only, never for
// its models.

const SEARCH_ID = /^[a-z0-9][a-z0-9-]{0,31}$/

function searchesOf(spec) {
  const m = new Map()
  for (const h of hooks) {
    if (h.spec !== spec) continue
    const t = h.hooks.search
    if (!t || typeof t !== "object") continue
    for (const [id, def] of Object.entries(t)) {
      if (SEARCH_ID.test(id) && def && typeof def.execute === "function") m.set(id, def)
    }
  }
  return m
}

function searchList(spec) {
  const out = []
  for (const [id, def] of searchesOf(spec)) {
    out.push({
      id,
      name: typeof def.name === "string" && def.name ? def.name : id,
      ...(typeof def.keysURL === "string" && /^https?:\/\//.test(def.keysURL) ? { keysURL: def.keysURL } : {}),
      ...(def.keyOptional === true ? { keyOptional: true } : {}),
      ...(def.url === true ? { needURL: true } : {}),
      ...(searchAuthOf(spec, def) ? { auth: def.auth } : {}),
    })
  }
  return out
}

// searchAuthOf is the provider a search entry signs in as: one the same
// plugin's auth hooks name, "" for none (an id it doesn't sign in to is
// none, so the entry stays a key's).
function searchAuthOf(spec, def) {
  return typeof def.auth === "string" && def.auth && signsIn(spec).includes(def.auth) ? def.auth : ""
}

async function runSearch({ spec, id, query, count, key, url, ms, account, proxy }) {
  const def = searchesOf(spec).get(id)
  if (!def) throw new Error(`${spec} has no search API ${id}`)
  const abort = AbortSignal.timeout(ms > 0 ? ms : 60_000)
  const late = new Promise((_, reject) => abort.addEventListener("abort", () => reject(new Error("no answer in time")), { once: true }))
  late.catch(() => {})
  const ctx = { directory, worktree: directory, abort }
  const provider = searchAuthOf(spec, def)
  let run = () => def.execute({ query, count, key: key ?? "", url: url ?? "" }, ctx)
  if (provider) {
    // the account magpie chose, and that one only: never the provider's
    // first in its place, nor one of another provider
    if (auths().get(provider)?.spec !== spec) throw new Error(`another plugin signs in to ${provider}`)
    const k = typeof account === "string" && account && providerOf(account) === provider ? account : ""
    if (!k || !(k in readAuth())) throw new Error(`not signed in to ${provider}`)
    ctx.getAuth = async () => authCopy(k)
    const execute = run
    run = () =>
      via.run(proxy ?? "", () =>
        inScope(provider, k, async () => {
          await Promise.race([fresh(provider, k), late])
          if (!(k in readAuth())) throw new Error(`not signed in to ${provider}`)
          return execute()
        }),
      )
  }
  let out
  try {
    out = await Promise.race([Promise.resolve().then(run), late])
  } catch (e) {
    const status = Number(e?.status)
    if (Number.isInteger(status) && status >= 300 && status < 600) {
      return { status, error: String(e?.message ?? e), retryAfter: e?.retryAfter == null ? "" : String(e.retryAfter) }
    }
    throw e
  }
  const rs = Array.isArray(out) ? out : Array.isArray(out?.results) ? out.results : null
  if (!rs) throw new Error("the search returned neither a list nor {results}")
  const str = (v) => (typeof v === "string" ? v : v == null ? "" : String(v))
  return {
    pages: rs.filter((r) => r && typeof r === "object").map((r) => ({
      title: str(r.title), url: str(r.url), text: str(r.text ?? r.content ?? r.snippet), age: str(r.age ?? r.date),
    })),
  }
}

function authOf(provider) {
  const a = auths().get(provider)
  if (!a) throw new Error(`no plugin signs in to ${provider}`)
  return a.auth
}

// ---- providers and models ----------------------------------------------------

let modelsDev
let modelsDevMtime = 0
function mdev() {
  if (modelsDevPath) {
    try {
      const st = fs.statSync(modelsDevPath, { throwIfNoEntry: false })
      const mtime = st ? st.mtimeMs : 0
      if (modelsDev === undefined || (mtime > 0 && mtime !== modelsDevMtime) || (Object.keys(modelsDev).length === 0 && mtime > 0)) {
        // a file caught half written (an older magpie's catalog.Sync wrote
        // it in place; it goes through a rename now)
        // reads as unknown: keep what was read before, rather than list the
        // providers without models.dev, and read it again next time
        const md = readJSON(modelsDevPath)
        if (md) {
          modelsDev = md
          modelsDevMtime = mtime
        } else if (modelsDev === undefined) modelsDev = {}
      }
    } catch {
      if (modelsDev === undefined) modelsDev = {}
    }
  } else if (modelsDev === undefined) {
    modelsDev = {}
  }
  return modelsDev
}

function cost(c) {
  return {
    input: c?.input ?? 0,
    output: c?.output ?? 0,
    cache: { read: c?.cache_read ?? c?.cache?.read ?? 0, write: c?.cache_write ?? c?.cache?.write ?? 0 },
  }
}

// MODALITIES are the kinds of input and output OpenCode's capabilities
// name, in its order.
const MODALITIES = ["text", "image", "audio", "video", "pdf"]

// modalities is a models.dev or config modalities list as capabilities
// has it: each kind true or false, text when it lists none.
const modalities = (list) => Object.fromEntries(MODALITIES.map((k) => [k, (list ?? ["text"]).includes(k)]))

// kinds are the kinds a capabilities.input or .output says yes to, for
// magpie: undefined when the plugin gave none.
function kinds(c) {
  if (!c || typeof c !== "object") return undefined
  const out = Object.keys(c).filter((k) => c[k] === true)
  return out.length ? out : undefined
}

// kindsSaid is whether a capabilities.input or .output says yes or no to a kind
// at all: one that says no to each (an embedding's {"text": false}, #3279)
// gives no kinds, and is not one that said nothing.
const kindsSaid = (c) => !!c && typeof c === "object" && MODALITIES.some((k) => typeof c[k] === "boolean")

// inputSaid is whether a capabilities.input is someone's word on what the
// model takes: the host's default for a config model said nothing of,
// {"text": true} alone, is nobody's word (#1595). Whoever says the kinds
// says them each — models.dev and the config expand them all, and a
// plugin's models hook gives its own (#4436).
const inputSaid = (c) => kindsSaid(c) && (Object.keys(c).length > 1 || c.text !== true)

// fromModelsDev is a models.dev model as OpenCode's provider list has it.
// temperature is what models.dev itself said of it, left out where it said
// nothing: capabilities.temperature is OpenCode's own, and its false is
// what OpenCode reads a model.dev silence as, not a refusal said.
function fromModelsDev(p, m) {
  return {
    id: m.id,
    providerID: p.id,
    name: m.name ?? m.id,
    family: m.family,
    api: { id: m.id, url: m.provider?.api ?? p.api ?? "", npm: m.provider?.npm ?? p.npm ?? "@ai-sdk/openai-compatible" },
    status: m.status ?? "active",
    headers: {},
    options: {},
    cost: cost(m.cost),
    limit: { context: m.limit?.context ?? 0, input: m.limit?.input, output: m.limit?.output ?? 0 },
    ...(m.temperature !== undefined ? { temperature: m.temperature } : {}),
    capabilities: {
      temperature: m.temperature ?? false,
      reasoning: m.reasoning ?? false,
      attachment: m.attachment ?? false,
      toolcall: m.tool_call ?? true,
      input: { ...modalities(m.modalities?.input), text: true },
      output: modalities(m.modalities?.output),
      interleaved: m.interleaved ?? false,
    },
    release_date: m.release_date ?? "",
    variants: {},
  }
}

// info is the provider as OpenCode builds it: models.dev's entry, what
// the config (with the plugins' config hooks) says of it, the plugin's
// provider.models hook.
// info is what OpenCode knows of provider id, its models as the plugin
// lists them for account key (the first when none); strict fails where the
// plugin's list does, rather than keeping the list it was given.
// quiet asks no models hook (a provider switched off in magpie): the
// list is told as fallen back, which keeps the one magpie had. The
// listing's caller may give up (signal), which a hook is told through
// its own options' signal too (#4678).
// refusedLists is each account whose models hook said its vendor refused
// the sign-in (signIn "expired"): {sign, at, n, error}. Its list isn't
// asked again for that sign-in until retryAfter(n) has passed, as a
// renewal that failed isn't: two dead Zed sign-ins were listed, and
// refused by Zed, ~30 times a minute (JZGiVvkSOA75376 on X). Signing in
// again (another sign-in) asks at once.
const refusedLists = new Map()

// signInOf is what tells one sign-in of an account from the next.
const signInOf = (a) => JSON.stringify([a?.access ?? "", a?.refresh ?? "", a?.key ?? ""])

async function info(id, key, strict, limit, quiet, signal) {
  const md = mdev()[id]
  const cfg = providerConfig(id)
  const out = {
    id,
    name: cfg?.name ?? md?.name ?? id,
    source: md ? "api" : "config",
    env: cfg?.env ?? md?.env ?? [],
    options: { ...(cfg?.options ?? {}) },
    npm: cfg?.npm ?? md?.npm,
    api: cfg?.api ?? md?.api,
    // magpie's own field: the plugin serves a decision API (System One)
    // for this provider, asked at its base /systemone through its fetch
    decide: cfg?.decide === true,
    // magpie's own field: the provider's upstream answers a web search
    // tool offered on its Anthropic or Responses API by itself, so a
    // client's web_search goes to it as sent (#2027)
    searches: cfg?.searches === true,
    models: {},
  }
  if (md) for (const m of Object.values(md.models ?? {})) out.models[m.id] = fromModelsDev(md, m)
  for (const [key, m] of Object.entries(cfg?.models ?? {})) {
    const was = out.models[m.id ?? key]
    const npm = m.provider?.npm ?? cfg.npm ?? was?.api.npm ?? md?.npm ?? "@ai-sdk/openai-compatible"
    out.models[key] = {
      ...(was ?? {}),
      id: key,
      providerID: id,
      name: m.name ?? was?.name ?? key,
      api: { id: m.id ?? was?.api.id ?? key, url: m.provider?.api ?? cfg.api ?? was?.api.url ?? md?.api ?? "", npm },
      cost: m.cost ? cost(m.cost) : was?.cost ?? cost(),
      limit: { ...(was?.limit ?? { context: 0, output: 0 }), ...(m.limit ?? {}) },
      options: { ...(was?.options ?? {}), ...(m.options ?? {}) },
      headers: { ...(was?.headers ?? {}), ...(m.headers ?? {}) },
      capabilities: {
        ...(was?.capabilities ?? { temperature: true, toolcall: true, input: { text: true }, output: { text: true } }),
        ...(m.reasoning !== undefined ? { reasoning: m.reasoning } : {}),
        ...(m.attachment !== undefined ? { attachment: m.attachment } : {}),
        ...(m.tool_call !== undefined ? { toolcall: m.tool_call } : {}),
        ...(m.modalities?.input ? { input: modalities(m.modalities.input) } : {}),
        ...(m.modalities?.output ? { output: modalities(m.modalities.output) } : {}),
      },
      variants: m.variants ?? was?.variants ?? {},
      // what the config itself says of the model's sampling, as Factory's
      // config models say temperature of their OpenAI route (plugins#154)
      ...(m.temperature !== undefined ? { temperature: m.temperature } : {}),
      // magpie's own mark: a decision model, answering System One
      ...(typeof m.decides === "boolean" ? { decides: m.decides } : was?.decides !== undefined ? { decides: was.decides } : {}),
      // and one that searches the web by itself, asked in plain words
      ...(typeof m.searches === "boolean" ? { searches: m.searches } : was?.searches !== undefined ? { searches: was.searches } : {}),
      // and the other APIs its vendor serves it on, by their AI SDK packages
      ...(Array.isArray(m.also) ? { also: m.also } : was?.also !== undefined ? { also: was.also } : {}),
    }
  }
  const server = auths().get(id)?.spec
  if (quiet) out.fellBack = true
  for (const h of quiet ? [] : hooks) {
    const ph = h.hooks.provider
    if (ph?.id !== id || typeof ph.models !== "function") continue
    // a plugin that signs in to id too but doesn't serve it: its list
    // would be asked with the serving plugin's account
    if (server && h.spec !== server && signsIn(h.spec).includes(id)) continue
    const k = key ?? accountsOf(readAuth(), id)[0] ?? id
    // the wait for a due renewal spends the hook's own budget: one
    // provider then costs one listWait in a listing, not listWait plus
    // the renewal wait — the sum (15s + 20s) took a single provider past
    // refreshDeadline, and the whole listing with it (#4065)
    const began = Date.now()
    await fresh(id, k)
    const all = readAuth()
    const held = refusedLists.get(k)
    if (held && all[k] && held.sign === signInOf(all[k]) && Date.now() - held.at < retryAfter(held.n)) {
      // refused before, and nothing changed since: the vendor isn't asked
      // again, and the account stays marked as it was
      const r = reach.getStore()
      if (r) r.refused ??= held.error
      out.fellBack = true
      out.listError = held.error
      continue
    }
    // the hook's own signal: its listWait deadline aborts it, and so does
    // the listing's caller going away, whichever comes first (#4678)
    const ctl = new AbortController()
    if (signal?.aborted) ctl.abort()
    else signal?.addEventListener("abort", () => ctl.abort(), { once: true })
    try {
      const given = JSON.parse(JSON.stringify(out))
      const l = { tried: false, lastOk: false }
      const ask = () => listing.run(l, () => inScope(id, k, () => ph.models(given, { auth: authCopy(k), signal: ctl.signal })))
      const next = limit > 0 ? await inTime(ask, Math.max(limit - (Date.now() - began), 0), "the plugin's model list", ctl) : await ask()
      // a hook that asked its vendor, got no list and gave back the one it
      // was given fell back: magpie keeps the list it had, as a built-in
      // whose fetch failed keeps the one it fetched last; so does one that
      // says so, handing back a list of its own (Symbol.for("magpie.fellBack")
      // on it: Command Code's Go table, ZCode's models)
      out.fellBack = (next === given.models && l.tried && !l.lastOk) || next?.[Symbol.for("magpie.fellBack")] === true
      // why, as far as the host saw it: the editor says so rather than
      // show the plugin's short defaults as if they were the account's
      if (out.fellBack) out.listError = l.said || "the plugin couldn't get its vendor's list"
      out.models = Object.fromEntries(Object.entries(next ?? {}).map(([k, m]) => [k, { ...m, id: k, providerID: id }]))
      refusedLists.delete(k)
    } catch (e) {
      // a listing that gave up waiting: what the hook gives later is no
      // list a request should be built from
      if (e instanceof Late) out.late = true
      // an error the models hook throws may say what it means for the
      // sign-in, as an answer's X-Magpie-Sign-In does: a built-in whose
      // model list the vendor refused marked the account
      if (["expired", "kept", "renewed"].includes(e?.signIn) && all[k]) send({ event: "signIn", provider: id, account: k, said: e.signIn })
      // strict (a move's check) fails on what the account can't do, but
      // a sign-in the vendor refused isn't that: it is marked, as the
      // built-in marked it, and goes along untried, as a lapsed one does
      if (strict && !(e?.signIn === "expired" && all[k])) throw e
      const r = reach.getStore()
      if (r && e?.signIn === "expired") r.refused ??= e?.message ?? String(e)
      if (e?.signIn === "expired" && all[k]) {
        const was = refusedLists.get(k)
        const sign = signInOf(all[k])
        refusedLists.set(k, { sign, at: Date.now(), n: was?.sign === sign ? was.n + 1 : 1, error: e?.message ?? String(e) })
      }
      // a hook that threw on its vendor's failure has no list to tell:
      // magpie keeps the one it had, as for one that gave its defaults back
      out.fellBack = true
      out.listError = e?.message ?? String(e)
      // a listing the caller gave up on says its abort itself, not as if
      // the vendor had failed
      if (!ctl.signal.aborted) send({ event: "log", level: "error", message: `${h.spec}: provider.models: ${e?.message ?? e}` })
    }
  }
  for (const [k, m] of Object.entries(cfg?.models ?? {})) if (m?.disabled) delete out.models[k]
  return out
}

// infoOf is info(id, key) as a request has it: the one built for the
// account, until its sign-in or the options change. What it gives is
// shared: the caller must not change it.
async function infoOf(id, key) {
  const md = mdev()
  const had = infos.get(key)
  if (had && had.gen === infoGen && had.md === md) return had.info
  const gen = infoGen
  const p = await info(id, key)
  // a sign-in changed while it was built: the next request builds it again
  if (gen === infoGen) infos.set(key, { gen, md, info: p })
  return p
}

// keepInfo keeps p, info(id, key) just built by a listing, for the
// account's requests.
function keepInfo(key, p, gen) {
  if (gen === infoGen && !p.failed && !p.late) infos.set(key, { gen, md: mdev(), info: p })
}

// shown is a plugin's string as magpie shows it: trimmed and at most n
// characters, "" for anything else.
const shown = (v, n) => (typeof v === "string" ? v.trim().slice(0, n) : "")

// optionsOf are the switches a plugin declares for its provider, magpie's
// own field (#2038, Vocllum): auth.options, each {key, label, description?,
// default?, confirm?, labels?, descriptions?, confirms?, filtersModels?,
// placement?}.
// magpie shows each as an Off/On switch in the provider's editor and keeps
// what the user sets; the plugin reads it with input.magpie.option(provider,
// key). A switch is kept and handed over, and the models listed again (one
// may go by it); nothing else is run. Keys are [a-z0-9_-], 1–40 long, the
// first of a key kept; at most MAX_OPTIONS; one without a key or a label is
// left out. labels, descriptions and confirms are the same words in the
// window's other languages ({"zh": "…", "zh-TW": "…", "ja": "…", "de":
// "…"}), as a page's titles are (plugins#590): label stays a string beside
// them, so an older magpie still reads the option. filtersModels says the
// plugin's model list leaves models out while the switch is on, which the
// editor says by the models. placement "top" draws the switch under the
// accounts, above Routing, for one about the whole plugin (#5269); anything
// else, or none, keeps it under the models, where an older magpie draws it.
const MAX_OPTIONS = 16
const OPTION_KEY = /^[a-z0-9_-]{1,40}$/
// inLanguages are words by language, each capped at n, at most 16, without
// the empty ones; undefined for none or for what isn't an object
function inLanguages(m, n) {
  if (!m || typeof m !== "object" || Array.isArray(m)) return undefined
  let out
  for (const [k, v] of Object.entries(m)) {
    const lang = k.trim(), said = shown(v, n)
    if (!lang || lang.length > 16 || !said) continue
    out ??= {}
    if (Object.keys(out).length >= 16) break
    out[lang] = said
  }
  return out
}
function optionsOf(auth) {
  const out = []
  if (!Array.isArray(auth?.options)) return out
  for (const o of auth.options) {
    if (out.length >= MAX_OPTIONS) break
    if (!o || typeof o !== "object" || typeof o.key !== "string" || !OPTION_KEY.test(o.key)) continue
    if (out.some((x) => x.key === o.key)) continue
    const label = shown(o.label, 60)
    if (!label) continue
    out.push({
      key: o.key,
      label,
      description: shown(o.description, 300),
      default: o.default === true,
      confirm: shown(o.confirm, 300),
      labels: inLanguages(o.labels, 60),
      descriptions: inLanguages(o.descriptions, 300),
      confirms: inLanguages(o.confirms, 300),
      filtersModels: o.filtersModels === true,
      placement: o.placement === "top" ? "top" : undefined,
    })
  }
  return out
}

// optionValues are provider's options as they stand: the user's value,
// else the plugin's default, for each key its plugin declares.
function optionValues(provider) {
  const own = chosen?.[provider] ?? {}
  const out = {}
  for (const o of optionsOf(auths().get(provider)?.auth)) out[o.key] = typeof own[o.key] === "boolean" ? own[o.key] : o.default
  return out
}

// reportLoad is magpie.load(provider, loads): how busy each of the
// provider's models is now, as a plugin knows from its vendor, for the
// model list to show and routing to weigh (axelburks on Discord). loads
// maps a model id to a percent from 0, past 100 for a vendor that queues
// (200 is twice what it serves at once; capped at LOAD_MOST), or to null
// to say it isn't known any more. Called whenever the plugin
// likes, from its own timer: a model's load counts for five minutes from
// the call that reported it, so a plugin that stops calling leaves its
// models unknown, never idle. It returns the count of models taken.
const LOAD_MODELS = 1000
const LOAD_MOST = 10000 // a sanity cap: 100 times full, so the UI stays sane
function reportLoad(provider, loads) {
  if (typeof provider !== "string" || !provider || provider.length > 200) throw new TypeError("magpie.load: provider must be a provider id")
  if (!loads || typeof loads !== "object" || Array.isArray(loads)) throw new TypeError("magpie.load: loads must map model ids to a number from 0 up, or null")
  const out = {}
  let n = 0
  for (const [id, v] of Object.entries(loads)) {
    if (n >= LOAD_MODELS) break
    if (!id || id.length > 200) continue
    if (v === null) out[id] = -1 // cleared
    else if (typeof v === "number" && Number.isFinite(v)) out[id] = Math.min(LOAD_MOST, Math.max(0, v))
    else continue
    n++
  }
  if (n) send({ event: "load", provider, loads: out })
  return n
}

// reportUsage is magpie.usage(provider, call): a request a plugin sent its
// vendor on its own, outside any request magpie handed it (Devin's cache
// keepalive ping, plugins#592), for the request log and the usage totals
// to list and count. call is {kind, model, input?, output?, cacheRead?,
// cacheWrite?, status?, ms?, error?, account?}: kind says what the
// request was for ([a-z0-9_-], 1–40: "cache_keepalive"), model the
// model's id as magpie handed it to the plugin; the tokens are whole
// numbers from 0 (input without the cache's), status the vendor's (0 for
// none), ms how long it took. account is the account the request was
// sent for, as magpie.account(provider) named it while a request of
// magpie's was in the plugin's hands; without it, the account of the
// request being served now, else the provider's first. It returns true
// when the call was sent on.
const USAGE_KIND = /^[a-z0-9_-]{1,40}$/
function reportUsage(provider, call) {
  if (typeof provider !== "string" || !provider || provider.length > 200) throw new TypeError("magpie.usage: provider must be a provider id")
  if (!call || typeof call !== "object" || Array.isArray(call)) throw new TypeError("magpie.usage: call must be an object")
  if (typeof call.kind !== "string" || !USAGE_KIND.test(call.kind)) throw new TypeError("magpie.usage: kind must be 1–40 of [a-z0-9_-]")
  const model = shown(call.model, 200)
  if (!model) throw new TypeError("magpie.usage: model must be the model's id")
  const n = (v, most = 1e12) => (typeof v === "number" && Number.isFinite(v) && v > 0 ? Math.min(Math.floor(v), most) : 0)
  const account = typeof call.account === "string" && call.account && providerOf(call.account) === provider ? call.account : keyFor(provider)
  send({
    event: "usage",
    provider,
    account,
    usage: { kind: call.kind, model, input: n(call.input), output: n(call.output), cacheRead: n(call.cacheRead), cacheWrite: n(call.cacheWrite), status: n(call.status, 999), ms: n(call.ms), error: shown(call.error, 500) },
  })
  return true
}

// optionValue is one of provider's options as it stands; false for a key
// its plugin doesn't declare.
function optionValue(provider, key) {
  return optionValues(provider)[key] === true
}

// byLang is a sign-in word by language, magpie's own field beside
// OpenCode's English one (a method's labels, a question's messages, a
// choice's labels and hints: {"zh": "…", "zh-TW": "…"}), as { [name]: map }
// to spread, or {}: codes of 1–16 characters, at most 16, empty and
// non-string ones left out, as a page's titles are (plugin.titles).
function byLang(v, name) {
  if (!v || typeof v !== "object" || Array.isArray(v)) return {}
  const out = {}
  let n = 0
  for (const [k, x] of Object.entries(v)) {
    const code = k.trim(), text = shown(x, 300)
    if (!code || code.length > 16 || !text || n === 16) continue
    out[code] = text
    n++
  }
  return n ? { [name]: out } : {}
}

// methods are the auth hook's ways to sign in. An "api" one may say what
// its key looks like (placeholder, magpie's own: OpenCode's dialog says
// "API key"); its label titles the key's field, as OpenCode's does.
function methods(auth) {
  return (auth.methods ?? []).map((m) => ({
    type: m.type,
    label: m.label,
    ...byLang(m.labels, "labels"),
    ...(m.type === "api" && m.keyless === true && typeof m.authorize === "function" ? { keyless: true } : {}),
    ...(keyAsked(m) ? { keyAsked: true } : {}),
    ...(m.type === "api" && shown(m.placeholder, 200) ? { placeholder: shown(m.placeholder, 200) } : {}),
  }))
}

// keyAsked is whether an "api" method's own questions ask its key: it has
// prompts and an authorize that turns their answers into the key, as
// inkstone's and Devin's session-token method do. magpie asks no key of
// its own beside them; one typed there went nowhere (authorize gets the
// answers only), so the user gave the same token twice.
function keyAsked(m) {
  return m.type === "api" && Array.isArray(m.prompts) && m.prompts.length > 0 && typeof m.authorize === "function"
}

// iconOf is the picture a plugin gives its provider, magpie's own field
// (OpenCode's providers have none): the auth hook's icon, else
// package.json's magpie.icon. An https URL or a data:image URI; magpie
// checks and keeps it (internal/provider), nothing is fetched here.
function iconOf(a) {
  const ok = (v) => {
    const s = typeof v === "string" ? v.trim() : ""
    // a data URI of a picture over 1 MB, magpie's most, isn't carried
    return s.length <= 3 << 19 && /^(https:\/\/|data:image\/)/i.test(s) ? s : ""
  }
  const own = ok(a.auth.icon)
  if (own || !a.target) return own
  const dir = fs.statSync(a.target, { throwIfNoEntry: false })?.isDirectory() ? a.target : path.dirname(a.target)
  return ok(readJSON(path.join(dir, "package.json"))?.magpie?.icon)
}

// concurrencyOf is how many requests the plugin says each of its accounts
// takes at once, magpie's own field: the auth hook's maxConcurrency, else
// package.json's magpie.maxConcurrency. A whole number over 0, else none
// (0); the user's setting on the provider goes over it.
function concurrencyOf(a) {
  return declared(a, "maxConcurrency")
}

// atOnceOf is how many of its accounts the plugin says may take requests
// at the same time, magpie's own field: the auth hook's maxAccounts, else
// package.json's magpie.maxAccounts, as concurrencyOf reads its own. The
// user's setting on the provider goes over it.
function atOnceOf(a) {
  return declared(a, "maxAccounts")
}

// declared is a whole number over 0 the plugin declares under key, on its
// auth hook or else in its package.json's magpie, at most 1000; else 0.
function declared(a, key) {
  const ok = (v) => (Number.isInteger(v) && v > 0 ? Math.min(v, 1000) : 0)
  const own = ok(a.auth[key])
  if (own || !a.target) return own
  const dir = fs.statSync(a.target, { throwIfNoEntry: false })?.isDirectory() ? a.target : path.dirname(a.target)
  return ok(readJSON(path.join(dir, "package.json"))?.magpie?.[key])
}

// rateOf reads a model's credit rate as a plugin gives it: a number (0.5)
// or as the vendor's picker writes it ("x0.03", "0.5×"); 0 is none, as is
// one it can't read
function rateOf(v) {
  const n = typeof v === "string" ? Number(v.trim().replace(/^[x×]\s*|\s*[x×]$/gi, "")) : v
  return typeof n === "number" && Number.isFinite(n) && n > 0 ? n : 0
}

// tiersOf reads the service tiers a plugin says a model serves
// (m.serviceTiers): ids ("priority", "ultrafast"), or objects with an id as
// Codex's catalog writes them; anything else is dropped
function tiersOf(v) {
  if (!Array.isArray(v)) return []
  const out = []
  for (const t of v) {
    const id = typeof t === "string" ? t.trim() : typeof t?.id === "string" ? t.id.trim() : ""
    if (id && !out.includes(id)) out.push(id)
  }
  return out
}

// providers lists each provider and its accounts' models, each asked
// through its proxy (proxies[provider][key], "" the provider's own), as a
// built-in fetches each account's list through the account's. A caller
// that gave up (signal) has every hook it would still have asked told, so
// a timed-out listing stops its vendor reads instead of multiplying them
// (#4678).
async function providers({ proxies, off } = {}, signal) {
  const stored = readAuth()
  // a provider switched off in magpie is asked nothing: its models hook
  // (and the sign-in renewal before it) would reach its vendor. It is
  // told as fallen back, so magpie keeps the list it had.
  const quiet = new Set(Array.isArray(off) ? off : [])
  // every provider is listed side by side, each models hook given
  // listWait: one whose vendor is stuck holds up no other's row
  const row = async ([id, a]) => {
    const keys = accountsOf(stored, id)
    const through = (k) => proxies?.[id]?.[k ?? keys[0] ?? ""] ?? proxies?.[id]?.[""] ?? ""
    const gen = infoGen
    if (quiet.has(id)) {
      const p = await info(id, undefined, false, listWait, true)
      return rowOf(id, a, keys, stored, p, [])
    }
    // every account is listed side by side, each hook given listWait: a
    // second account no longer waits behind the first, where two
    // listWaits back to back took one provider past refreshDeadline twice
    // over (#4065)
    const [p, own] = await Promise.all([
      via.run(through(), () => info(id, undefined, false, listWait, false, signal)).then((q) => (keepInfo(keys[0] ?? id, q, gen), q)),
      // each account's own models, as the built-ins read each account's: a
      // plan may serve fewer, or others, than the first account's. One that
      // can't be read is taken to have them all, or the ones it was last
      // told to have, as a built-in account whose fetch failed keeps its own.
      Promise.all(keys.slice(1).map((k) => via.run(through(k), () => info(id, k, true, listWait, false, signal)).then((q) => (keepInfo(k, q, gen), q)).catch((e) => ({ failed: true, listError: e?.message ?? String(e) })))),
    ])
    return rowOf(id, a, keys, stored, p, own)
  }
  return Promise.all([...auths()].map(row))
}

// rowOf is the providers row of id (registered as a), its accounts keys
// of stored, p its info and own the other accounts' infos.
function rowOf(id, a, keys, stored, p, own) {
  const models = { ...p.models }
  for (const q of own) for (const [k, m] of Object.entries(q?.models ?? {})) models[k] ??= m
  const ids = (q) => Object.values(q.models).filter((m) => m.status !== "deprecated").map((m) => m.id)
  const first = stored[keys[0]]
  // the provider's list is its accounts' union, so it fell back only when
  // no account told a list of its own: a lapsed account kept first beside
  // a live one doesn't say the provider fell back, and its list goes on
  // moving (the last one told is kept for a provider whose every account
  // fell back) (#4917)
  const fellBack = keys.length > 0 && !!p.fellBack && own.every((q) => !q || q.fellBack || q.failed)
  return {
    id,
    spec: a.spec,
    name: p.name,
    npm: p.npm ?? "",
    api: p.api ?? "",
    methods: methods(a.auth),
    single: a.auth.single === true,
    // its accounts can be brought in from a file (auth.import), and which
    // file that is, as the plugin says it
    ...importOf(a.auth),
    // the vendor CLI its sign-in and requests run through, which magpie
    // installs with the plugin's own installer when it isn't there
    ...cliOf(a.auth),
    icon: iconOf(a),
    usage: typeof a.auth.usage === "function",
    checkin: typeof a.auth.checkin === "function",
    calls: typeof a.auth.calls === "function",
    // its sign-ins are renewed in the background as they come due, so
    // long before expires (keepSignedIn)
    renews: keepsAlive(a.auth),
    renewLead: keepsAlive(a.auth) ? leadOf(a.auth) : 0,
    // the vendor spends a refresh token once: renewed, the old one is
    // refused (invalid_grant). magpie never carries such a sign-in to
    // another computer (#5583)
    rotating: a.auth.rotating === true,
    options: optionsOf(a.auth),
    decide: p.decide === true,
    searches: p.searches === true,
    maxConcurrency: concurrencyOf(a),
    maxAccounts: atOnceOf(a),
    signedIn: keys.length > 0,
    authType: first?.type ?? "",
    accountId: whoOf(first),
    fellBack,
    listError: fellBack ? p.listError ?? "" : "",
    accounts: keys.map((k, i) => ({
      key: k,
      type: stored[k]?.type ?? "",
      accountId: whoOf(stored[k]),
      hint: hintOf(stored[k]),
      models: own.length === 0 ? undefined : i === 0 ? ids(p) : own[i - 1]?.models ? ids(own[i - 1]) : undefined,
      fellBack: i === 0 ? !!p.fellBack : !!(own[i - 1]?.fellBack || own[i - 1]?.failed),
      listError: (i === 0 ? p.fellBack && p.listError : own[i - 1]?.listError) || "",
    })),
    models: Object.values(models)
      .filter((m) => m.status !== "deprecated")
      .map((m) => ({
        id: m.id,
        name: m.name,
        npm: m.api?.npm ?? p.npm ?? "",
        url: m.api?.url ?? "",
        apiId: m.api?.id ?? m.id,
        context: m.limit?.context ?? 0,
        input: m.limit?.input ?? 0,
        output: m.limit?.output ?? 0,
        reasoning: !!m.capabilities?.reasoning,
        image: !!m.capabilities?.input?.image,
        imageSaid: typeof m.capabilities?.input?.image === "boolean",
        // every kind it takes and gives, not image alone (#1595)
        inputs: kinds(m.capabilities?.input),
        inputsSaid: inputSaid(m.capabilities?.input),
        outputs: kinds(m.capabilities?.output),
        outputsSaid: kindsSaid(m.capabilities?.output),
        // whether the model takes temperature and top_p, where something
        // said so (models.dev, the config, the models hook); unsaid goes
        // to magpie as nothing, which keeps them (#2492)
        ...(typeof m.temperature === "boolean" ? { temperature: m.temperature } : {}),
        released: m.release_date ?? "",
        cost: m.cost,
        variants: Object.keys(m.variants ?? {}),
        free: m.free === true,
        // what a request costs of the plan's credits, as a multiple,
        // and before a discount running now (Qoder's price_factor)
        rate: rateOf(m.rate),
        rateWas: rateOf(m.rateWas),
        // run fast when the request's service_tier is priority (Cursor's)
        fast: m.fast === true,
        // the service tiers the vendor serves it at (service_tier
        // "priority", "ultrafast"), which Codex then offers (#2514)
        serviceTiers: tiersOf(m.serviceTiers),
        // a decision model (System One's), hidden from agents and
        // offered as a routing group's classifier
        decides: m.decides === true,
        // a model that searches the web by itself, asked in plain
        // words, which magpie may search with for other models
        searches: m.searches === true,
        // the other APIs its vendor serves it on besides npm's, by their AI
        // SDK packages (Open Design's amr-link serves every model on all
        // three), where a client's request is relayed as it was sent
        also: Array.isArray(m.also) ? m.also.filter((x) => typeof x === "string" && x !== "") : [],
      })),
  }
}

// ---- signing in --------------------------------------------------------------

function applies(prompt, inputs) {
  if (prompt.when) {
    const v = inputs[prompt.when.key]
    if (v === undefined) return false
    const eq = v === prompt.when.value
    if (prompt.when.op === "eq" ? !eq : eq) return false
  }
  if (typeof prompt.condition === "function" && !prompt.condition(inputs)) return false
  return true
}

// nextPrompt is the method's next question for inputs so far, as
// OpenCode's CLI asks them: in order, those whose when/condition hold. A
// method may ask its own way (magpie's hook, which pi's sign-ins use, as
// they ask as they go): ask(inputs) → the next question, or null.
async function nextPrompt(provider, index, inputs) {
  const m = authOf(provider).methods[index]
  if (!m) throw new Error(`no sign-in method ${index} for ${provider}`)
  if (typeof m.ask === "function") return (await m.ask(inputs)) ?? null
  for (const p of m.prompts ?? []) {
    if (p.key in inputs) continue
    if (!applies(p, inputs)) continue
    return {
      type: p.type,
      key: p.key,
      message: p.message,
      ...byLang(p.messages, "messages"),
      placeholder: p.placeholder ?? "",
      options: p.type === "select" ? p.options.map((o) => ({ label: o.label, value: o.value, hint: o.hint ?? "", ...byLang(o.labels, "labels"), ...byLang(o.hints, "hints") })) : undefined,
    }
  }
  return null
}

function validate(provider, index, key, value) {
  const p = (authOf(provider).methods[index]?.prompts ?? []).find((x) => x.key === key)
  if (!p || typeof p.validate !== "function") return null
  return p.validate(value) ?? null
}

// save keeps a successful sign-in as OpenCode's CLI does, at key (or, for
// a sign-in the plugin says is another provider's, a new account of
// that one). It gives the provider and where the account is kept.
async function save(provider, key, result, inputs, apiKey, method) {
  const id = result.provider ?? provider
  if (id !== provider) key = freshKey(id)
  if ("refresh" in result) {
    const { type: _t, provider: _p, refresh, access, expires, ...extra } = result
    await setAuth(key, { type: "oauth", refresh, access, expires, ...extra })
  } else if ("key" in result || apiKey) {
    const md = { ...plainAnswers(provider, method, inputs), ...(result.metadata ?? {}) }
    await setAuth(key, { type: "api", key: result.key ?? apiKey, ...(Object.keys(md).length ? { metadata: md } : {}) })
  }
  return { provider: id, account: await settle(id, key) }
}

// plainAnswers are the answers to an "api" method's questions that the
// account's metadata keeps beside what its authorize gave: a choice
// (select) only. A typed answer may be the token or key itself (inkstone's
// apiKey and uaaToken, Cloudflare's token), and the key is kept as the
// account's key already; copied into metadata it sat there a second time,
// in plain text. A plugin that wants a typed answer kept puts it in its
// result's metadata, as every one in magpie-community/plugins does.
function plainAnswers(provider, method, inputs) {
  if (!inputs || typeof inputs !== "object") return {}
  const prompts = authOf(provider).methods[method]?.prompts ?? []
  const out = {}
  for (const [k, v] of Object.entries(inputs)) {
    if (prompts.some((p) => p?.key === k && p.type === "select")) out[k] = v
  }
  return out
}

// signInKey is where a sign-in goes: the account named, else a new one.
function signInKey(provider, account) {
  if (authOf(provider).single === true) return accountsOf(readAuth(), provider)[0] ?? provider
  return account && account !== "new" && providerOf(account) === provider ? account : freshKey(provider)
}

let nextSession = 1

// A sign-in goes through the proxy its subscription's requests will (proxy,
// as fetch's: "", "direct" or its URL), so the vendor sees the account
// come from one place from its first request on (#1663): authorize and the
// callback that trades the code are both run under it.
//
// A sign-in magpie gives up on (the user cancelled it, or it timed out)
// ends here as well (#5016): signal is aborted, the fetches the plugin
// makes for it fail, the plugin's own wait is handed it (magpie's hook:
// callback(code, { signal }), authorize(inputs, { signal }); OpenCode
// passes none), and whatever the plugin answers after it is dropped,
// nothing saved.
async function authorize({ provider, method, inputs, account, proxy }, signal) {
  const m = authOf(provider).methods[method]
  if (!m) throw new Error(`no sign-in method ${method} for ${provider}`)
  if (m.type !== "oauth") throw new Error("not an oauth method")
  const key = signInKey(provider, account)
  // the inputs go only to a method that asks something, as OpenCode's TUI
  // (/connect) passes them: an object with none is how its CLI (opencode
  // auth login) calls, and a plugin told so asks its questions on the
  // terminal, magpie's stdin here (opencode-antigravity-auth waited on its
  // "Project ID" prompt, the sign-in never starting)
  const a = await signingIn(signal, proxy ?? "", provider, key, () => m.authorize(m.prompts?.length ? inputs ?? {} : undefined, { signal }))
  if (signal?.aborted) throw abortedSignIn()
  // a plugin that can't start the sign-in may say why as an api method's
  // authorize does ({ type: "failed", error }) rather than throw; with no
  // callback there is nothing to wait on, and calling the one it lacks
  // said "s.a.callback is not a function" instead of its reason
  // (plugins#658)
  if (a?.type === "failed") return failed(a)
  if (typeof a?.callback !== "function") return { ok: false, error: `${provider}'s sign-in gave nothing to finish it with` }
  const session = String(nextSession++)
  sessions.set(session, { provider, key, a, inputs, proxy: proxy ?? "" })
  // opened (not OpenCode's) is a page the plugin's own CLI opened already:
  // Grok Build's `grok login` opens it on macOS and Windows, and magpie's
  // opening it too showed it twice (andyfmyth on X)
  return { ok: true, session, url: a.url ?? "", instructions: a.instructions ?? "", method: a.method, opened: a.opened === true }
}

// failed is a sign-in's failure, with why, where the plugin tells it
// ({ type: "failed", error: "…" }; OpenCode's own result has no reason).
function failed(r) {
  const why = typeof r?.error === "string" ? r.error.trim() : r?.error instanceof Error ? r.error.message : ""
  return why ? { ok: false, error: why.slice(0, 500) } : { ok: false }
}

function abortedSignIn() {
  return new DOMException("the sign-in was cancelled", "AbortError")
}

// signingIn runs a plugin's sign-in step fn for key under proxy and
// signal; it ends when signal aborts, whether fn does or not.
function signingIn(signal, proxy, provider, key, fn) {
  const step = via.run(proxy, () => inScope(provider, key, () => signInSignal.run(signal, fn), true))
  return signal ? bounded(Promise.resolve(step), signal) : step
}

async function callback({ session, code }, signal) {
  const s = sessions.get(session)
  if (!s) throw new Error("no such sign-in")
  sessions.delete(session)
  const r = await signingIn(signal, s.proxy, s.provider, s.key, () => (s.a.method === "code" ? s.a.callback(code ?? "", { signal }) : s.a.callback(undefined, { signal })))
  if (signal?.aborted) throw abortedSignIn()
  if (!r || r.type !== "success") return failed(r)
  const kept = keptAs(r)
  if (!kept) return { ok: false, error: `${s.provider}'s sign-in gave nothing to keep` }
  return { ok: true, ...(await save(s.provider, s.key, kept, undefined)) }
}

// keptAs is an OAuth callback's success r as save keeps it. save keeps an
// OAuth sign-in only when "refresh" is in r, as OpenCode's CLI does; a
// plugin whose sign-in has no refresh token and leaves the field out
// (codearts' CodeArts Agent sign-in, plugins#998) was told "signed in"
// with nothing kept. An access with no refresh is kept with an empty one,
// and a success with nothing at all to keep is null, a failure, as
// apiKey's is.
function keptAs(r) {
  if ("refresh" in r || "key" in r) return r
  if (typeof r.access === "string" && r.access) return { ...r, refresh: "" }
  return null
}

// ---- accounts from a file -----------------------------------------------------

// A plugin may take accounts from a file the user picks (magpie's own
// hook, which OpenCode ignores), as magpie's "Import accounts from a
// file…" does for its built-ins:
//   auth.import(texts, { signal }) → what each file's text holds, checked
//     with the vendor as a sign-in is: an array of sign-in results
//     ({ type: "success", … } as a callback or an api authorize gives one,
//     or { type: "failed", error, accountId? }), or one of them.
//   auth.importFrom: the file to pick, in a sentence (importFroms: the same
//     in other languages, by language code).
// Each success is kept as a sign-in is, as one more account or over the
// account it already is. It runs through the provider's proxy, as a
// sign-in does (#1663).
const IMPORT_MOST = 100

function importOf(auth) {
  if (typeof auth?.import !== "function") return {}
  const from = shown(auth.importFrom, 300)
  const froms = inLanguages(auth.importFroms, 300)
  return { importable: true, ...(from ? { importFrom: from } : {}), ...(froms ? { importFroms: froms } : {}) }
}

// ---- the vendor CLI a plugin runs through ------------------------------------

// A plugin that runs its vendor's CLI may say how to install it (magpie's
// own hook, which OpenCode ignores), so a machine without it, magpie in a
// container above all, gets it from magpie's sign-in rather than a shell:
//   auth.cli.name: the CLI, as the sign-in names it ("Installing {name}…").
//   auth.cli.installed(): whether it is on this machine now.
//   auth.cli.install({ signal }): installs it, resolving once it is there;
//     magpie's sign-in starts this when installed() is false, then asks
//     the method's authorize.
//   auth.cli.manual: how a person installs it, told when install fails.
function cliOf(auth) {
  const c = auth?.cli
  if (!c || typeof c !== "object" || typeof c.installed !== "function" || typeof c.install !== "function") return {}
  const name = shown(c.name, 80)
  if (!name) return {}
  const manual = shown(c.manual, 300)
  return { cli: { name, ...(manual ? { manual } : {}) } }
}

function cliHook(provider) {
  const c = authOf(provider).cli
  if (!cliOf({ cli: c }).cli) throw new Error(`${provider} names no CLI to install`)
  return c
}

async function cliInstalled({ provider }) {
  return { installed: (await cliHook(provider).installed()) === true }
}

// installCli runs the plugin's installer through the proxy the
// subscription's requests take (#1663), ended when magpie gives it up
async function installCli({ provider, proxy }, signal) {
  const c = cliHook(provider)
  const step = via.run(proxy ?? "", () => c.install({ signal }))
  await (signal ? bounded(Promise.resolve(step), signal) : step)
  if (signal?.aborted) throw new DOMException("the install was cancelled", "AbortError")
  return null
}

async function importFiles({ provider, texts, proxy }, signal) {
  const a = authOf(provider)
  if (typeof a.import !== "function") throw new Error(`${provider} takes no accounts from a file`)
  const files = (Array.isArray(texts) ? texts : []).filter((t) => typeof t === "string" && t.trim())
  const got = await signingIn(signal, proxy ?? "", provider, freshKey(provider), () => a.import(files, { signal }))
  if (signal?.aborted) throw abortedSignIn()
  const list = (Array.isArray(got) ? got : got ? [got] : []).slice(0, IMPORT_MOST)
  const results = []
  for (const r of list) {
    const user = typeof whoOf(r) === "string" ? whoOf(r).slice(0, 200) : ""
    if (!r || r.type !== "success") {
      results.push({ status: "failed", user, error: failed(r).error || `${provider} didn't take it` })
      continue
    }
    const kept = keptAs(r)
    if (!kept) {
      results.push({ status: "failed", user, error: `${provider} gave nothing to keep` })
      continue
    }
    const had = new Set(accountsOf(readAuth(), r.provider ?? provider))
    const saved = await save(provider, signInKey(provider), kept)
    results.push({ status: had.has(saved.account) ? "updated" : "added", user, ...saved })
  }
  return { results }
}

async function apiKey({ provider, method, inputs, key, account, proxy }, signal) {
  const m = authOf(provider).methods[method]
  if (!m || m.type !== "api") throw new Error("not an API key method")
  const at = signInKey(provider, account)
  if (typeof m.authorize !== "function") {
    const md = inputs && Object.keys(inputs).length ? { metadata: inputs } : {}
    await setAuth(at, { type: "api", key, ...md })
    return { ok: true, provider, account: await settle(provider, at) }
  }
  const r = await signingIn(signal, proxy ?? "", provider, at, () => m.authorize(inputs ?? {}, { signal }))
  if (signal?.aborted) throw abortedSignIn()
  if (!r || r.type !== "success") return failed(r)
  // nothing to keep: no key typed, and the plugin's authorize gave none
  if (!("key" in r) && !("refresh" in r) && !key) return { ok: false, error: `${provider}'s sign-in gave no key` }
  return { ok: true, ...(await save(provider, at, r, inputs, key, method)) }
}

// ---- renewing a sign-in ------------------------------------------------------

// A plugin may leave renewing its accounts' tokens to magpie (magpie's own
// hook, which OpenCode ignores):
//   auth.refresh(auth, provider) → the account's sign-in renewed: the
//     fields to keep over the old ({ access, refresh, expires, … }), or
//     nothing when there is nothing to renew. It throws when it can't; an
//     error with signIn "expired" says the vendor turned the sign-in away
//     for good, and the account is marked.
//   auth.refreshLead: how long (ms) before expires a token is renewed,
//     5 minutes when not said.
//   auth.keepAlive: false leaves an account nothing uses to lapse; any
//     other value has the magpie serving the gateway renew it in the
//     background as it comes due too (keepSignedIn, #5080).
// An OAuth sign-in whose expires is that close is renewed before its
// loader, models, usage or a request runs, once at a time per account: the
// requests that find it due wait for the one renewal (a vendor that spends
// a refresh token once turns a second away). A renewal that fails leaves
// the sign-in as it was, for the vendor's answer to tell, and isn't tried
// again for RENEW_RETRY, doubled for each failure in a row up to
// RENEW_RETRY_MOST (retryAfter), or, turned away for good, till it is
// signed in again. A refresh that gives nothing back while the sign-in is
// due failed too: magpie-ima-auth's swallows ima's "sign in again" and
// returns undefined, and every request, models listing and usage read
// asked ima again, ~80 times a minute for two dead accounts
// (JZGiVvkSOA75376 on X).

const RENEW_LEAD = 5 * 60 * 1000
const RENEW_WAIT = 15 * 1000
const RENEW_RETRY = 30 * 1000
const RENEW_RETRY_MOST = 30 * 60 * 1000

// retryAfter is how long a sign-in whose renewal failed n times in a row
// waits before it is tried again.
function retryAfter(n) {
  return Math.min(RENEW_RETRY * 2 ** Math.max(0, Math.min(n, 20) - 1), RENEW_RETRY_MOST)
}

// failedRenewal notes that the sign-in was's renewal at key failed, one
// more in a row when the last one was of the same sign-in.
function failedRenewal(key, was, gone, error) {
  const f = unrenewed.get(key)
  const secret = secretOf(was)
  const n = f && f.secret === secret ? (f.n ?? 1) + 1 : 1
  unrenewed.set(key, { at: Date.now(), secret, gone, error, n })
}

function leadOf(a) {
  const v = a?.refreshLead
  return Number.isFinite(v) && v >= 0 ? Math.min(v, 24 * 3600e3) : RENEW_LEAD
}

// renewAt is when the sign-in at key is due to be renewed, 0 for never: an
// OAuth one with an expiry, of a plugin that renews through magpie.
function renewAt(a, stored) {
  if (typeof a?.refresh !== "function" || stored?.type !== "oauth") return 0
  const exp = Number(stored.expires)
  return Number.isFinite(exp) && exp > 0 ? Math.max(1, exp - leadOf(a)) : 0
}

// renewal is the renewal of the sign-in at key under way, started now if
// it is due and none is: null when it isn't due, or its last renewal
// failed less than retryAfter ago (or for good). A request and the
// background keep-alive (keepSignedIn) share it, so an account is renewed
// once at a time whoever asks.
function renewal(provider, key) {
  const a = auths().get(provider)?.auth
  const stored = readAuth()[key]
  const at = renewAt(a, stored)
  if (!at || Date.now() < at) return null
  let r = renewing.get(key)
  if (r) return r
  const f = unrenewed.get(key)
  if (f && f.secret === secretOf(stored) && (f.gone || Date.now() - f.at < retryAfter(f.n ?? 1))) return null
  pending++ // the host doesn't leave in the middle of it
  r = renew(provider, key, a).finally(() => {
    renewing.delete(key)
    // nor does magpie stop it in the middle of one (host.go's stop)
    send({ event: "renewing", count: renewing.size })
    done()
  })
  renewing.set(key, r)
  send({ event: "renewing", count: renewing.size })
  return r
}

// fresh renews the sign-in at key when it is due, waiting at most
// RENEW_WAIT for it: a renewal that takes longer goes on, and is kept when
// it ends, but what waits for it goes on with the sign-in as it is.
async function fresh(provider, key) {
  const r = renewal(provider, key)
  if (!r) return
  let t
  await Promise.race([r, new Promise((ok) => (t = setTimeout(ok, RENEW_WAIT)))])
  clearTimeout(t)
}

// keepSignedIn renews the sign-in at key if it is due, as a request would
// (renewal), with nothing waiting on the account: the gateway's
// keep-alive asks it of each account whose token comes due while nothing
// uses it (#5080: OrcaTerm's WeChat token lives two hours and can't be
// renewed once it ends). It waits for the renewal to end and says what
// came of it: due false, it wasn't due (renewed meanwhile, no expiry, a
// plugin that doesn't renew through magpie or said keepAlive false);
// renewed, the sign-in is a new one; error, why it couldn't be, and gone
// whether the vendor turned it away for good (the account is marked, as a
// request's renewal marks it). A renewal that failed lately isn't tried
// again here either: waiting says so.
async function keepSignedIn({ provider, account, proxy }) {
  return via.run(proxy ?? "", async () => {
    const key = accountKey(provider, account)
    const a = auths().get(provider)?.auth
    const was = readAuth()[key]
    if (!was) throw new Error("not signed in")
    const at = keepsAlive(a) ? renewAt(a, was) : 0
    if (!at || Date.now() < at) return { due: false, renewed: false, waiting: false, gone: false, error: "" }
    const r = renewal(provider, key)
    if (r) await r
    const now = readAuthState().all[key]
    if (now && changed(now, was)) return { due: true, renewed: true, waiting: false, gone: false, error: "" }
    const f = unrenewed.get(key)
    const failed = !!f && f.secret === secretOf(was)
    return { due: true, renewed: false, waiting: !r && failed, gone: failed && !!f.gone, error: failed ? f.error ?? "" : "" }
  })
}

// keepsAlive says the plugin's sign-ins are renewed in the background as
// they come due (keepSignedIn): any that renews through magpie, unless
// its auth hook says keepAlive: false (a renewal that costs the account
// something, or a refresh token another client holds too).
function keepsAlive(a) {
  return typeof a?.refresh === "function" && a?.keepAlive !== false
}

// Each magpie process runs a host of its own (the gateway, a CLI's quota
// or usage, a host restarting), and each renews the same sign-in when it
// comes due: a vendor that spends a refresh token once turned the second
// away, and the account was marked signed out under the token the first
// had saved (plugins #331). So a renewal is made under a lock file of the
// account's beside plugin-auth.json, held across the plugin's refresh,
// and the sign-in is read again under it. The lock names its holder's pid:
// one whose holder is gone is taken over at once, and one held longer
// than RENEW_LOCK_STALE by anyone. A pid is this host's to ask about only
// in its own pid namespace: two Kubernetes pods on one volume (a rolling
// update's old pod and new one) each run magpie as pid 1 and a host as a
// low pid of their own, and a pod asking after the other's pid asked after
// a process of its own, found none, and took a lock the other held: both
// spent one refresh token. So a lock names where its pid is (renewHost), and
// one from elsewhere is judged by its age alone.
const RENEW_LOCK_STALE = 60 * 1000

// renewHost is where this host's pid means something: the machine's (or
// pod's) name and, on Linux, its pid namespace.
const renewHost = (() => {
  let ns = ""
  try {
    ns = fs.readlinkSync("/proc/self/ns/pid")
  } catch {}
  return `${os.hostname()} ${ns}`.trim()
})()

function alive(pid) {
  try {
    process.kill(pid, 0)
    return true
  } catch (e) {
    return e?.code === "EPERM" // someone else's process
  }
}

async function lockRenew(key) {
  const lock = `${authPath}.renew-${createHash("sha256").update(key).digest("hex").slice(0, 16)}.lock`
  const me = JSON.stringify({ pid: process.pid, host: renewHost, at: Date.now(), n: randomUUID() })
  fs.mkdirSync(path.dirname(authPath), { recursive: true })
  for (let start = Date.now(), wait = 10; ; wait = Math.min(wait * 2, 500)) {
    try {
      fs.writeFileSync(lock, me, { flag: "wx", mode: 0o600 })
      return () => {
        try {
          if (fs.readFileSync(lock, "utf8") === me) fs.unlinkSync(lock)
        } catch {}
      }
    } catch (e) {
      if (!["EEXIST", "EPERM", "EACCES"].includes(e?.code)) throw e
    }
    let raw = null
    let h = null
    let age = 0
    try {
      raw = fs.readFileSync(lock, "utf8")
      age = Date.now() - fs.statSync(lock).mtimeMs
      h = JSON.parse(raw)
    } catch {}
    // a holder gone, or one too long at it; a file not written yet is
    // judged by its age, and one whose pid is another pod's or machine's
    // by its own time (a host before renewHost names none: its pid is asked
    // after here, as it was)
    const here = h?.host === undefined || h.host === renewHost
    const stale = Number.isInteger(h?.pid) ? (here && !alive(h.pid)) || Date.now() - h.at > RENEW_LOCK_STALE : age > RENEW_LOCK_STALE
    if (raw !== null && (stale || Date.now() - start > 2 * RENEW_LOCK_STALE)) {
      try {
        if (fs.readFileSync(lock, "utf8") === raw) fs.unlinkSync(lock)
      } catch {}
      continue
    }
    await new Promise((ok) => setTimeout(ok, wait))
  }
}

// changed says the sign-in now isn't was any longer: signed out, signed
// in again or renewed, by this host or another.
function changed(now, was) {
  return !now || secretOf(now) !== secretOf(was) || now.access !== was.access
}

async function renew(provider, key, a) {
  const was = readAuth()[key]
  const at = renewAt(a, was)
  if (!at || Date.now() < at) return // renewed meanwhile
  const unlock = await lockRenew(key)
  try {
    await renewLocked(provider, key, a, was)
  } finally {
    unlock()
  }
}

async function renewLocked(provider, key, a, was) {
  // read past authSeen: another host renewed it while this one waited,
  // and its token stands
  const now = readAuthState().all[key]
  if (changed(now, was)) {
    unrenewed.delete(key)
    forgetAccount(key)
    return
  }
  const at = renewAt(a, now)
  if (!at || Date.now() < at) return
  was = now
  let got
  try {
    got = await inScope(provider, key, () => a.refresh(JSON.parse(JSON.stringify(was)), provider))
  } catch (e) {
    // the token was spent by a host that didn't wait for this one's lock
    // (a magpie from before it) and saved what it got: not signed out
    if (changed(readAuthState().all[key], was)) {
      send({ event: "log", level: "error", message: `${auths().get(provider)?.spec ?? provider}: auth.refresh: ${e?.message ?? e} (renewed elsewhere meanwhile)` })
      forgetAccount(key)
      return
    }
    failedRenewal(key, was, e?.signIn === "expired", String(e?.message ?? e))
    if (e?.signIn === "expired") send({ event: "signIn", provider, account: key, said: "expired" })
    send({ event: "log", level: "error", message: `${auths().get(provider)?.spec ?? provider}: auth.refresh: ${e?.message ?? e}` })
    return
  }
  if (!got || typeof got !== "object") {
    // nothing to keep while it is due: not renewed, and not asked again
    // at once (failedRenewal)
    failedRenewal(key, was, false, "")
    return
  }
  // signed out, signed in again or renewed by another host while it ran:
  // that one stands. Checked under the lock the save is made under, so
  // nothing comes between the two.
  const { type: _t, ...fields } = got
  const kept = await changeAuth((all) => {
    if (changed(all[key], was)) return false
    all[key] = { ...was, ...fields, type: "oauth" }
  })
  if (kept === false) return
  unrenewed.delete(key)
  forgetAccount(key)
  send({ event: "auth", provider, account: key })
  send({ event: "signIn", provider, account: key, said: "renewed" })
}

// ---- requests ----------------------------------------------------------------

// options is what the provider's auth loader returned for the account at
// key, run once per sign-in as OpenCode runs it once per start. Run in
// the account's scope, whatever the loader keeps (its fetch) saves to it.
async function options(provider, key) {
  await fresh(provider, key)
  // read first: a sign-in another magpie saved since drops the loader
  // made from the one before (noteAuth)
  const stored = readAuth()[key]
  if (loaders.has(key)) return loaders.get(key)
  const a = auths().get(provider)?.auth
  let opts = {}
  if (a?.loader && stored) {
    const p = await infoOf(provider, key)
    opts = (await inScope(provider, key, () => a.loader(async () => authCopy(key), JSON.parse(JSON.stringify(p))))) ?? {}
  }
  if (!opts.apiKey && stored?.type === "api") opts = { ...opts, apiKey: stored.key }
  loaders.set(key, opts)
  return opts
}

async function load({ provider, account, proxy }) {
  const key = accountKey(provider, account)
  const o = await via.run(proxy ?? "", () => inScope(provider, key, () => options(provider, key)))
  return {
    baseURL: typeof o.baseURL === "string" ? o.baseURL : "",
    apiKey: typeof o.apiKey === "string" ? o.apiKey : "",
    headers: o.headers && typeof o.headers === "object" ? o.headers : {},
    fetch: typeof o.fetch === "function",
  }
}

// ---- usage -------------------------------------------------------------------

// usage is how much of its allowance the account at key has used, as the
// plugin's auth.usage says (magpie's own hook, which OpenCode ignores):
//   auth.usage(getAuth, provider) → {
//     plan?, user? (the account as the service names it), until?,
//     renew?: "auto" | "off", balance?, error?,
//     resets?: { count, until?, byWindow?, fiveHour?, weekly?, team?
//       (a team plan's, not the person's own), each?: [{ until? (ISO or
//       ms; none: it never runs out), window? ("fiveHour", "weekly") }]
//       (every reset still to be used) } (the rate-limit resets the
//       account may spend),
//     windows?: [{ name, used (percent, 0–100; left out: the plugin didn't
//       say, which is no share and not 0%), resetsAt? (ISO or ms),
//       resetSecs?, display?, amount? / limit? / unit? (the window's own
//       count, when it counts in amounts: amount of limit used, in unit,
//       "credits"), span? (seconds the window runs), model? (a
//       word in the ids of the only models it counts), models? / notModels?
//       (the ids it counts, or all but these; a bare id and its id@size
//       count as one), family? (the model family a model's own window
//       belongs to: "Gemini"), pool? (the group of models sharing one
//       allowance: "Gemini", "Claude & GPT"; on the group's own windows,
//       with no family, and on each of its models' — the GUI shows the
//       group's windows in place of its models', as for magpie's built-in
//       Antigravity), aside? (using it up doesn't
//       stop the account), runsOutAt? (ISO or ms: when the soonest
//       credits left in it are lost, apart from its reset, as WorkBuddy's
//       packages end) / runsOut? (how many are lost then), credits?
//       (used up, the account goes on past it on credits it holds, which
//       its vendor spends after it, as a Kiro trial's: routing asks the
//       provider's other accounts first; sent with aside, which a magpie
//       that doesn't know it goes by), balance?
//       (how much is left, in unit, of a prepaid wallet with no total:
//       sent without used, it is no share, yet routing knows the account
//       and puts the one with the most left first among the provider's
//       accounts in the same unit, #3950) }],
//     signIn?: "expired" | "kept" | "renewed" (what the read means for the
//       account's sign-in, as a model request's X-Magpie-Sign-In says;
//       without it an error saying to sign in again marks the account and
//       a clean read clears it)
//   }
// Run in the account's scope, a token it renews is saved to that account.
// A caller that gave up (signal) is what the hook is handed as its own
// options, so a vendor read that supports stopping stops (#4678).
async function usage({ provider, account, proxy }, signal) {
  return via.run(proxy ?? "", () => usageOf(provider, account, signal))
}

async function usageOf(provider, account, signal) {
  const a = auths().get(provider)?.auth
  if (typeof a?.usage !== "function") throw new Error(`${provider}'s plugin doesn't tell its usage`)
  const key = accountKey(provider, account)
  if (!readAuth()[key]) throw new Error("not signed in")
  await fresh(provider, key)
  const p = await infoOf(provider, key)
  const u = (await inScope(provider, key, () => a.usage(async () => authCopy(key), JSON.parse(JSON.stringify(p)), signal === undefined ? undefined : { signal }))) ?? {}
  const when = (v) => {
    if (v === undefined || v === null || v === "") return ""
    const d = new Date(typeof v === "number" && v < 1e11 ? v * 1000 : v)
    return isNaN(d) ? "" : d.toISOString()
  }
  const text = (v) => (typeof v === "string" ? v : "")
  const num = (v) => (typeof v === "number" && isFinite(v) ? v : 0)
  const ids = (v) => (Array.isArray(v) ? v.filter((x) => typeof x === "string") : [])
  return {
    plan: text(u.plan),
    until: when(u.until),
    renew: u.renew === "auto" || u.renew === "off" ? u.renew : "",
    balance: text(u.balance),
    error: text(u.error),
    user: text(u.user),
    signIn: ["expired", "kept", "renewed"].includes(u.signIn) ? u.signIn : "",
    resets:
      u.resets && typeof u.resets === "object"
        ? {
            count: num(u.resets.count), until: when(u.resets.until), byWindow: !!u.resets.byWindow,
            fiveHour: num(u.resets.fiveHour), weekly: num(u.resets.weekly), team: !!u.resets.team,
            each: (Array.isArray(u.resets.each) ? u.resets.each : [])
              .filter((r) => r && typeof r === "object")
              .map((r) => ({ until: when(r.until), window: r.window === "fiveHour" || r.window === "weekly" ? r.window : "" })),
          }
        : null,
    windows: (Array.isArray(u.windows) ? u.windows : []).map((w) => ({
      name: text(w?.name),
      used: w?.used == null ? null : Math.max(0, num(w?.used)), // past 100 when overspent; null: not said
      resetsAt: when(w?.resetsAt),
      resetSecs: Math.max(0, Math.round(num(w?.resetSecs))),
      display: text(w?.display),
      amount: Math.max(0, num(w?.amount)),
      limit: Math.max(0, num(w?.limit)),
      unit: text(w?.unit),
      span: Math.max(0, num(w?.span)),
      model: text(w?.model),
      models: ids(w?.models),
      notModels: ids(w?.notModels),
      family: text(w?.family),
      pool: text(w?.pool),
      aside: !!w?.aside,
      runsOutAt: when(w?.runsOutAt),
      runsOut: Math.max(0, num(w?.runsOut)),
      credits: !!w?.credits,
      balance: typeof w?.balance === "number" && isFinite(w.balance) ? Math.max(0, w.balance) : null, // null: not said
    })),
  }
}

// ---- check-in ----------------------------------------------------------------

// checkin presses the vendor's daily check-in (签到) for the account at key,
// as the plugin's auth.checkin does it (magpie's own hook, which OpenCode
// ignores; Lemon on Discord: 签到 belongs in the plugins):
//   auth.checkin(getAuth, provider) → {
//     outcome: "claimed" (checked in now) | "done" (in already today) |
//       "ineligible" (the account can't take part) | "inactive" (no
//       check-in running) | "captcha" (the vendor asks for a captcha: the
//       user checks in in its own app; never solved) | "failed",
//     credit? (what it gave), streak? (days in a row), message?,
//     until? (when the vendor's own day turns: an RFC 3339 time or epoch
//       milliseconds; Qoder's daily credits open at 10:00 UTC+8)
//   }
// magpie asks once a Beijing day per account while the user has it on, and
// again later that day after a failure, or at until; a hook that throws is
// a failure.
// Run in the account's scope, a token it renews is saved to that account.
async function checkin({ provider, account, proxy }) {
  return via.run(proxy ?? "", () => checkinOf(provider, account))
}

const checkinOutcomes = ["claimed", "done", "ineligible", "inactive", "captcha", "failed"]

async function checkinOf(provider, account) {
  const a = auths().get(provider)?.auth
  if (typeof a?.checkin !== "function") throw new Error(`${provider}'s plugin doesn't check in`)
  const key = accountKey(provider, account)
  if (!readAuth()[key]) throw new Error("not signed in")
  await fresh(provider, key)
  const p = await infoOf(provider, key)
  const r = (await inScope(provider, key, () => a.checkin(async () => authCopy(key), JSON.parse(JSON.stringify(p))))) ?? {}
  const num = (v) => (typeof v === "number" && isFinite(v) ? Math.max(0, v) : 0)
  const message = typeof r.message === "string" ? r.message : ""
  // a time that doesn't read is not told
  const t = typeof r.until === "number" ? r.until : typeof r.until === "string" ? Date.parse(r.until) : NaN
  const until = Number.isFinite(t) && t > 0 ? new Date(t).toISOString() : ""
  if (!checkinOutcomes.includes(r.outcome)) {
    const said = r.outcome === undefined || r.outcome === null ? "no outcome" : JSON.stringify(r.outcome)
    return { outcome: "failed", credit: 0, streak: 0, message: message ? `the plugin answered ${said}: ${message}` : `the plugin answered ${said}`, until }
  }
  return { outcome: r.outcome, credit: num(r.credit), streak: Math.round(num(r.streak)), message, until }
}

// ---- calls made outside magpie -------------------------------------------------

// calls are the account's calls made outside magpie (the vendor's own
// editor or CLI) from since on, for magpie's Usage page, as the plugin's
// auth.calls gives them (magpie's own hook, which OpenCode ignores):
//   auth.calls(getAuth, provider, {since, signal}) → {calls: [{
//     rid (the vendor's id of the call: magpie counts each once),
//     t (epoch ms), model, in (input tokens, cache apart), out,
//     cache_read, cache_write, cost? (USD, as the vendor priced it),
//     session? (the conversation it was part of)
//   }]}
// A call without an rid or a time is dropped. A hook that throws fails
// the reading: magpie asks again from the same time.
// Run in the account's scope, a token it renews is saved to that account.
async function calls({ provider, account, proxy, since }, signal) {
  return via.run(proxy ?? "", () => callsOf(provider, account, since, signal))
}

async function callsOf(provider, account, since, signal) {
  const a = auths().get(provider)?.auth
  if (typeof a?.calls !== "function") throw new Error(`${provider}'s plugin doesn't list its calls`)
  const key = accountKey(provider, account)
  if (!readAuth()[key]) throw new Error("not signed in")
  await fresh(provider, key)
  const p = await infoOf(provider, key)
  const r = (await inScope(provider, key, () => a.calls(async () => authCopy(key), JSON.parse(JSON.stringify(p)), { since: Number(since) || 0, ...(signal === undefined ? {} : { signal }) }))) ?? {}
  const num = (v) => (typeof v === "number" && isFinite(v) ? Math.max(0, v) : 0)
  const text = (v) => (typeof v === "string" ? v : "")
  return {
    calls: (Array.isArray(r.calls) ? r.calls : [])
      .filter((c) => c && typeof c === "object" && text(c.rid) && num(c.t) > 0)
      .map((c) => ({
        rid: text(c.rid),
        t: Math.round(num(c.t)),
        model: text(c.model),
        in: Math.round(num(c.in)),
        out: Math.round(num(c.out)),
        cache_read: Math.round(num(c.cache_read)),
        cache_write: Math.round(num(c.cache_write)),
        cost: typeof c.cost === "number" && isFinite(c.cost) && c.cost >= 0 ? c.cost : null,
        session: text(c.session),
      })),
  }
}

// sdkHeaders are what the AI SDK package the model is on sends of the key.
function sdkHeaders(npm, key) {
  if (!key) return {}
  if (npm === "@ai-sdk/anthropic" || npm === "@ai-sdk/google-vertex/anthropic") return { "x-api-key": key }
  if (npm === "@ai-sdk/google") return { "x-goog-api-key": key }
  return { authorization: `Bearer ${key}` }
}

// bodyOf is the body as OpenCode hands it to a plugin's fetch: the string
// the AI SDK built, as the plugins reshape only a string body. One that
// isn't UTF-8 text stays bytes.
function bodyOf(b64) {
  if (!b64) return undefined
  const b = Buffer.from(b64, "base64")
  try {
    return new TextDecoder("utf-8", { fatal: true, ignoreBOM: true }).decode(b)
  } catch {
    return b
  }
}

async function doFetch(id, record, params) {
  const key = accountKey(params.provider, params.account)
  return via.run(params.proxy ?? "", () => inScope(params.provider, key, () => fetchAs(id, record, key, params)))
}

// staleAccess is what a bare 401 says of the OAuth sign-in at key that
// had a refresh token when the request began (before): "kept" when the
// access token alone was turned away, so the next request renews it, and
// nothing, for the 401 to mark the account, when the refresh is what
// failed. An access token the vendor dropped before the expiry it was
// stored with (an idle account, another client trading the same refresh
// token: plugins#618) isn't the sign-in gone. So such a 401, with the
// access token the request began with still the stored one and not due
// yet, makes it due (expires 1) and keeps the sign-in: the plugin's
// loader, or magpie's auth.refresh, renews it on the next request. The
// account is marked when the token was renewed for this request and
// still turned away, or was due and the plugin didn't renew it; a
// refresh the vendor refuses says so itself (signIn "expired"). A
// sign-in that can't be read now is unknown: kept, not marked.
async function staleAccess(key, before) {
  const now = readAuthState()
  if (!now.known) return "kept"
  if (before?.type !== "oauth" || !before.refresh) return ""
  const cur = now.all[key]
  if (cur?.type !== "oauth" || cur.refresh !== before.refresh || cur.access !== before.access) return ""
  const exp = Number(before.expires)
  if (!Number.isFinite(exp) || exp <= Date.now()) return ""
  await changeAuth((all) => {
    const a = all[key]
    if (a?.type !== "oauth" || a.access !== before.access || a.refresh !== before.refresh) return false
    all[key] = { ...a, expires: 1 }
  })
  forgetAccount(key)
  return "kept"
}

// callerOf is the agent a request came from, as magpie's headers for the
// plugin say it: {agent: "claude", name: "Claude Code", via: [computers]},
// via empty for an agent on this computer, else the computers another
// magpie passed it on through, the agent's first. undefined when magpie
// said none.
function callerOf(headers) {
  const dec = (v) => {
    try {
      return decodeURIComponent(v ?? "")
    } catch {
      return ""
    }
  }
  const agent = dec(headers?.["x-magpie-caller-agent"])
  if (!agent) return undefined
  const via = String(headers["x-magpie-caller-via"] ?? "").split(",").map(dec).filter(Boolean)
  return { agent, name: dec(headers["x-magpie-caller-name"]) || agent, via }
}

async function fetchAs(id, record, key, params) {
  const { controller: ctl, gate } = record
  const { provider, model, npm, url, method, headers, body, session } = params
  // the sign-in as the request found it, before its loader or magpie
  // renewed it: a 401 is judged against it (staleAccess)
  const before = key ? readAuth()[key] : undefined
  try {
    const o = await options(provider, key)
    checkFetch(record)
    const h = new Headers()
    for (const [k, v] of Object.entries(sdkHeaders(npm, o.apiKey))) h.set(k, v)
    for (const [k, v] of Object.entries(o.headers ?? {})) h.set(k, String(v))
    for (const [k, v] of Object.entries(headers ?? {})) h.set(k, v)
    // chat.headers: what the plugins add to the request, theirs winning.
    // They are told the provider as info builds it once for the account
    // (infoOf), the one object every request is told, as OpenCode tells
    // them the provider it keeps
    const said = hooks.filter((x) => typeof x.hooks["chat.headers"] === "function")
    let p, m
    if (said.length) {
      p = await infoOf(provider, key)
      checkFetch(record)
      m = p.models[model] ?? { id: model, providerID: provider, api: { id: model, npm } }
    }
    // input.agent stays OpenCode's own agent ("build"), which a plugin may
    // go by; who asked is magpie's input.caller
    const caller = callerOf(headers)
    for (const x of said) {
      const fn = x.hooks["chat.headers"]
      const out = { headers: {} }
      try {
        await fn(
          {
            sessionID: session ?? "",
            agent: "build",
            ...(caller ? { caller: structuredClone(caller) } : {}),
            model: m,
            provider: { source: "custom", info: p, options: o },
            message: { id: "", sessionID: session ?? "", role: "user", time: { created: Date.now() }, agent: "build", model: { providerID: provider, modelID: model } },
          },
          out,
        )
      } catch (e) {
        send({ event: "log", level: "error", message: `${x.spec}: chat.headers: ${e?.message ?? e}` })
      }
      checkFetch(record)
      for (const [k, v] of Object.entries(out.headers)) h.set(k, String(v))
    }
    const f = typeof o.fetch === "function" ? o.fetch : fetch
    // the fetch is owned since its message was read: an abort that came while
    // this setup was awaiting must not let the upstream start
    checkFetch(record)
    const res = await f(url, {
      method: method ?? "POST",
      headers: h,
      body: bodyOf(body),
      signal: ctl.signal,
    })
    record.unconsumedBody = res.body
    checkFetch(record)
    transitionFetch(record, "streaming")
    const rh = {}
    res.headers.forEach((v, k) => (rh[k] = v))
    if (res.status === 401 && !("x-magpie-sign-in" in rh) && key) {
      const said = await staleAccess(key, before)
      if (said) rh["x-magpie-sign-in"] = said
    }
    const head = headCost(rh)
    if (head.entries > HEAD_ENTRIES || head.n > HEAD_MAX) {
      // the reply's head is past magpie's envelope: give the fetch up rather
      // than write a line the host would refuse, and let the body go
      throw new Error(`the reply's headers are past the host's head envelope (${HEAD_MAX} bytes or ${HEAD_ENTRIES} entries)`)
    }
    await sendForFetch(record, { id, event: "head", status: res.status, headers: rh })
    checkFetch(record)
    if (res.body) {
      record.unconsumedBody = undefined
      // an abort gives up this fetch's own pending write, so its wait ends and
      // the loop leaves rather than parking on a stream that never drains
      for await (const chunk of res.body) {
        checkFetch(record)
        // a chunk the stream handed over is this reader's: viewed, not copied
        const bytes = Buffer.isBuffer(chunk) ? chunk : ArrayBuffer.isView(chunk) ? Buffer.from(chunk.buffer, chunk.byteOffset, chunk.byteLength) : Buffer.from(chunk)
        for (let off = 0; off < bytes.length; ) {
          // one frame at a time, no more than the reader has credited: a
          // reader that stopped leaves the upstream paused here
          const n = await gate.take(Math.min(MAX_FRAME, bytes.length - off))
          checkFetch(record)
          if (n <= 0) break
          const data = bytes.subarray(off, off + n).toString("base64")
          gate.spend(n + FRAME_OVERHEAD)
          await sendForFetch(record, chunkLine(id, data))
          checkFetch(record)
          off += n
        }
      }
    }
    checkFetch(record)
    transitionFetch(record, "finishing")
    await sendForFetch(record, { id, result: null })
  } catch (e) {
    transitionFetch(record, "finishing")
    // a fetch hook that threw on its sign-in (a refresh the vendor turned
    // away) marks the account, as a built-in's refused refresh marked it
    if (["expired", "kept", "renewed"].includes(e?.signIn) && key) send({ event: "signIn", provider, account: key, said: e.signIn })
    // a fetch the host already gave up on needs no late error: its slot is
    // gone, and queueing an uncancellable line for it would only be noise
    await sendForFetch(record, { id, error: { message: String(e?.message ?? e) } })
  }
}

// ---- the loop ----------------------------------------------------------------

const handlers = {
  gatewayStates() {
    return Object.fromEntries(gatewayHooks.map(({ spec, state }) => [spec, {
      calls: state.calls, failures: state.failures,
      avgMillis: state.calls ? state.totalMillis / state.calls : 0,
      lastOutcome: state.lastOutcome, lastBytes: state.lastBytes,
    }]))
  },
  async init(p) {
    authPath = p.authPath
    modelsDevPath = p.modelsDevPath ?? ""
    piPath = p.piPath ?? ""
    directory = p.directory ?? directory
    userConfig = p.config ?? {}
    prefer = p.prefer ?? {}
    chosen = p.options ?? {}
    if (p.loadMillis > 0) loadWait = p.loadMillis
    if (p.listMillis > 0) listWait = p.listMillis
    await loadPlugins(p.plugins ?? [])
    // from here on, when magpie holds this host as the one calls go to
    watchHeavy(p.heavy)
    return { plugins: loaded.map(withProviders) }
  },
  providers: (p, signal) => providers(p ?? {}, signal),
  prompt: async (p) => ({ prompt: await nextPrompt(p.provider, p.method, p.inputs ?? {}) }),
  validate: (p) => ({ error: validate(p.provider, p.method, p.key, p.value) }),
  authorize,
  callback,
  tool: runTool,
  search: runSearch,
  apiKey,
  importFiles,
  cliInstalled,
  installCli,
  load,
  usage,
  checkin,
  calls,
  keepSignedIn,
  // setOptions takes the user's option values over, as init did: the
  // plugin reads them when it next asks (#2038)
  setProviderOptions(p) {
    chosen = p?.options ?? {}
    // a models hook may list by them
    infos.clear()
    infoGen++
    return null
  },
  // check tries one account as a request would: its loader, then its
  // models as the plugin lists them for it, then its usage, which asks the
  // vendor of the account itself. refused is the models hook saying the
  // vendor turned the sign-in away. A models hook may fall back to a list
  // it keeps when the vendor can't be reached, so a check that reached
  // none of the places it asked proves nothing, and fails.
  async check(p, signal) {
    const key = accountKey(p.provider, p.account)
    const r = { reached: false, failed: null, refused: null }
    return via.run(p.proxy ?? "", () => reach.run(r, async () => {
      await load({ provider: p.provider, account: key })
      const pi = await info(p.provider, key, true, undefined, false, signal)
      const u = typeof auths().get(p.provider)?.auth?.usage === "function" ? await usageOf(p.provider, key, signal) : null
      if (r.failed && !r.reached) throw new Error(`couldn't reach ${pi.name}: ${r.failed?.message ?? r.failed}`)
      return { models: Object.keys(pi.models), usage: u, refused: r.refused ?? "" }
    }))
  },
  // import keeps a sign-in made elsewhere (a built-in subscription's, moved
  // onto its plugin) as one more account, or as the account it already is
  // import keeps a sign-in made elsewhere as one of provider's accounts;
  // with a key, as that account again (one taken back)
  async import(p) {
    if (p.key && providerOf(p.key) === p.provider) {
      await setAuth(p.key, p.auth)
      return { account: p.key }
    }
    const key = freshKey(p.provider)
    await setAuth(key, p.auth)
    return { account: await settle(p.provider, key) }
  },
  // take gives the accounts named (else every account of the provider) and
  // forgets them in one step: nothing renews a token between the two
  async take(p) {
    const out = {}
    await changeAuth((all) => {
      for (const k of p.accounts?.length ? p.accounts : accountsOf(all, p.provider)) {
        if (!(k in all)) continue
        out[k] = all[k]
        delete all[k]
      }
      if (!Object.keys(out).length) return false
    })
    for (const k of Object.keys(out)) {
      forgetAccount(k)
      send({ event: "auth", provider: providerOf(k), account: k })
    }
    return { auths: out }
  },
  // signOut forgets the account named, else every account of the provider
  async signOut(p) {
    const keys = p.account ? [p.account] : accountsOf(readAuth(), p.provider)
    for (const k of keys) await removeAuth(k)
    return null
  },
  reload(p) {
    for (const k of p.account ? [p.account] : accountsOf(readAuth(), p.provider)) forgetAccount(k)
    return null
  },
}

// Everything waits for init; a request is answered even when magpie has
// closed stdin behind it, the host leaving once nothing is pending.
let ready
let readyState
const readyWaiters = new Set()
function setReady(promise) {
  readyState = undefined
  ready = promise
  const settle = (state) => {
    readyState = state
    for (const waiter of readyWaiters) waiter(state)
    readyWaiters.clear()
  }
  ready.then(
    () => settle({}),
    (error) => settle({ error }),
  )
}
function waitForReady(record) {
  const signal = record.controller.signal
  if (!ready) return Promise.reject(new Error("the host has not been initialised"))
  return new Promise((resolve, reject) => {
    const finish = (state) => {
      readyWaiters.delete(finish)
      signal.removeEventListener("abort", abort)
      record.readyCleanup = null
      if ("error" in state) reject(state.error)
      else resolve()
    }
    const abort = () => finish({ error: new DOMException("Aborted", "AbortError") })
    record.readyCleanup = abort
    if (signal.aborted) abort()
    else if (readyState) finish(readyState)
    else {
      readyWaiters.add(finish)
      signal.addEventListener("abort", abort, { once: true })
    }
  })
}
let pending = 0
let closed = false
const done = () => {
  if (--pending === 0 && closed) process.exit(0)
}

const gatewayInflight = new Map() // RPC id -> AbortController
// A sign-in's steps are given up on by magpie as a gateway request is
// (host.go's cancellable): the user cancelled it, or it timed out. Its
// answer is then nobody's, and it counts as done at once, whether the
// plugin's wait ends or not (#5016).
const signInSteps = new Set(["authorize", "callback", "apiKey", "importFiles"])
const signInInflight = new Map() // RPC id -> AbortController
const rpcInflight = new Map() // an ordinary RPC's id -> its AbortController (#4678)
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
    const id = msg.params?.id
    const record = inflight.get(id)
    if (record) transitionFetch(record, "cancelled")
    gatewayInflight.get(id)?.abort()
    signInInflight.get(id)?.abort()
    rpcInflight.get(id)?.abort()
    return
  }
  if (msg.method === "credit") {
    const record = inflight.get(msg.params?.id)
    if (record?.state === "streaming") record.gate.add(msg.params?.n ?? 0)
    return
  }
  pending++
  if (msg.method === "init") {
    setReady(handlers.init(msg.params ?? {}))
    ready.then(
      (result) => send({ id: msg.id, result }),
      (e) => send({ id: msg.id, error: { message: String(e?.message ?? e) } }),
    ).finally(done)
    return
  }
  if (msg.method === "gatewayRequest") {
    const ctl = new AbortController()
    gatewayInflight.set(msg.id, ctl)
    // Own cancellation before waiting for init, just as provider fetches do.
    bounded(ready ?? Promise.reject(new Error("the host has not been initialised")), ctl.signal)
      .then(() => gatewayRequest(msg.params ?? {}, ctl.signal))
      .then(
        (result) => {
          if (!ctl.signal.aborted) return send({ id: msg.id, result })
          if (msg.params?.operation === "begin" && result?.id) closeGateway(gatewayRuns.get(result.id))
        },
        () => { if (!ctl.signal.aborted) return send({ id: msg.id, result: null }) },
      )
      .finally(() => { gatewayInflight.delete(msg.id); done() })
    return
  }
  if (msg.method === "fetch") {
    const id = msg.id
    // the fetch is owned the moment its message is read, before the async
    // setup below: an abort that comes before the upstream is reached still
    // stops it, and one that comes during the await is not missed
    const record = {
      state: "setup", controller: new AbortController(),
      gate: makeGate(msg.params?.window > 0 ? msg.params.window : 512 << 10),
      pendingSend: null, readyCleanup: null, unconsumedBody: undefined,
    }
    inflight.set(id, record)
    // waiting for the host to be ready is the host's own await: an abort must
    // end it too, so a request whose setup never finishes still runs its
    // finally and lets its controller, gate and body go. The shared setup
    // promise itself is not cancelled — only this request's wait on it.
    const start = () => {
      checkFetch(record)
      return doFetch(id, record, msg.params ?? {})
    }
    waitForReady(record).then(start)
      .catch((e) => {
        transitionFetch(record, "finishing")
        return sendForFetch(record, { id, error: { message: String(e?.message ?? e) } })
      })
      .finally(() => {
        transitionFetch(record, "finished")
        inflight.delete(id)
        done()
      })
    return
  }
  if (signInSteps.has(msg.method)) {
    const ctl = new AbortController()
    signInInflight.set(msg.id, ctl)
    let over = false
    const end = () => {
      if (over) return
      over = true
      signInInflight.delete(msg.id)
      done()
    }
    ctl.signal.addEventListener("abort", end, { once: true })
    bounded(ready ?? Promise.reject(new Error("the host has not been initialised")), ctl.signal)
      .then(() => handlers[msg.method](msg.params ?? {}, ctl.signal))
      .then(
        (result) => { if (!ctl.signal.aborted) send({ id: msg.id, result: result ?? null }) },
        (e) => { if (!ctl.signal.aborted) send({ id: msg.id, error: { message: String(e?.message ?? e) } }) },
      )
      .finally(end)
    return
  }
  const wait = ready ?? Promise.reject(new Error("the host has not been initialised"))
  const fn = handlers[msg.method]
  if (!fn) {
    send({ id: msg.id, error: { message: `no method ${msg.method}` } })
    done()
    return
  }
  // An ordinary RPC is told when magpie gives up on it, as a fetch and a
  // gateway request are: the handler is handed the signal, so the work
  // that supports stopping stops (#4678), and its answer, nobody's then,
  // is not sent. It stays pending until the handler ends all the same: a
  // handler that ignores the signal may still be renewing or saving a
  // sign-in, and a host whose stdin closed must not exit under it.
  const ctl = new AbortController()
  rpcInflight.set(msg.id, ctl)
  wait
    .then(() => fn(msg.params ?? {}, ctl.signal))
    .then(
      (result) => { if (!ctl.signal.aborted) send({ id: msg.id, result: result ?? null }) },
      (e) => { if (!ctl.signal.aborted) send({ id: msg.id, error: { message: String(e?.message ?? e) } }) },
    )
    .finally(() => {
      rpcInflight.delete(msg.id)
      done()
    })
})
rl.on("close", () => {
  closed = true
  if (pending === 0) process.exit(0)
})
