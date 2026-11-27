export function writeTauriVersion(
  configPath: string,
  version: string
): Promise<{ previous: unknown; version: string }>;

export function writeCrateVersion(
  cargoTomlPath: string,
  cargoLockPath: string,
  packageName: string,
  version: string
): Promise<{
  previous: string;
  cargoTomlChanged: boolean;
  cargoLockChanged: boolean;
}>;
