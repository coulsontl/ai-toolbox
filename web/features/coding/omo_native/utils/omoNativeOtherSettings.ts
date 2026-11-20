import type { OmoNativeOtherSettingsInput } from '@/types/omoNative';

export type OtherConfigPayloadResult =
  | { ok: true; payload: Record<string, unknown> }
  | { ok: false; reason: 'invalid' | 'notObject' };

/**
 * 把编辑器失焦时的原文/解析结果归一成保存载荷。
 *
 * - 语法错误：调用方已经跳过保存；
 * - 空文本：按 `{}` 保存（用户显式清空）；
 * - 非空但不是对象（`[]` / `null` / 数字 / 字符串）：报错并跳过，**不能**强转
 *   `{}`——`buildOtherSettingsPatch` 会把它解释成「删除所有键」。
 */
export function parseOtherConfigPayload(
  rawText: string,
  parsed: unknown | null,
  isValid: boolean,
): OtherConfigPayloadResult {
  if (!isValid) {
    return { ok: false, reason: 'invalid' };
  }
  if (rawText.trim() === '') {
    return { ok: true, payload: {} };
  }
  if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) {
    return { ok: false, reason: 'notObject' };
  }
  return { ok: true, payload: parsed as Record<string, unknown> };
}

/**
 * 只提交相对基线新增/修改与删除的键。
 *
 * `settings.json` 在页面打开期间仍会被引擎写入（`tipsHistory` 等）；如果整份
 * 覆盖「其他设置」切片，页面内存副本会把这些新写入回退。后端拿到 `set` /
 * `remove` 后在盘上最新内容上打补丁。
 */
export function buildOtherSettingsPatch(
  baseline: Record<string, unknown>,
  next: Record<string, unknown>,
): OmoNativeOtherSettingsInput {
  const set: Record<string, unknown> = {};
  for (const [key, value] of Object.entries(next)) {
    if (!(key in baseline) || JSON.stringify(baseline[key]) !== JSON.stringify(value)) {
      set[key] = value;
    }
  }
  const remove = Object.keys(baseline).filter((key) => !(key in next));
  return { set, remove };
}
