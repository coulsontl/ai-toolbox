import { parseGitRepo } from './gitUrl';

export type SkillAliasValidationError =
  | 'empty'
  | 'tooLong'
  | 'startsWithDot'
  | 'endsWithDot'
  | 'invalidChars'
  | 'exists';

const MAX_ALIAS_LENGTH = 128;
const INVALID_ALIAS_CHAR_PATTERN = /[/\\<>:"|?*\u0000-\u001f\u007f]/;
const INVALID_REPO_NAME_CHAR_PATTERN = /[/\\<>:"|?*\u0000-\u001f\u007f]/g;

function sanitizeNameSegment(raw: string): string | null {
  const sanitized = raw
    .replace(INVALID_REPO_NAME_CHAR_PATTERN, '-')
    .replace(/^[.-]+|[.-]+$/g, '')
    .trim();
  return sanitized || null;
}

function boundAliasBase(base: string, suffixLength: number): string {
  const maxBaseLength = Math.max(1, MAX_ALIAS_LENGTH - suffixLength);
  if (base.length <= maxBaseLength) {
    return base;
  }
  return base.slice(0, maxBaseLength).replace(/[.-]+$/, '') || 'skill';
}

/**
 * Validate a custom Skill alias against backend naming rules and existing names.
 */
export function validateSkillAlias(
  raw: string,
  existingNamesLower: ReadonlySet<string>,
): SkillAliasValidationError | null {
  const trimmed = raw.trim();
  if (!trimmed) {
    return 'empty';
  }
  if (trimmed.length > MAX_ALIAS_LENGTH) {
    return 'tooLong';
  }
  if (trimmed === '.' || trimmed === '..' || trimmed.startsWith('.')) {
    return 'startsWithDot';
  }
  if (trimmed.endsWith('.')) {
    return 'endsWithDot';
  }
  if (INVALID_ALIAS_CHAR_PATTERN.test(trimmed)) {
    return 'invalidChars';
  }
  if (existingNamesLower.has(trimmed.toLowerCase())) {
    return 'exists';
  }
  return null;
}

/**
 * Extract a sanitized repository name from a Git URL or owner/repo shorthand.
 */
export function extractRepoNameFromGitUrl(url: string): string | null {
  const trimmed = url.trim();
  if (!trimmed) {
    return null;
  }

  const parsed = parseGitRepo(trimmed);
  if (parsed) {
    return sanitizeNameSegment(parsed.repo);
  }

  const withoutQuery = trimmed.split(/[?#]/)[0].trim();
  if (!withoutQuery) {
    return null;
  }

  let pathPart = withoutQuery;
  if (/^[a-zA-Z][a-zA-Z0-9+.-]*:\/\//.test(withoutQuery)) {
    try {
      pathPart = new URL(withoutQuery).pathname;
    } catch {
      return null;
    }
  } else if (/^(?:[^@/:]+@)?[^/:]{2,}:(.+)$/.test(withoutQuery)) {
    const scpMatch = withoutQuery.match(/^(?:[^@/:]+@)?[^/:]{2,}:(.+)$/);
    pathPart = scpMatch ? scpMatch[1] : withoutQuery;
  }

  const strippedSubpath = pathPart.replace(/\/(?:-\/)?(?:tree|blob)\/[\s\S]*$/, '');
  const segments = strippedSubpath
    .split('/')
    .map((segment) => segment.trim())
    .filter(Boolean);

  if (segments.length === 0) {
    return null;
  }

  const gitSuffixIndex = segments.findIndex((segment) => segment.toLowerCase().endsWith('.git'));
  const targetSegment = gitSuffixIndex >= 0 ? segments[gitSuffixIndex] : segments[segments.length - 1];
  const rawRepo = targetSegment.replace(/\.git$/i, '');
  if (!rawRepo) {
    return null;
  }

  return sanitizeNameSegment(rawRepo);
}

/**
 * Suggest a non-conflicting Skill alias, optionally incorporating a repo hint.
 */
export function suggestSkillAlias(
  baseName: string,
  existingNamesLower: ReadonlySet<string>,
  repoHint?: string | null,
): string {
  const cleanedBase = sanitizeNameSegment(baseName) || 'skill';
  const cleanedHint = repoHint ? sanitizeNameSegment(repoHint) : null;

  if (cleanedHint) {
    const hintedBase = `${boundAliasBase(cleanedBase, cleanedHint.length + 1)}-${cleanedHint}`;
    const hintedError = validateSkillAlias(hintedBase, existingNamesLower);
    if (hintedError === null) {
      return hintedBase;
    }
    if (hintedError === 'exists') {
      for (let index = 2; index < 10_000; index += 1) {
        const suffix = `-${index}`;
        const candidate = `${boundAliasBase(hintedBase, suffix.length)}${suffix}`;
        const err = validateSkillAlias(candidate, existingNamesLower);
        if (err === null) {
          return candidate;
        }
        if (err !== 'exists') {
          break;
        }
      }
    }
  }

  for (let index = 2; index < 10_000; index += 1) {
    const suffix = `-${index}`;
    const candidate = `${boundAliasBase(cleanedBase, suffix.length)}${suffix}`;
    if (validateSkillAlias(candidate, existingNamesLower) === null) {
      return candidate;
    }
  }

  return `${boundAliasBase(cleanedBase, 14)}-${Date.now()}`;
}
