/**
 * The context / output lengths people actually type.
 *
 * Every model dialog had its own copy of this list (the shared
 * `ModelFormModal`, OpenClaw's, and — after this change — modelsJson's), which
 * is the same numbers retyped three times and free to drift. The values are
 * the ones the catalogues use: powers of two up to 64K, then the round decimal
 * figures real providers publish (128000, 200000, 1000000).
 *
 * `value` is a string because it is fed to `AutoComplete`/`InputNumber`, whose
 * input is text until it parses; `label` is what a reader recognises faster
 * than the digit count.
 */
export interface LimitPresetOption {
  value: string;
  label: string;
}

/** Longest prompt a model accepts. */
export const CONTEXT_LIMIT_PRESETS: LimitPresetOption[] = [
  { value: '4096', label: '4K' },
  { value: '8192', label: '8K' },
  { value: '16384', label: '16K' },
  { value: '32768', label: '32K' },
  { value: '65536', label: '64K' },
  { value: '128000', label: '128K' },
  { value: '200000', label: '200K' },
  { value: '256000', label: '256K' },
  { value: '1000000', label: '1M' },
  { value: '2000000', label: '2M' },
];

/** Longest answer a model can produce. Smaller by two orders of magnitude. */
export const OUTPUT_LIMIT_PRESETS: LimitPresetOption[] = [
  { value: '2048', label: '2K' },
  { value: '4096', label: '4K' },
  { value: '8192', label: '8K' },
  { value: '16384', label: '16K' },
  { value: '32768', label: '32K' },
  { value: '65536', label: '64K' },
];
