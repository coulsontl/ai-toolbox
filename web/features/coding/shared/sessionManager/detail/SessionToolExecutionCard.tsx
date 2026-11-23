import React from 'react';
import { Copy } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import type { SessionMessageBlock } from '../types';
import { getToolDisplayName, getToolIcon, getToolVariant } from './domain/toolCatalog';
import { normalizeToolBlock } from './domain/toolPairing';
import SessionRendererCard from './SessionRendererCard';
import SessionSearchHighlight from './SessionSearchHighlight';
import SessionToolResultBlock from './SessionToolResultBlock';
import styles from './SessionDetailWorkbench.module.less';

interface SessionToolExecutionCardProps {
  block: SessionMessageBlock;
  query: string;
  onCopyText: (text: string, successText: string) => void | Promise<void>;
}

const SessionToolExecutionCard: React.FC<SessionToolExecutionCardProps> = ({ block, query, onCopyText }) => {
  const { t } = useTranslation();
  const normalizedBlock = normalizeToolBlock(block);
  const normalizedToolName = normalizedBlock.normalizedToolName || 'unknown';
  const variant = getToolVariant(normalizedBlock.toolName, normalizedToolName);
  const Icon = getToolIcon(normalizedToolName, normalizedBlock.toolName);
  const title = getToolDisplayName(normalizedToolName, normalizedBlock.toolName);
  const toolId = normalizedBlock.toolId;

  const handleCopyToolId = (event: React.MouseEvent<HTMLElement> | React.KeyboardEvent<HTMLElement>) => {
    if (!toolId) {
      return;
    }
    event.preventDefault();
    event.stopPropagation();
    void onCopyText(toolId, t('sessionManager.copyToolIdSuccess'));
  };

  const handleToolIdKeyDown = (event: React.KeyboardEvent<HTMLElement>) => {
    if (event.key === 'Enter' || event.key === ' ') {
      handleCopyToolId(event);
    }
  };

  return (
    <SessionRendererCard
      icon={Icon}
      title={title}
      variant={variant}
      status={normalizedBlock.status}
      meta={toolId ? (
        <code
          className={styles.toolIdBadge}
          role="button"
          tabIndex={0}
          title={t('sessionManager.copyToolId')}
          aria-label={t('sessionManager.copyToolId')}
          onClick={handleCopyToolId}
          onKeyDown={handleToolIdKeyDown}
        >
          {toolId}
        </code>
      ) : null}
    >
      <ToolPayloadSection
        label="Input"
        copyText={toolPayloadToCopyText(normalizedBlock.input, true)}
        copyLabel={t('sessionManager.copyToolInput')}
        copySuccessText={t('sessionManager.copyToolInputSuccess')}
        onCopyText={onCopyText}
      >
        <ToolBody
          normalizedToolName={normalizedToolName}
          block={normalizedBlock}
          query={query}
          onCopyText={onCopyText}
        />
      </ToolPayloadSection>
      {normalizedBlock.kind === 'tool_execution' || normalizedBlock.kind === 'tool_result' ? (
        <ToolPayloadSection
          label="Result"
          copyText={toolPayloadToCopyText(normalizedBlock.output, false)}
          copyLabel={t('sessionManager.copyToolResult')}
          copySuccessText={t('sessionManager.copyToolResultSuccess')}
          onCopyText={onCopyText}
          separated
        >
          <SessionToolResultBlock output={normalizedBlock.output} query={query} status={normalizedBlock.status} />
        </ToolPayloadSection>
      ) : null}
    </SessionRendererCard>
  );
};

interface ToolPayloadSectionProps {
  label: string;
  copyText: string;
  copyLabel: string;
  copySuccessText: string;
  separated?: boolean;
  children: React.ReactNode;
  onCopyText: (text: string, successText: string) => void | Promise<void>;
}

const ToolPayloadSection: React.FC<ToolPayloadSectionProps> = ({
  label,
  copyText,
  copyLabel,
  copySuccessText,
  separated,
  children,
  onCopyText,
}) => {
  const canCopy = copyText.trim().length > 0;

  return (
    <div className={separated ? `${styles.toolPayloadSection} ${styles.toolPayloadSectionSeparated}` : styles.toolPayloadSection}>
      <div className={styles.toolPayloadHeader}>
        <span className={styles.sectionLabel}>{label}</span>
        {canCopy ? (
          <button
            type="button"
            className={styles.toolPayloadCopyButton}
            title={copyLabel}
            aria-label={copyLabel}
            onClick={() => void onCopyText(copyText, copySuccessText)}
          >
            <Copy size={12} aria-hidden="true" />
          </button>
        ) : null}
      </div>
      {children}
    </div>
  );
};

interface ToolBodyProps {
  normalizedToolName: string;
  block: SessionMessageBlock;
  query: string;
  onCopyText: (text: string, successText: string) => void | Promise<void>;
}

const ToolBody: React.FC<ToolBodyProps> = ({ normalizedToolName, block, query, onCopyText }) => {
  const input = toRecord(block.input);
  switch (normalizedToolName) {
    case 'bash':
      return <BashBody input={input} query={query} onCopyText={onCopyText} />;
    case 'read':
      return <ReadBody input={input} query={query} />;
    case 'write':
      return <WriteBody input={input} query={query} />;
    case 'edit':
    case 'multi_edit':
      return <EditBody input={input} query={query} />;
    case 'apply_patch':
      return <PatchBody input={input} query={query} />;
    case 'grep':
      return <GrepBody input={input} />;
    case 'glob':
      return <GlobBody input={input} />;
    case 'web_fetch':
      return <WebFetchBody input={input} query={query} />;
    case 'web_search':
      return <WebSearchBody input={input} />;
    case 'todo_write':
      return <TodoBody input={input} />;
    case 'update_plan':
      return <PlanBody input={input} />;
    case 'task':
    case 'agent':
      return <TaskBody input={input} query={query} />;
    case 'mcp':
      return <McpBody block={block} query={query} />;
    default:
      return <JsonBody value={block.input} query={query} />;
  }
};

const BashBody: React.FC<{
  input: Record<string, unknown>;
  query: string;
  onCopyText: (text: string, successText: string) => void | Promise<void>;
}> = ({ input, query, onCopyText }) => {
  const commandText = stringField(input, ['command', 'cmd', 'commandLine', 'command_line']) || safeJson(input);

  return (
    <div className={styles.toolBodyStack}>
      <OptionalText value={stringField(input, ['description', 'toolSummary', 'summary'])} className={styles.toolDescription} />
      <CodePanel
        title="Command"
        value={commandText}
        query={query}
        tone="terminal"
        copyLabelKey="sessionManager.copyCommand"
        copySuccessKey="sessionManager.copyCommandSuccess"
        onCopyText={onCopyText}
      />
      <MetaChips values={[
        chipValue('timeout', anyField(input, ['timeout', 'timeout_ms', 'waitMsBeforeAsync'])),
        chipValue('background', anyField(input, ['run_in_background', 'runInBackground', 'isDaemon'])),
      ]}
      />
    </div>
  );
};

const ReadBody: React.FC<{ input: Record<string, unknown>; query: string }> = ({ input, query }) => {
  const path = stringField(input, ['file_path', 'filePath', 'path']);
  const chips = [
    chipValue('offset', anyField(input, ['offset'])),
    chipValue('limit', anyField(input, ['limit'])),
    chipValue('start line', anyField(input, ['startLine', 'start_line'])),
    chipValue('end line', anyField(input, ['endLine', 'end_line'])),
    chipValue('byteOffset', anyField(input, ['byteOffset', 'byte_offset'])),
  ].filter(Boolean);

  if (!path && chips.length === 0 && Object.keys(input).length > 0) {
    return <JsonBody value={input} query={query} />;
  }

  return (
    <div className={styles.toolBodyStack}>
      <PathRow value={path} />
      <MetaChips values={chips} />
    </div>
  );
};

const WriteBody: React.FC<{ input: Record<string, unknown>; query: string }> = ({ input, query }) => {
  const path = stringField(input, ['file_path', 'filePath', 'path']);
  const content = stringField(input, ['content']);
  const chips = [
    chipValue('overwrite', anyField(input, ['overwrite'])),
    chipValue('append', anyField(input, ['append'])),
  ].filter(Boolean);

  if (!path && !content && Object.keys(input).length > 0) {
    return <JsonBody value={input} query={query} />;
  }

  return (
    <div className={styles.toolBodyStack}>
      <PathRow value={path} />
      {content ? <CodePanel title="Content" value={content} query={query} /> : null}
      <MetaChips values={chips} />
    </div>
  );
};

const EditBody: React.FC<{ input: Record<string, unknown>; query: string }> = ({ input, query }) => {
  const edits = Array.isArray(input.edits) ? input.edits : [];
  const path = stringField(input, ['file_path', 'filePath', 'path']);
  const before = stringField(input, ['old_string', 'oldString', 'target', 'targetContent', 'target_content']);
  const after = stringField(input, ['new_string', 'newString', 'replacement', 'replacementContent', 'replacement_content']);

  if (!path && edits.length === 0 && !before && !after && Object.keys(input).length > 0) {
    return <JsonBody value={input} query={query} />;
  }

  return (
    <div className={styles.toolBodyStack}>
      <PathRow value={path} />
      {edits.length > 0 ? (
        <div className={styles.editList}>
          {edits.map((edit, index) => (
            <CodePanel key={index} title={`Edit ${index + 1}`} value={safeJson(edit)} query={query} />
          ))}
        </div>
      ) : (
        <div className={styles.diffGrid}>
          <CodePanel title="Before" value={before} query={query} />
          <CodePanel title="After" value={after} query={query} />
        </div>
      )}
      <MetaChips values={[chipValue('replace all', anyField(input, ['replace_all', 'replaceAll']))]} />
    </div>
  );
};

const PatchBody: React.FC<{ input: Record<string, unknown>; query: string }> = ({ input, query }) => (
  <CodePanel title="Patch" value={stringField(input, ['patch']) || safeJson(input)} query={query} />
);

const GrepBody: React.FC<{ input: Record<string, unknown> }> = ({ input }) => (
  <div className={styles.toolBodyStack}>
    <PropertyRow label="Pattern" value={stringField(input, ['pattern'])} strong />
    <PropertyRow label="Path" value={stringField(input, ['path'])} />
    <PropertyRow label="Glob" value={stringField(input, ['glob'])} />
    <PropertyRow label="Type" value={stringField(input, ['type'])} />
    <MetaChips values={[
      chipValue('output', input.output_mode ?? input.outputMode),
      chipValue('limit', input.head_limit ?? input.headLimit),
      chipValue('-i', input.i ?? input.ignore_case),
      chipValue('-n', input.n ?? input.line_number),
      chipValue('multiline', input.multiline),
    ]}
    />
  </div>
);

const GlobBody: React.FC<{ input: Record<string, unknown> }> = ({ input }) => (
  <div className={styles.toolBodyStack}>
    <PropertyRow label="Pattern" value={stringField(input, ['pattern'])} strong />
    <PropertyRow label="Path" value={stringField(input, ['path'])} />
  </div>
);

const WebFetchBody: React.FC<{ input: Record<string, unknown>; query: string }> = ({ input, query }) => (
  <div className={styles.toolBodyStack}>
    <PropertyRow label="URL" value={stringField(input, ['url'])} strong />
    {stringField(input, ['prompt']) ? <CodePanel title="Prompt" value={stringField(input, ['prompt'])} query={query} /> : null}
  </div>
);

const WebSearchBody: React.FC<{ input: Record<string, unknown> }> = ({ input }) => (
  <div className={styles.toolBodyStack}>
    <PropertyRow label="Query" value={stringField(input, ['query'])} strong />
    <MetaChips values={[
      chipValue('allowed', input.allowed_domains ?? input.allowedDomains),
      chipValue('blocked', input.blocked_domains ?? input.blockedDomains),
    ]}
    />
  </div>
);

const TodoBody: React.FC<{ input: Record<string, unknown> }> = ({ input }) => {
  const todos = Array.isArray(input.todos) ? input.todos : [];
  if (todos.length === 0) {
    return <JsonBody value={input} query="" />;
  }

  return (
    <div className={styles.todoList}>
      {todos.map((todo, index) => {
        const item = toRecord(todo);
        return (
          <div key={index} className={styles.todoItem}>
            <span className={styles.todoStatus}>{String(item.status ?? 'pending')}</span>
            <span className={styles.todoText}>{String(item.content ?? item.text ?? '')}</span>
            {item.priority ? <span className={styles.todoPriority}>{String(item.priority)}</span> : null}
          </div>
        );
      })}
    </div>
  );
};

const PlanBody: React.FC<{ input: Record<string, unknown> }> = ({ input }) => {
  const plan = Array.isArray(input.plan) ? input.plan : [];
  return (
    <div className={styles.toolBodyStack}>
      <OptionalText value={stringField(input, ['explanation'])} className={styles.toolDescription} />
      {plan.length > 0 ? (
        <div className={styles.todoList}>
          {plan.map((step, index) => {
            const item = toRecord(step);
            return (
              <div key={index} className={styles.todoItem}>
                <span className={styles.todoStatus}>{String(item.status ?? 'pending')}</span>
                <span className={styles.todoText}>{String(item.step ?? '')}</span>
              </div>
            );
          })}
        </div>
      ) : null}
    </div>
  );
};

const TaskBody: React.FC<{ input: Record<string, unknown>; query: string }> = ({ input, query }) => (
  <div className={styles.toolBodyStack}>
    <OptionalText value={stringField(input, ['description', 'role'])} className={styles.toolDescription} />
    <MetaChips values={[
      chipValue('subagent', anyField(input, ['subagent_type', 'subagentType', 'typeName'])),
      chipValue('model', anyField(input, ['model'])),
      chipValue('background', anyField(input, ['run_in_background', 'runInBackground'])),
    ]}
    />
    {stringField(input, ['prompt']) ? <CodePanel title="Prompt" value={stringField(input, ['prompt'])} query={query} /> : null}
  </div>
);

const McpBody: React.FC<{ block: SessionMessageBlock; query: string }> = ({ block, query }) => (
  <div className={styles.toolBodyStack}>
    <PropertyRow label="Tool" value={block.toolName || block.normalizedToolName || 'MCP Tool'} strong />
    <JsonBody value={block.input} query={query} />
  </div>
);

const JsonBody: React.FC<{ value: unknown; query: string }> = ({ value, query }) => (
  <CodePanel value={safeJson(value ?? {})} query={query} />
);

const CodePanel: React.FC<{
  title?: string;
  value: string;
  query: string;
  tone?: 'default' | 'terminal';
  copyLabelKey?: string;
  copySuccessKey?: string;
  onCopyText?: (text: string, successText: string) => void | Promise<void>;
}> = ({
  title,
  value,
  query,
  tone = 'default',
  copyLabelKey,
  copySuccessKey,
  onCopyText,
}) => {
  const { t } = useTranslation();
  let copyButton: React.ReactNode = null;
  if (copyLabelKey && copySuccessKey && onCopyText && value.trim()) {
    const copyLabel = t(copyLabelKey);
    const copySuccessText = t(copySuccessKey);
    const handleCopyText = onCopyText;
    copyButton = (
      <button
        type="button"
        className={styles.toolPayloadCopyButton}
        title={copyLabel}
        aria-label={copyLabel}
        onClick={() => void handleCopyText(value, copySuccessText)}
      >
        <Copy size={12} aria-hidden="true" />
      </button>
    );
  }

  return (
    <div className={styles.preBlockShell}>
      {title ? (
        <div className={styles.preBlockTitle}>
          <span>{title}</span>
          {copyButton}
        </div>
      ) : null}
      <pre className={`${styles.preBlock}${tone === 'terminal' ? ` ${styles.terminalPreBlock}` : ''}`}>
        <SessionSearchHighlight text={value} query={query} />
      </pre>
    </div>
  );
};

const PathRow: React.FC<{ value: string }> = ({ value }) => (
  value ? <PropertyRow label="Path" value={value} strong /> : null
);

const PropertyRow: React.FC<{ label: string; value: string; strong?: boolean }> = ({ label, value, strong }) => {
  if (!value) {
    return null;
  }
  return (
    <div className={styles.propertyRow}>
      <span>{label}</span>
      <code className={strong ? styles.propertyValueStrong : undefined}>{value}</code>
    </div>
  );
};

const OptionalText: React.FC<{ value: string; className: string }> = ({ value, className }) => (
  value ? <div className={className}>{value}</div> : null
);

const MetaChips: React.FC<{ values: string[] }> = ({ values }) => {
  const visibleValues = values.filter(Boolean);
  if (visibleValues.length === 0) {
    return null;
  }
  return (
    <div className={styles.metaChips}>
      {visibleValues.map((value) => <span key={value}>{value}</span>)}
    </div>
  );
};

function chipValue(label: string, value: unknown): string {
  if (value === undefined || value === null || value === '' || value === false) {
    return '';
  }
  if (value === true) {
    return label;
  }
  if (Array.isArray(value)) {
    return `${label}: ${value.join(', ')}`;
  }
  return `${label}: ${String(value)}`;
}

function toRecord(value: unknown): Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
    ? value as Record<string, unknown>
    : {};
}

function anyField(record: Record<string, unknown>, keys: string[]): unknown {
  for (const key of keys) {
    if (record[key] !== undefined) {
      return record[key];
    }
  }
  const lowerKeys = new Set(keys.map((key) => key.toLowerCase()));
  for (const [entryKey, entryValue] of Object.entries(record)) {
    if (lowerKeys.has(entryKey.toLowerCase()) && entryValue !== undefined) {
      return entryValue;
    }
  }
  return undefined;
}

function stringField(record: Record<string, unknown>, keys: string[]): string {
  const value = anyField(record, keys);
  return typeof value === 'string' ? value : '';
}

function toolPayloadToCopyText(value: unknown, useEmptyObjectFallback: boolean): string {
  if (value === undefined || value === null) {
    return useEmptyObjectFallback ? '{}' : '';
  }
  if (typeof value === 'string') {
    return value;
  }
  return safeJson(value);
}

function safeJson(value: unknown): string {
  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return String(value);
  }
}

export default React.memo(SessionToolExecutionCard);
