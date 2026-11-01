/**
 * Trim `tauri/target/debug/incremental` to the newest sessions per crate.
 *
 * ## Why the directory grows without bound
 *
 * Cargo writes each build session into a directory named `<crate>-<hash>`, and
 * it never removes an old one. The hash changes when the compiler settings or
 * the crate's dependency graph change, so a tree that alternates between
 * `cargo check`, `cargo test` and `cargo run` accumulates a session per shape.
 * On this machine 42 sessions had built up and the directory reached 12.6 GB --
 * 11.9 GB of it in the seven sessions belonging to the crate that dominates the
 * build.
 *
 * ## Why trimming is safe
 *
 * A session directory is a cache: cargo reuses the newest one to avoid
 * recompiling and rebuilds from scratch when it does not match. Deleting old
 * sessions therefore costs nothing, but deleting the one cargo is about to
 * reuse costs a full recompile. The prune keeps the newest sessions per crate
 * -- ordered by modification time, since the hash suffix carries no ordering.
 *
 * ## Why it runs detached
 *
 * It must not race cargo. `dev:debug` spawns this as a separate process after
 * `tauri dev` has had time to create the session it is going to write, so the
 * session cargo is using is the newest one by mtime and is kept.
 */

import fs from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const KEEP_PER_CRATE = 2;

/** `ai_toolbox_lib-3lcycwp8kvyb6` -> `ai_toolbox_lib`. */
function crateName(sessionDirectoryName) {
  const separator = sessionDirectoryName.lastIndexOf('-');
  return separator === -1 ? sessionDirectoryName : sessionDirectoryName.slice(0, separator);
}

async function walk(directory, visitFile) {
  const pending = [directory];
  while (pending.length > 0) {
    const current = pending.pop();
    let entries;
    try {
      entries = await fs.readdir(current, { withFileTypes: true });
    } catch {
      continue;
    }
    for (const entry of entries) {
      const entryPath = path.join(current, entry.name);
      if (entry.isDirectory()) {
        pending.push(entryPath);
      } else {
        await visitFile(entryPath);
      }
    }
  }
}

/** Newest mtime anywhere inside -- cargo touches files as it writes the session. */
async function newestMtimeMs(directory) {
  let newest = 0;
  await walk(directory, async (filePath) => {
    try {
      const stats = await fs.stat(filePath);
      if (stats.mtimeMs > newest) newest = stats.mtimeMs;
    } catch {
      // Removed by a concurrent cargo run; simply not part of this measurement.
    }
  });
  return newest;
}

async function directorySize(directory) {
  let total = 0;
  await walk(directory, async (filePath) => {
    try {
      total += (await fs.stat(filePath)).size;
    } catch {
      // Same as above.
    }
  });
  return total;
}

async function pruneIncrementalRoot(incrementalRoot) {
  let sessionNames;
  try {
    sessionNames = await fs.readdir(incrementalRoot);
  } catch {
    return null; // Nothing has been built yet.
  }

  const sessionsByCrate = new Map();
  for (const sessionName of sessionNames) {
    const sessionPath = path.join(incrementalRoot, sessionName);
    try {
      if (!(await fs.stat(sessionPath)).isDirectory()) continue;
    } catch {
      continue;
    }
    const crate = crateName(sessionName);
    if (!sessionsByCrate.has(crate)) sessionsByCrate.set(crate, []);
    sessionsByCrate.get(crate).push({
      sessionName,
      sessionPath,
      newestMtimeMs: await newestMtimeMs(sessionPath),
    });
  }

  const removed = [];
  let freedBytes = 0;
  for (const sessions of sessionsByCrate.values()) {
    sessions.sort((left, right) => right.newestMtimeMs - left.newestMtimeMs);
    for (const stale of sessions.slice(KEEP_PER_CRATE)) {
      const size = await directorySize(stale.sessionPath);
      try {
        await fs.rm(stale.sessionPath, { recursive: true, force: true });
        freedBytes += size;
        removed.push(stale.sessionName);
      } catch (error) {
        console.warn(`[prune-incremental] could not remove ${stale.sessionName}: ${error.message}`);
      }
    }
  }

  return { removed, freedBytes, crateCount: sessionsByCrate.size };
}

/**
 * Prune one checkout's incremental cache. Never throws: this runs alongside a
 * dev session and housekeeping must not be able to end it.
 */
export async function pruneCargoIncremental({ tauriRoot }) {
  const incrementalRoot = path.join(tauriRoot, 'target', 'debug', 'incremental');
  let result;
  try {
    result = await pruneIncrementalRoot(incrementalRoot);
  } catch (error) {
    console.warn(`[prune-incremental] skipped: ${error.message}`);
    return;
  }
  if (result === null) return;

  if (result.removed.length === 0) {
    console.log(
      `[prune-incremental] nothing to trim (${result.crateCount} crates, keeping newest ${KEEP_PER_CRATE} each)`,
    );
    return;
  }
  console.log(
    `[prune-incremental] removed ${result.removed.length} stale session(s), ` +
      `freed ${Math.round(result.freedBytes / (1024 * 1024))} MB ` +
      `(${result.crateCount} crates, keeping newest ${KEEP_PER_CRATE} each)`,
  );
}

const invokedDirectly =
  process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url);

if (invokedDirectly) {
  const scriptDirectory = path.dirname(fileURLToPath(import.meta.url));
  await pruneCargoIncremental({ tauriRoot: path.resolve(scriptDirectory, '..', 'tauri') });
}
