#!/usr/bin/env node
/**
 * Computes the next rolling beta version for the release workflow.
 *
 * Layout: `<stable+1 patch>-beta.<YYMMDD><letter>` (Asia/Shanghai clock).
 *
 * - The stable base is always the latest non-prerelease release plus one
 *   patch step, so a beta stays ahead of the shipped stable and a stable
 *   with the same number can later supersede it (semver: `1.2.1` >
 *   `1.2.1-beta.anything`).
 * - Patch carries: `1.2.9 -> 1.3.0`, `1.9.9 -> 2.0.0`.
 * - The letter restarts at `a` each day and increments per same-day build.
 *   The stamp is a lexicographic-only identifier on purpose: `YYMMDD` keeps
 *   dictionary order equal to chronological order across months and years,
 *   so plain semver comparison (ours and the updater plugin's) stays correct
 *   with no custom comparator.
 *
 * CLI (used by release.yml): queries GitHub for the latest stable release
 * and the rolling `beta` release's current `latest.json`, then prints the
 * decision as one JSON line on stdout.
 */

import { execFileSync } from 'node:child_process';

export const BETA_TAG = 'beta';

/** `1.2.0 -> 1.2.1`, `1.2.9 -> 1.3.0`, `1.9.9 -> 2.0.0`. */
export function bumpPatch(version) {
  const match = /^(\d+)\.(\d+)\.(\d+)$/.exec(version);
  if (!match) {
    throw new Error(`Not a plain semver version: ${version}`);
  }
  let major = Number(match[1]);
  let minor = Number(match[2]);
  let patch = Number(match[3]) + 1;
  if (patch > 9) {
    patch = 0;
    minor += 1;
  }
  if (minor > 9) {
    minor = 0;
    major += 1;
  }
  return `${major}.${minor}.${patch}`;
}

/** `YYMMDD` in Asia/Shanghai, the clock the stamp is meant to read in. */
export function shanghaiDayStamp(now = new Date()) {
  const parts = new Intl.DateTimeFormat('en-CA', {
    timeZone: 'Asia/Shanghai',
    year: '2-digit',
    month: '2-digit',
    day: '2-digit',
  }).formatToParts(now);
  const get = (type) => parts.find((part) => part.type === type).value;
  return `${get('year')}${get('month')}${get('day')}`;
}

/** Extracts the `<YYMMDD><letter>` suffix from a beta version string. */
export function parseBetaStamp(version) {
  const match = /-beta\.(\d{6})([a-z]+)$/.exec(version);
  if (!match) {
    return null;
  }
  return { day: match[1], letters: match[2] };
}

/** `a -> b`, `z -> za`; the longer form still compares greater. */
export function bumpLetters(letters) {
  const last = letters[letters.length - 1];
  if (last === 'z') {
    return `${letters}a`;
  }
  return `${letters.slice(0, -1)}${String.fromCharCode(last.charCodeAt(0) + 1)}`;
}

/**
 * The next rolling beta version.
 *
 * `currentBetaVersion` is what the rolling `beta` release publishes right
 * now (`latest.json`'s `version`), or `null` when there is none yet.
 */
export function nextBetaVersion({ stableVersion, currentBetaVersion, now = new Date() }) {
  const base = bumpPatch(stableVersion);
  const day = shanghaiDayStamp(now);
  const stamp = currentBetaVersion ? parseBetaStamp(currentBetaVersion) : null;
  // The letter only rolls forward when today's builds continue the same
  // stable base; a new base (a stable shipped in between) restarts at `a`.
  const letters =
    stamp && stamp.day === day && currentBetaVersion.startsWith(`${base}-beta.`)
      ? bumpLetters(stamp.letters)
      : 'a';
  return `${base}-beta.${day}${letters}`;
}

function ghOutput(args) {
  return execFileSync('gh', args, { encoding: 'utf8', maxBuffer: 8 * 1024 * 1024 }).trim();
}

function fetchStableVersion() {
  try {
    const tag = JSON.parse(ghOutput(['api', 'repos/{owner}/{repo}/releases/latest'])).tag_name;
    const version = tag.replace(/^v/, '');
    if (!/^\d+\.\d+\.\d+$/.test(version)) {
      throw new Error(`latest stable release tag is not plain semver: ${tag}`);
    }
    return version;
  } catch (error) {
    throw new Error(`Failed to read the latest stable release: ${error.message}`);
  }
}

function fetchCurrentBetaVersion() {
  // The rolling beta release is a draft for the whole build, and
  // `releases/tags/<tag>` answers 404 for drafts — reading it through the
  // list endpoint instead keeps the same-day letter counting even when a
  // re-dispatch happens mid-build. Losing this would reset the letter to
  // `a`, colliding with an already-published version.
  //
  // A list request that fails is NOT the same as "no beta release yet":
  // treating them alike would silently fall back to letter `a` and collide.
  // Only an absent release or an absent `latest.json` means "nothing built".
  const releases = JSON.parse(
    ghOutput(['api', 'repos/{owner}/{repo}/releases?per_page=20'])
  );
  const release = releases.find((candidate) => candidate.tag_name === BETA_TAG) || null;
  if (!release) {
    return null; // no rolling beta release yet
  }
  const asset = (release.assets || []).find((candidate) => candidate.name === 'latest.json');
  if (!asset) {
    return null; // release exists but has never published a build
  }
  const latestJson = ghOutput([
    'api',
    '-H',
    'Accept: application/octet-stream',
    `repos/{owner}/{repo}/releases/assets/${asset.id}`,
  ]);
  const version = JSON.parse(latestJson).version;
  if (typeof version !== 'string') {
    throw new Error('Latest beta manifest carries no string `version` field');
  }
  return version;
}

function main() {
  const stableVersion = fetchStableVersion();
  const currentBetaVersion = fetchCurrentBetaVersion();
  const next = nextBetaVersion({ stableVersion, currentBetaVersion });
  process.stdout.write(
    `${JSON.stringify({ stableVersion, currentBetaVersion, nextVersion: next })}\n`
  );
}

if (process.argv[1] && process.argv[2] === '--compute') {
  main();
}
