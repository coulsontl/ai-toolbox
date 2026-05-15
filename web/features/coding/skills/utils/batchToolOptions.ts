export interface BatchToolOptions {
  quiet?: boolean;
  overwriteExisting?: boolean;
}

export const GROUP_TOOL_BATCH_OPTIONS: BatchToolOptions = {
  quiet: true,
  overwriteExisting: true,
};

export function shouldOverwriteExistingTarget(options?: BatchToolOptions): boolean {
  return options?.overwriteExisting === true;
}
