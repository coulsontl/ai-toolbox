import type React from 'react';

/**
 * The shape every model dialog in this app uses.
 *
 * ## Why labels sit above the field
 *
 * The dialogs used to lay their labels in a left column (`layout="horizontal"`
 * + `labelCol` / `wrapperCol`). That kept every row's input starting at the same
 * x, which reads well for a form of short, uniform fields — but a model dialog
 * is not that: its fields are heterogeneous (a JSON editor, a chip group of
 * capability switches, two paired number inputs), and a fixed label column
 * squeezes every one of them into the same narrow right-hand strip. The JSON
 * editors ended up the width of a text input, and the capability checkboxes
 * wrapped into a ragged column.
 *
 * Labels above the field give each control the dialog's full width, which is
 * what the editors need and what makes the paired limits sit side by side.
 *
 * ## What this is NOT
 *
 * It is not "vertical is fine now". The rule the guard enforces is
 * **uniformity**: every model/provider form dialog in `features/coding` uses
 * this shape. The original ZCode bug was one dialog using vertical while the
 * other five used horizontal — two products' worth of chrome on one page. A
 * single dialog switching back to horizontal recreates exactly that.
 *
 * Exported from one place so the spans cannot drift between the shared
 * `ModelFormModal` and the module-local ones (codex / grok / kimi / zcode /
 * openclaw / modelsJson).
 */

/**
 * The label/field split applied by `Form` when this returns vertical.
 *
 * Deliberately a named constant rather than an inline `layout="vertical"`:
 * `scripts/verify-form-modal-layout.mjs` scans for the literal, and a shape
 * expressed in one shared file is a shape that cannot be half-migrated.
 */
export const MODEL_FORM_LAYOUT = 'vertical' as const;

/**
 * Gap under each label, matching antd's own vertical default.
 *
 * Stated explicitly because a `labelCol` anywhere in the tree wins over
 * `layout="vertical"`: a caller that passes one gets a hybrid where the label
 * sits above but is still indented by the column. The guard treats a
 * `labelCol` on a governed file as a violation for that reason.
 */
export const MODEL_FORM_ITEM_STYLE: React.CSSProperties = {
  marginBottom: 20,
};

/**
 * Width every model dialog shares.
 *
 * Wider than the old 640: the fields now span the dialog, and a JSON editor at
 * 640px is a scroll box rather than a document. 720 fits the longest line of
 * the largest placeholder (the variants map) without wrapping.
 */
export const MODEL_FORM_MODAL_WIDTH = 720;

/**
 * The two-column grid for paired limits (context / output).
 *
 * They are one fact split in two and belong on one row; stacking them made the
 * dialog taller than the viewport while each input still held six digits.
 */
export const MODEL_FORM_PAIR_STYLE: React.CSSProperties = {
  display: 'grid',
  gridTemplateColumns: 'repeat(2, minmax(0, 1fr))',
  gap: 16,
};

/**
 * The capability switch group: a wrapping row, one chip per capability.
 *
 * `gap: 8px 20px` — the horizontal gap is wider than the vertical so the
 * labels read as separate items rather than as one sentence.
 */
export const MODEL_FORM_CHIP_ROW_STYLE: React.CSSProperties = {
  display: 'flex',
  flexWrap: 'wrap',
  gap: '8px 20px',
};
