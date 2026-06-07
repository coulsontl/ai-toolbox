import type { SessionMessageBlock } from '../../types';

import { getNormalizedToolName, getToolVariant, inferNormalizedToolNameFromInput } from './toolCatalog';

export function pairToolBlocks(blocks: SessionMessageBlock[]): SessionMessageBlock[] {
  const result: SessionMessageBlock[] = [];
  const pendingCallIndex = new Map<string, number>();

  blocks.forEach((block) => {
    if (block.kind === 'tool_call') {
      const normalizedBlock = normalizeToolBlock(block);
      if (normalizedBlock.toolId) {
        pendingCallIndex.set(normalizedBlock.toolId, result.length);
      }
      result.push(normalizedBlock);
      return;
    }

    if (block.kind === 'tool_result' && block.toolId) {
      const callIndex = pendingCallIndex.get(block.toolId);
      if (callIndex !== undefined) {
        const callBlock = result[callIndex];
        result[callIndex] = normalizeToolBlock({
          ...callBlock,
          kind: 'tool_execution',
          output: block.output,
          isError: block.isError,
          status: inferDisplayStatus({ ...callBlock, output: block.output, isError: block.isError, status: block.status }),
        });
        pendingCallIndex.delete(block.toolId);
        return;
      }
    }

    result.push(normalizeToolBlock(block));
  });

  return result;
}

export function normalizeToolBlock(block: SessionMessageBlock): SessionMessageBlock {
  if (!['tool_call', 'tool_result', 'tool_execution'].includes(block.kind)) {
    return block;
  }

  const normalizedToolName = normalizeBlockToolName(block);
  return {
    ...block,
    normalizedToolName,
    variant: block.variant || getToolVariant(block.toolName, normalizedToolName),
    status: inferDisplayStatus({ ...block, normalizedToolName }),
  };
}

export function inferDisplayStatus(block: SessionMessageBlock): string {
  if (block.isError) {
    return 'error';
  }

  const status = block.status?.toLowerCase();
  if (status && ['error', 'failed', 'failure', 'interrupted'].includes(status)) {
    return 'error';
  }
  if (status && ['warning', 'warn'].includes(status)) {
    return 'warning';
  }
  if (status && ['pending', 'running'].includes(status)) {
    return 'pending';
  }
  if (status && ['success', 'ok', 'completed'].includes(status)) {
    return 'success';
  }

  return block.output === undefined ? 'pending' : 'success';
}

function normalizeBlockToolName(block: SessionMessageBlock): string {
  const directName = getNormalizedToolName(block.toolName);
  if (directName !== 'unknown') {
    return directName;
  }

  const providedName = getNormalizedToolName(block.normalizedToolName);
  if (providedName !== 'unknown') {
    return providedName;
  }

  return inferNormalizedToolNameFromInput(block.input);
}
