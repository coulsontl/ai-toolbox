/**
 * The context / output lengths people actually type.
 *
 * **One list for every model dialog.** Each of them used to carry its own copy
 * — the shared `ModelFormModal`, OpenClaw, modelsJson, Kimi — which was the
 * same numbers retyped and free to drift. All four now import from here.
 *
 * The values are the ones the catalogues use: powers of two up to 64K, then the
 * round decimal figures real providers publish (128000, 200000, 1000000).
 *
 * Which list a field takes follows the **magnitude of that field**, not its
 * position in a dialog: a context window and an input limit are both prompt-
 * side and use `CONTEXT_LIMIT_PRESETS`; only the completion side is two orders
 * of magnitude smaller and uses `OUTPUT_LIMIT_PRESETS`.
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
