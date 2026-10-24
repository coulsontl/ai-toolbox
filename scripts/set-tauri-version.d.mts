export function writeTauriVersion(
  configPath: string,
  version: string
): Promise<{ previous: unknown; version: string }>;
