import type { SessionMeta } from './types';

export function reconcileSessionSelection(
  selectedSourcePaths: string[],
  sessions: SessionMeta[],
): string[] {
  const loadedSourcePaths = new Set(sessions.map((session) => session.sourcePath));
  const nextSelection = selectedSourcePaths.filter((sourcePath) => loadedSourcePaths.has(sourcePath));
  return nextSelection.length === selectedSourcePaths.length ? selectedSourcePaths : nextSelection;
}

export function toggleLoadedSessionSelection(
  selectedSourcePaths: string[],
  sessions: SessionMeta[],
): string[] {
  const loadedSourcePaths = new Set(sessions.map((session) => session.sourcePath));
  const selectedPathSet = new Set(selectedSourcePaths);
  const allLoadedSelected = loadedSourcePaths.size > 0
    && [...loadedSourcePaths].every((sourcePath) => selectedPathSet.has(sourcePath));

  if (allLoadedSelected) {
    return selectedSourcePaths.filter((sourcePath) => !loadedSourcePaths.has(sourcePath));
  }

  loadedSourcePaths.forEach((sourcePath) => selectedPathSet.add(sourcePath));
  return Array.from(selectedPathSet);
}
