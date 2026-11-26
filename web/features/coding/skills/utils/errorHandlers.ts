import React from 'react';
import { Modal, message } from 'antd';
import type { TFunction } from 'i18next';
import { formatGitError, isGitError } from './gitErrorParser';
import type { ToolOption } from '../types';

/**
 * Check if error is a SKILL_EXISTS error
 */
export function isSkillExistsError(errMsg: string): boolean {
  return errMsg.includes('SKILL_EXISTS|');
}

/**
 * Extract skill name from SKILL_EXISTS error
 */
export function extractSkillName(errMsg: string): string {
  const match = errMsg.match(/SKILL_EXISTS\|(.+)/);
  return match ? match[1] : '';
}

/**
 * Parse TARGET_EXISTS error
 */
export function parseTargetExistsError(errMsg: string): { targetPath: string } | null {
  if (!errMsg.includes('TARGET_EXISTS|')) return null;
  const match = errMsg.match(/TARGET_EXISTS\|(.+)/);
  return match ? { targetPath: match[1] } : null;
}

/**
 * Show git error or general error message
 */
export function showGitError(
  errMsg: string,
  t: TFunction,
  allTools?: ToolOption[]
): void {
  // Handle TOOL_NOT_INSTALLED|toolKey|skillsPath error
  if (errMsg.startsWith('TOOL_NOT_INSTALLED|')) {
    const parts = errMsg.split('|');
    const toolKey = parts[1] || '';
    const skillsPath = parts[2] || '';
    const tool = allTools?.find((t) => t.id === toolKey);
    const toolName = tool?.label || toolKey;
    Modal.error({
      title: t('common.error'),
      content: React.createElement('div', null, [
        React.createElement('p', { key: 'msg' }, t('skills.errors.toolNotInstalled', { tool: toolName })),
        React.createElement('p', { key: 'path', style: { fontSize: 12, color: 'var(--color-text-tertiary)' } },
          t('skills.errors.checkSkillsPath', { path: skillsPath })
        ),
      ]),
    });
    return;
  }

  if (isGitError(errMsg)) {
    Modal.error({
      title: t('common.error'),
      content: React.createElement('div', {
        style: { whiteSpace: 'pre-wrap', maxHeight: '400px', overflow: 'auto' }
      }, formatGitError(errMsg, t)),
      width: 600,
    });
  } else {
    message.error(errMsg);
  }
}

/**
 * Confirm overwriting a target that already exists in a tool
 */
export function confirmTargetOverwrite(
  skillName: string,
  toolLabel: string,
  targetPath: string,
  t: TFunction
): Promise<boolean> {
  return new Promise((resolve) => {
    Modal.confirm({
      title: t('skills.targetExists.title'),
      content: t('skills.targetExists.message', { skill: skillName, tool: toolLabel, path: targetPath }),
      okText: t('skills.overwrite.confirm'),
      okType: 'danger',
      cancelText: t('skills.overwrite.skip'),
      onOk: () => resolve(true),
      onCancel: () => resolve(false),
    });
  });
}
