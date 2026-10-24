export const BETA_TAG: string;
export function bumpPatch(version: string): string;
export function shanghaiDayStamp(now?: Date): string;
export function parseBetaStamp(version: string): { day: string; letters: string } | null;
export function bumpLetters(letters: string): string;
export function nextBetaVersion(options: {
  stableVersion: string;
  currentBetaVersion: string | null;
  now?: Date;
}): string;
