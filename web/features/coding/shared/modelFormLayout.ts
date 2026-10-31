import type React from 'react';

/**
 * The class `Form`'s semantic `classNames.label` should receive.
 *
 * A CSS-Modules class name is resolved per import site, so the `.less` file is
 * imported by each dialog that needs it and the name comes from
 * `styles.label` there. This module re-exports nothing of the sort: exporting
 * `styles.label` from here would read as a convenience but breaks the
 * `verify-css-module-classes` scan, which resolves `styles.x` inside the file
 * that imports the stylesheet.
 */

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
 * The label's *inline* style — colour only.
 *
 * The colon is drawn by `.label::after` in the `.less` module, because a
 * pseudo-element cannot be expressed as an inline style. This object stays for
 * the cases where a caller sets a style on one specific label (a group caption)
 * and must keep the same colour without dragging in the class.
 */
export const MODEL_FORM_LABEL_STYLE: React.CSSProperties = {
  fontWeight: 400,
  color: 'var(--color-text-primary)',
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
 * The grid for two fields that are one fact split in two.
 *
 * Which pairs qualify: context/output limits, input/output modalities, a
 * reasoning-level list and its default. They belong on one row because reading
 * them apart is what costs the dialog its height — each on its own row is a
 * label line plus a control line, and a dialog of eight short selects outgrows
 * the viewport while every box still holds six digits or three words.
 *
 * What does *not* qualify: fields that merely sit next to each other (a name
 * and an id), or controls of different heights (a select beside a JSON
 * editor). Those keep their own rows.
 *
 * User-directed twice on 2026-11-01: 「最关键的这些怎么还是在竖着排啊，太占空间了」
 * for the limits, then 「也和上下文一样，改成左右排列，它们是一组的」 for the
 * modalities.
 */
export const MODEL_FORM_PAIR_STYLE: React.CSSProperties = {
  display: 'grid',
  gridTemplateColumns: 'repeat(2, minmax(0, 1fr))',
  gap: 16,
};

/** Three columns, for limits plus temperature. */
export const MODEL_FORM_TRIPLE_STYLE: React.CSSProperties = {
  display: 'grid',
  gridTemplateColumns: 'repeat(3, minmax(0, 1fr))',
  gap: 16,
};



/**
 * A label for a group of switches drawn as one inline row.
 *
 * Booleans are the densest fields a model has — five of them, each a single
 * bit — and giving each its own label line plus its own control line spent ten
 * rows on five bits. One caption above one wrapping checkbox row says the same
 * thing in two.
 *
 * **No `marginBottom` here.** This span is rendered *inside* the `<label>` that
 * antd owns, and a block child with a bottom margin makes that label 30px tall
 * instead of 22px — the label's line box then extends past the text, and the
 * `::after` colon (which belongs to the same line box) drops to sit below
 * 「能力开关」 instead of after it (user-reported, 2026-11-01). The gap to the
 * checkbox row is antd's own vertical label padding, which is already correct.
 */
export const MODEL_FORM_GROUP_LABEL_STYLE: React.CSSProperties = {
  ...MODEL_FORM_LABEL_STYLE,
  display: 'block',
};

/**
 * The capability switch group: a wrapping row, one chip per capability.
 *
 * `gap: 8px 20px` — the horizontal gap is wider than the vertical so the
 * labels read as separate items rather than as one sentence.
 *
 * Each checkbox inside also takes `MODEL_FORM_CHECKBOX_STYLES`: a size on this
 * container would never reach the captions, because antd writes its own
 * `font-size` onto every `.ant-checkbox-wrapper`.
 */
export const MODEL_FORM_CHIP_ROW_STYLE: React.CSSProperties = {
  display: 'flex',
  flexWrap: 'wrap',
  gap: '8px 20px',
};

/**
 * The hint line under a field (`Form.Item`'s `extra`), one size below a label.
 *
 * Passed through `Form`'s semantic `styles.extra` so every hint in a dialog is
 * 12px without each field wrapping its own `<Text>`. antd already colours
 * `-extra` secondary; the size was the problem — it matched the label exactly,
 * so a hint under 「端点」 read as a second field name (user-directed,
 * 2026-11-01: 「下面内容的这些字体比标题小一点」).
 */
export const MODEL_FORM_EXTRA_STYLE: React.CSSProperties = {
  fontSize: 12,
};

/**
 * A checkbox caption: one size below, and quieter than, the group caption.
 *
 * `<Checkbox styles={MODEL_FORM_CHECKBOX_STYLES}>` — antd puts the semantic
 * style on the `<span>` that holds the caption, which is the node carrying both
 * the text and antd's own `font-size` / `color`. Same inline-vs-class reasoning
 * as `MODEL_FORM_EXTRA_STYLE`: at 14px and the primary colour, 「支持工具调用」
 * was indistinguishable from the 「能力开关」 caption above it
 * (user-directed, 2026-11-01: 「下面内容的这些字体比标题小一点」, then
 * 「内容灰色也没做啊」).
 *
 * `color` is what makes them read as *values* of the group rather than as
 * further headings. The box itself stays as antd draws it; only the caption
 * text is muted, so a checked switch is still obvious.
 */
export const MODEL_FORM_CHECKBOX_STYLES = {
  label: { fontSize: 13, color: 'var(--color-text-secondary)' },
} satisfies Record<'label', React.CSSProperties>;
