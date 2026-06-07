import React from 'react';

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
}

const SessionToolExecutionCard: React.FC<SessionToolExecutionCardProps> = ({ block, query }) => {
  const normalizedBlock = normalizeToolBlock(block);
  const normalizedToolName = normalizedBlock.normalizedToolName || 'unknown';
  const variant = getToolVariant(normalizedBlock.toolName, normalizedToolName);
  const Icon = getToolIcon(normalizedToolName, normalizedBlock.toolName);
  const title = getToolDisplayName(normalizedToolName, normalizedBlock.toolName);

  return (
    <SessionRendererCard
      icon={Icon}
      title={title}
      variant={variant}
      status={normalizedBlock.status}
      meta={normalizedBlock.toolId ? <code className={styles.toolIdBadge}>{normalizedBlock.toolId}</code> : null}
    >
      <ToolBody normalizedToolName={normalizedToolName} block={normalizedBlock} query={query} />
      {normalizedBlock.kind === 'tool_execution' || normalizedBlock.kind === 'tool_result' ? (
        <div className={styles.toolResultSection}>
          <div className={styles.sectionLabel}>Result</div>
          <SessionToolResultBlock output={normalizedBlock.output} query={query} status={normalizedBlock.status} />
        </div>
      ) : null}
    </SessionRendererCard>
  );
};

interface ToolBodyProps {
  normalizedToolName: string;
  block: SessionMessageBlock;
  query: string;
}

const ToolBody: React.FC<ToolBodyProps> = ({ normalizedToolName, block, query }) => {
  const input = toRecord(block.input);
  switch (normalizedToolName) {
    case 'bash':
      return <BashBody input={input} query={query} />;
    case 'read':
      return <ReadBody input={input} />;
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

const BashBody: React.FC<{ input: Record<string, unknown>; query: string }> = ({ input, query }) => (
  <div className={styles.toolBodyStack}>
    <OptionalText value={stringField(input, ['description'])} className={styles.toolDescription} />
    <CodePanel title="Command" value={stringField(input, ['command', 'cmd']) || safeJson(input)} query={query} tone="terminal" />
    <MetaChips values={[
      chipValue('timeout', input.timeout ?? input.timeout_ms),
      chipValue('background', input.run_in_background ?? input.runInBackground),
    ]}
    />
  </div>
);

const ReadBody: React.FC<{ input: Record<string, unknown> }> = ({ input }) => (
  <div className={styles.toolBodyStack}>
    <PathRow value={stringField(input, ['file_path', 'filePath', 'path'])} />
    <MetaChips values={[
      chipValue('offset', input.offset),
      chipValue('limit', input.limit),
    ]}
    />
  </div>
);

const WriteBody: React.FC<{ input: Record<string, unknown>; query: string }> = ({ input, query }) => (
  <div className={styles.toolBodyStack}>
    <PathRow value={stringField(input, ['file_path', 'filePath', 'path'])} />
    {stringField(input, ['content']) ? <CodePanel title="Content" value={stringField(input, ['content'])} query={query} /> : null}
  </div>
);

const EditBody: React.FC<{ input: Record<string, unknown>; query: string }> = ({ input, query }) => {
  const edits = Array.isArray(input.edits) ? input.edits : [];
  return (
    <div className={styles.toolBodyStack}>
      <PathRow value={stringField(input, ['file_path', 'filePath', 'path'])} />
      {edits.length > 0 ? (
        <div className={styles.editList}>
          {edits.map((edit, index) => (
            <CodePanel key={index} title={`Edit ${index + 1}`} value={safeJson(edit)} query={query} />
          ))}
        </div>
      ) : (
        <div className={styles.diffGrid}>
          <CodePanel title="Before" value={stringField(input, ['old_string', 'oldString'])} query={query} />
          <CodePanel title="After" value={stringField(input, ['new_string', 'newString'])} query={query} />
        </div>
      )}
      <MetaChips values={[chipValue('replace all', input.replace_all ?? input.replaceAll)]} />
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
    <OptionalText value={stringField(input, ['description'])} className={styles.toolDescription} />
    <MetaChips values={[
      chipValue('subagent', input.subagent_type ?? input.subagentType),
      chipValue('model', input.model),
      chipValue('background', input.run_in_background ?? input.runInBackground),
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
  <CodePanel title="Input" value={safeJson(value ?? {})} query={query} />
);

const CodePanel: React.FC<{ title: string; value: string; query: string; tone?: 'default' | 'terminal' }> = ({
  title,
  value,
  query,
  tone = 'default',
}) => (
  <div className={styles.preBlockShell}>
    <div className={styles.preBlockTitle}>{title}</div>
    <pre className={`${styles.preBlock}${tone === 'terminal' ? ` ${styles.terminalPreBlock}` : ''}`}>
      <SessionSearchHighlight text={value} query={query} />
    </pre>
  </div>
);

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

function stringField(record: Record<string, unknown>, keys: string[]): string {
  for (const key of keys) {
    const value = record[key];
    if (typeof value === 'string') {
      return value;
    }
  }
  return '';
}

function safeJson(value: unknown): string {
  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return String(value);
  }
}

export default SessionToolExecutionCard;
