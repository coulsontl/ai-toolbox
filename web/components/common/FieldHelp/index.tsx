import React from 'react';
import { Tooltip, Typography } from 'antd';
import { QuestionCircleOutlined } from '@ant-design/icons';
import { parseHelpInline, parseHelpText } from './parseHelpText';

const { Text } = Typography;

/** Renders one line, turning `**bold**` and `` `code` `` into their elements. */
const renderInline = (text: string): React.ReactNode[] =>
  parseHelpInline(text).map((run, index) => {
    if (run.kind === 'bold') {
      return <strong key={index}>{run.text}</strong>;
    }
    if (run.kind === 'code') {
      return (
        <Text key={index} code>
          {run.text}
        </Text>
      );
    }
    return run.text;
  });

/**
 * Renders the help copy the way ZCode's own dialog does: blank-line-separated
 * paragraphs, `- ` lines as bullets, `**bold**` and `` `code` `` inline.
 *
 * Without this the copy would render as one run-on line — the markers are
 * literal characters in the string, and a tooltip does not interpret them.
 */
const HelpContent: React.FC<{ text: string }> = ({ text }) => (
  <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
    {parseHelpText(text).map((block, index) =>
      block.kind === 'list' ? (
        <ul key={index} style={{ margin: 0, paddingLeft: 16 }}>
          {block.items.map((item, itemIndex) => (
            <li key={itemIndex}>{renderInline(item)}</li>
          ))}
        </ul>
      ) : (
        <p key={index} style={{ margin: 0, whiteSpace: 'pre-line' }}>
          {renderInline(block.text)}
        </p>
      ),
    )}
  </div>
);

export interface FieldHelpProps {
  /**
   * The explanation. Plain text with `\n\n` paragraphs, `- ` bullets and
   * `**bold**` / `` `code` `` inline markers.
   */
  text: string;
  /** Accessible name; defaults to the help text itself. */
  ariaLabel?: string;
}

/**
 * A question mark that explains the field it sits beside, on hover.
 *
 * Use it when the label alone cannot carry the meaning — "推理参数映射",
 * "模型能力", "输入类型" all need a sentence about allowed values and caveats,
 * while "上下文窗口" does not.
 *
 * Two placement rules:
 *
 * - Render it **beside** the label, never inside it: a click on the label must
 *   still focus the control.
 * - Keep it out of the `<label>` element so screen readers read the field name,
 *   not the field name plus the whole explanation. The icon carries its own
 *   `aria-label`, so the copy stays reachable.
 */
const FieldHelp: React.FC<FieldHelpProps> = ({ text, ariaLabel }) => (
  <Tooltip
    title={<HelpContent text={text} />}
    // The default tooltip is a single-line overlay; a paragraph with bullets
    // needs a ceiling on width or it stretches across the window. Set on
    // `container` (the padded box), not `root` (the positioning wrapper), so
    // the padding counts against the 360px rather than sitting outside it.
    styles={{ container: { maxWidth: 360 } }}
  >
    <QuestionCircleOutlined
      role="img"
      aria-label={ariaLabel ?? text}
      style={{
        marginLeft: 4,
        fontSize: 12,
        color: 'var(--color-text-tertiary)',
        cursor: 'help',
        verticalAlign: 'middle',
      }}
    />
  </Tooltip>
);

export default FieldHelp;

/**
 * A field label with its help marker.
 *
 * For plain-string labels (`Form.Item label={...}`); a caller that needs richer
 * label content should compose `<FieldHelp>` itself.
 *
 * **The colon is part of the text here, not drawn by the form's `.label::after`.**
 * That pseudo-element hangs off the `<label>` node, so it lands after every
 * child — i.e. after the help icon, reading 「上下文窗口 ⓘ:」 instead of
 * 「上下文窗口: ⓘ」. The shared stylesheet suppresses its colon for any label
 * that contains this icon (`:has(.anticon-question-circle)`), which is why the
 * marker element is not optional.
 */
export const labelWithHelp = (label: string, help: string): React.ReactNode => (
  <span>
    {label}:
    <FieldHelp text={help} />
  </span>
);
