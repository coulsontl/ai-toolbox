import type { SessionMessage, SessionMessageBlock } from '../../types';

import { hasSessionCommandTags, parseSessionCommandTags } from './commandTags';
import { getToolDisplayName } from './toolCatalog';

export function getMessageBlocks(message: SessionMessage): SessionMessageBlock[] {
  if (message.blocks && message.blocks.length > 0) {
    return message.blocks;
  }

  if (!message.content.trim()) {
    return [];
  }

  return [{
    kind: message.role.toLowerCase() === 'system' ? 'system' : 'text',
    text: message.content,
    variant: message.role.toLowerCase() === 'system' ? 'system' : undefined,
  }];
}

export function isToolBlock(block: SessionMessageBlock): boolean {
  return block.kind === 'tool_call'
    || block.kind === 'tool_result'
    || block.kind === 'tool_execution';
}

export function getBlockText(block: SessionMessageBlock): string {
  const parts = [
    block.title,
    block.text,
    block.toolName,
    block.normalizedToolName,
    valueToSearchText(block.input),
    valueToSearchText(block.output),
    valueToSearchText(block.metadata),
  ];

  return parts.filter(Boolean).join('\n');
}

export function getMessagePreview(message: SessionMessage, maxLength = 96): string {
  const blocks = getMessageBlocks(message);
  const text = blocks
    .map((block) => {
      if (isToolBlock(block)) {
        return getToolPreview(block);
      }
      if (hasSessionCommandTags(block.text || '')) {
        const parsedCommand = parseSessionCommandTags(block.text || '');
        return [
          parsedCommand.commandName,
          parsedCommand.commandMessage,
          parsedCommand.commandArgs,
          parsedCommand.remainingText,
        ].filter(Boolean).join(' ');
      }
      return block.text || '';
    })
    .filter(Boolean)
    .join(' ');

  const collapsed = collapsePreview(text || message.content);
  if (collapsed.length <= maxLength) {
    return collapsed;
  }
  return `${collapsed.slice(0, maxLength)}...`;
}

export function isMeaningfulPreview(value: string | undefined): boolean {
  const normalized = collapsePreview(value || '').toLowerCase();
  if (!normalized) {
    return false;
  }
  if (normalized === 'unknown' || normalized === 'tool: unknown') {
    return false;
  }
  if (/^\[?tool:\s*unknown\]?$/i.test(normalized)) {
    return false;
  }
  if (/^\((user|assistant|tool|system) message\)$/i.test(normalized)) {
    return false;
  }
  return true;
}

export function valueToSearchText(value: unknown): string {
  if (value === null || value === undefined) {
    return '';
  }
  if (typeof value === 'string') {
    return value;
  }
  if (typeof value === 'number' || typeof value === 'boolean') {
    return String(value);
  }
  if (Array.isArray(value)) {
    return value.map(valueToSearchText).filter(Boolean).join('\n');
  }
  if (typeof value === 'object') {
    const record = value as Record<string, unknown>;
    const preferred = ['text', 'content', 'stdout', 'stderr', 'output', 'error', 'file_path', 'path', 'command', 'query', 'pattern'];
    const direct = preferred
      .map((key) => valueToSearchText(record[key]))
      .filter(Boolean)
      .join('\n');

    if (direct) {
      return direct;
    }

    try {
      return JSON.stringify(value, null, 2);
    } catch {
      return String(value);
    }
  }

  return '';
}

function getToolPreview(block: SessionMessageBlock): string {
  const directName = meaningfulName(block.toolName);
  if (directName) {
    return directName;
  }

  const normalizedName = meaningfulName(block.normalizedToolName);
  if (normalizedName) {
    return getToolDisplayName(normalizedName, undefined);
  }

  return meaningfulName(block.title) || '';
}

function meaningfulName(value: string | undefined): string {
  const collapsed = collapsePreview(value || '');
  return isMeaningfulPreview(collapsed) ? collapsed : '';
}

function collapsePreview(value: string): string {
  return value
    .replace(/<[^>]*>/g, ' ')
    .replace(/\s+/g, ' ')
    .trim();
}
