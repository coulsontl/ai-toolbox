interface SkillOpenPathInput {
  source_type: string;
  source_ref?: string | null;
  central_path?: string | null;
}

const SKILL_MANIFEST_FILE = 'SKILL.md';

export function normalizeSkillPath(path: string | null | undefined): string {
  return (path ?? '').trim();
}

export function joinSkillPath(basePath: string | null | undefined, childPath: string): string | null {
  const normalizedBase = normalizeSkillPath(basePath).replace(/[\\/]+$/, '');
  const normalizedChild = normalizeSkillPath(childPath).replace(/^[\\/]+/, '');

  if (!normalizedBase || !normalizedChild) {
    return null;
  }

  const separator = normalizedBase.includes('\\') && !normalizedBase.includes('/') ? '\\' : '/';
  return `${normalizedBase}${separator}${normalizedChild}`;
}

export function getSkillManifestPath(centralPath: string | null | undefined): string | null {
  return joinSkillPath(centralPath, SKILL_MANIFEST_FILE);
}

function pushUniquePath(paths: string[], path: string | null | undefined): void {
  const normalizedPath = normalizeSkillPath(path);
  if (normalizedPath && !paths.includes(normalizedPath)) {
    paths.push(normalizedPath);
  }
}

export function getSkillFolderOpenCandidates(skill: SkillOpenPathInput): string[] {
  const paths: string[] = [];

  if (skill.source_type.toLowerCase() === 'local') {
    pushUniquePath(paths, skill.source_ref);
  }

  pushUniquePath(paths, skill.central_path);

  return paths;
}
