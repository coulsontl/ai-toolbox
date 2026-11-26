import React from 'react';
import { Modal, Input, Button, type InputRef } from 'antd';
import { useTranslation } from 'react-i18next';
import {
  suggestSkillAlias,
  validateSkillAlias,
  type SkillAliasValidationError,
} from '../../utils/skillAlias';
import styles from './SkillConflictModal.module.less';

export type SkillConflictResolution =
  | { action: 'alias'; alias: string }
  | { action: 'overwrite' }
  | { action: 'overwriteAll' }
  | { action: 'skip' };

export interface SkillConflictModalProps {
  open: boolean;
  skillName: string;
  suggestedAlias: string;
  existingNames: ReadonlySet<string>;
  isBatch?: boolean;
  hasMore?: boolean;
  onResolve: (resolution: SkillConflictResolution) => void;
}

export interface PromptSkillConflictOptions {
  skillName: string;
  existingNames: ReadonlySet<string>;
  repoHint?: string | null;
  isBatch?: boolean;
  hasMore?: boolean;
}

const EMPTY_EXISTING_NAMES: ReadonlySet<string> = new Set();

const ALIAS_ERROR_KEYS: Record<SkillAliasValidationError, string> = {
  empty: 'skills.overwrite.aliasErrors.empty',
  tooLong: 'skills.overwrite.aliasErrors.tooLong',
  startsWithDot: 'skills.overwrite.aliasErrors.startsWithDot',
  endsWithDot: 'skills.overwrite.aliasErrors.endsWithDot',
  invalidChars: 'skills.overwrite.aliasErrors.invalidChars',
  exists: 'skills.overwrite.aliasErrors.exists',
};

export const SkillConflictModal: React.FC<SkillConflictModalProps> = ({
  open,
  skillName,
  suggestedAlias,
  existingNames,
  isBatch = false,
  hasMore = false,
  onResolve,
}) => {
  const { t } = useTranslation();
  const [alias, setAlias] = React.useState(suggestedAlias);
  const inputRef = React.useRef<InputRef>(null);

  React.useEffect(() => {
    if (!open) {
      return;
    }
    setAlias(suggestedAlias);

    const frameId = window.requestAnimationFrame(() => {
      inputRef.current?.focus();
      inputRef.current?.select();
    });
    return () => window.cancelAnimationFrame(frameId);
  }, [open, skillName, suggestedAlias]);

  const validationError = React.useMemo(
    () => validateSkillAlias(alias, existingNames),
    [alias, existingNames],
  );

  const handleConfirmAlias = () => {
    if (validationError) {
      return;
    }
    onResolve({ action: 'alias', alias: alias.trim() });
  };

  return (
    <Modal
      title={t('skills.overwrite.title')}
      open={open}
      onCancel={() => onResolve({ action: 'skip' })}
      footer={null}
      width={480}
      zIndex={1050}
    >
      <p className={styles.description}>
        {t('skills.overwrite.messageWithAlias', { name: skillName })}
      </p>

      <div className={styles.formRow}>
        <label className={styles.label} htmlFor="skill-conflict-alias-input">
          {t('skills.overwrite.aliasLabel')}
        </label>
        <div className={styles.inputGroup}>
          <Input
            id="skill-conflict-alias-input"
            ref={inputRef}
            value={alias}
            onChange={(e) => setAlias(e.target.value)}
            onPressEnter={handleConfirmAlias}
            placeholder={t('skills.overwrite.aliasPlaceholder')}
            status={validationError ? 'error' : undefined}
          />
          {validationError ? (
            <div className={styles.errorText}>{t(ALIAS_ERROR_KEYS[validationError])}</div>
          ) : (
            <div className={styles.hintText}>{t('skills.overwrite.aliasHint')}</div>
          )}
        </div>
      </div>

      <div className={styles.footer}>
        <Button
          className={styles.cancelButton}
          onClick={() => onResolve({ action: 'skip' })}
        >
          {isBatch ? t('skills.overwrite.skip') : t('common.cancel')}
        </Button>

        <div className={styles.actions}>
          <Button danger onClick={() => onResolve({ action: 'overwrite' })}>
            {t('skills.overwrite.confirm')}
          </Button>
          {isBatch && hasMore && (
            <Button danger onClick={() => onResolve({ action: 'overwriteAll' })}>
              {t('skills.overwrite.overwriteAll')}
            </Button>
          )}
          <Button
            type="primary"
            disabled={Boolean(validationError)}
            onClick={handleConfirmAlias}
          >
            {t('skills.overwrite.installWithAlias')}
          </Button>
        </div>
      </div>
    </Modal>
  );
};

interface ActiveConflictPrompt extends PromptSkillConflictOptions {
  suggestedAlias: string;
}

/**
 * Manage an imperative async conflict prompt backed by a controlled SkillConflictModal.
 */
export function useSkillConflictPrompt() {
  const [request, setRequest] = React.useState<ActiveConflictPrompt | null>(null);
  const resolverRef = React.useRef<((resolution: SkillConflictResolution) => void) | null>(null);

  React.useEffect(() => {
    return () => {
      if (resolverRef.current) {
        resolverRef.current({ action: 'skip' });
        resolverRef.current = null;
      }
    };
  }, []);

  const promptSkillConflict = React.useCallback(
    (options: PromptSkillConflictOptions): Promise<SkillConflictResolution> => {
      if (resolverRef.current) {
        resolverRef.current({ action: 'skip' });
        resolverRef.current = null;
      }

      const suggestedAlias = suggestSkillAlias(
        options.skillName,
        options.existingNames,
        options.repoHint,
      );

      return new Promise<SkillConflictResolution>((resolve) => {
        resolverRef.current = resolve;
        setRequest({
          ...options,
          suggestedAlias,
        });
      });
    },
    [],
  );

  const handleResolve = React.useCallback((resolution: SkillConflictResolution) => {
    const resolve = resolverRef.current;
    resolverRef.current = null;
    setRequest(null);
    resolve?.(resolution);
  }, []);

  const conflictModal = (
    <SkillConflictModal
      open={Boolean(request)}
      skillName={request?.skillName ?? ''}
      suggestedAlias={request?.suggestedAlias ?? ''}
      existingNames={request?.existingNames ?? EMPTY_EXISTING_NAMES}
      isBatch={request?.isBatch}
      hasMore={request?.hasMore}
      onResolve={handleResolve}
    />
  );

  return {
    conflictModal,
    promptSkillConflict,
  };
}
