// opens.js is preloaded into the plugin host (bun run --preload) and into
// every Bun program the host and its plugins start with a script of their
// own, so it is in each of them before any plugin code.
//
// It refuses a command that opens nothing: `cmd /c start "" ""` (or a
// start with only a title, or none), explorer with no path or an empty
// one, `open ""` and `xdg-open ""`. Each of those opens the program's own
// folder instead, which for a program a plugin starts is plugin-work: on
// Windows Explorer came up on %USERPROFILE%\.config\magpie\plugin-work
// about 10s after every start of the host, and again each minute after
// it was closed (unbrokefix on 𝕏, magpie 0.1.4732). A page plugin ran
// `magpie-bun -e <scan>` with no folder, the scan called every installed
// plugin's factory, and one of them opened an empty target. Such a
// command is run as a program that does nothing and ends at once, so the
// caller sees one that finished, and stderr says which it was. A real path
// or URL opens as before.
//
// node:child_process starts every program through Bun.spawn and
// Bun.spawnSync (exec and execFile, their Sync forms and spawn), so these
// two see them all.

import path from "node:path"

const self = import.meta.path

// who names the plugin running, when the host can tell (host.js sets it)
const WHO = Symbol.for("magpie.spawnedBy")

const base = (p) =>
  String(p ?? "")
    .split(/[\\/]/)
    .pop()
    .trim()
    .toLowerCase()
    .replace(/\.(exe|com|cmd|bat)$/, "")

// cmdWords splits a cmd.exe line into its words, as cmd and start read
// them: a word in double quotes keeps its spaces and may be empty, and ^
// escapes the character after it outside quotes.
function cmdWords(line) {
  const out = []
  let i = 0
  while (i < line.length) {
    while (i < line.length && /\s/.test(line[i])) i++
    if (i >= line.length) break
    let v = ""
    let quoted = line[i] === '"'
    let inQ = false
    while (i < line.length && (inQ || !/\s/.test(line[i]))) {
      const c = line[i]
      if (c === '"') inQ = !inQ
      else if (c === "^" && !inQ && i + 1 < line.length) v += line[++i]
      else v += c
      i++
    }
    out.push({ v, quoted })
  }
  return out
}

// shWords splits a POSIX shell line into its words: '…', "…" (with \
// escapes) and \ outside quotes.
function shWords(line) {
  const out = []
  let i = 0
  while (i < line.length) {
    while (i < line.length && /\s/.test(line[i])) i++
    if (i >= line.length) break
    let v = ""
    while (i < line.length && !/\s/.test(line[i])) {
      const c = line[i]
      if (c === "'") {
        const j = line.indexOf("'", i + 1)
        const end = j < 0 ? line.length : j
        v += line.slice(i + 1, end)
        i = end + 1
      } else if (c === '"') {
        i++
        while (i < line.length && line[i] !== '"') {
          if (line[i] === "\\" && i + 1 < line.length) i++
          v += line[i++]
        }
        i++
      } else if (c === "\\" && i + 1 < line.length) {
        v += line[i + 1]
        i += 2
      } else {
        v += c
        i++
      }
    }
    out.push(v)
  }
  return out
}

// A line with more than one command in it (&, |, ;) is left alone: only a
// command that is nothing but an empty open is refused.
const chained = (line) => /[&|;<>\n]/.test(line.replace(/"[^"]*"/g, '""'))

// startEmpty is whether a cmd.exe start, given its words after "start",
// has no target: switches (/d takes a folder) and the title, the first
// quoted word, don't count.
function startEmpty(ws) {
  let i = 0
  if (ws[i]?.quoted) i++
  while (i < ws.length && !ws[i].quoted && ws[i].v.startsWith("/")) i += ws[i].v.toLowerCase() === "/d" ? 2 : 1
  // a title after the switches is a title too when a target follows
  return i >= ws.length || ws.slice(i).every((w) => w.v.trim() === "")
}

// noTarget is whether explorer's, open's or xdg-open's arguments name
// nothing: each is empty or a switch.
const noTarget = (args, sw) => args.every((a) => String(a).trim() === "" || sw.test(String(a)))

// emptyOpen is why argv opens nothing, or "".
export function emptyOpen(argv) {
  if (!Array.isArray(argv) || argv.length === 0) return ""
  const prog = base(argv[0])
  const args = argv.slice(1).map(String)
  if (prog === "explorer") return noTarget(args, /^\/(e|n|separate),?$/i) ? "explorer with no path" : ""
  if (prog === "open" || prog === "xdg-open") return noTarget(args, /^-/) ? `${prog} with no target` : ""
  if (prog === "cmd") {
    const c = args.findIndex((a) => /^\/[ck]$/i.test(a))
    if (c < 0) return ""
    let line = args.slice(c + 1).join(" ").trim()
    // /s takes the first and last quote off the line, as exec's
    // `cmd /d /s /c "<command>"` sends it
    if (args.slice(0, c).some((a) => /^\/s$/i.test(a)) && line.startsWith('"') && line.endsWith('"') && line.length > 1) line = line.slice(1, -1)
    if (chained(line)) return ""
    const ws = cmdWords(line)
    if (ws.length === 0) return ""
    const first = base(ws[0].v)
    if (first === "start" && startEmpty(ws.slice(1))) return "start with no target"
    if (first === "explorer" && noTarget(ws.slice(1).map((w) => w.v), /^\/(e|n|separate),?$/i)) return "explorer with no path"
    return ""
  }
  if (/^(sh|bash|dash|zsh|ash)$/.test(prog)) {
    const c = args.indexOf("-c")
    if (c < 0 || c + 1 >= args.length) return ""
    const line = args[c + 1]
    if (chained(line.replace(/'[^']*'/g, "''"))) return ""
    const ws = shWords(line)
    const first = base(ws[0])
    if ((first === "open" || first === "xdg-open") && noTarget(ws.slice(1), /^-/)) return `${first} with no target`
    return ""
  }
  return ""
}

// withPreload is argv with this file preloaded, when argv runs Bun on a
// script: `bun -e …`, `bun file.js`, `bun run file.js`. Bun's other
// commands (add, install, test, a package.json script) are left as they
// are. A Bun given --preload already is too.
const SCRIPT = /\.(m|c)?(j|t)sx?$/i
export function withPreload(argv) {
  if (!Array.isArray(argv) || argv.length < 2 || !self) return argv
  const p = String(argv[0])
  if (!/^(magpie-)?bun(-[\w.]+)?$/.test(base(p)) && path.resolve(p) !== process.execPath) return argv
  if (argv.some((a) => a === "--preload" || String(a).startsWith("--preload=") || a === "-r")) return argv
  const a1 = String(argv[1])
  if (["-e", "--eval", "-p", "--print"].includes(a1) || SCRIPT.test(a1)) return [argv[0], "--preload", self, ...argv.slice(1)]
  if (a1 === "run" && argv.length > 2 && SCRIPT.test(String(argv[2]))) return [argv[0], "run", "--preload", self, ...argv.slice(2)]
  return argv
}

// checked is argv as it is to run: a program that ends at once in place
// of an empty open, this file preloaded into a Bun script.
function checked(argv) {
  const why = emptyOpen(argv)
  if (why) {
    let who = ""
    try {
      who = globalThis[WHO]?.() ?? ""
    } catch {}
    refused[0]++
    process.stderr.write(`plugin host: refused to open an empty target (${why}${who ? `, from ${who}` : ""}): ${JSON.stringify(argv)}\n`)
    return [process.execPath, "-e", ""]
  }
  return withPreload(argv)
}

// refused counts the commands turned away, and emptyOpen is there for a
// test to ask, both under one symbol: {refused, emptyOpen}
const refused = [0]
if (!globalThis[Symbol.for("magpie.opens")]) {
  globalThis[Symbol.for("magpie.opens")] = { refused: () => refused[0], emptyOpen }
  for (const name of ["spawn", "spawnSync"]) {
    const run = Bun[name]
    Bun[name] = function (a, b) {
      if (Array.isArray(a)) return run.call(this, checked(a), b)
      if (a && typeof a === "object" && Array.isArray(a.cmd)) return run.call(this, { ...a, cmd: checked(a.cmd) }, b)
      return run.call(this, a, b)
    }
  }
}
