import React from 'react';
import { Button, Modal, Tag, Tooltip, Typography, message } from 'antd';
import { EyeOutlined, FolderOpenOutlined } from '@ant-design/icons';
import { revealItemInDir } from '@tauri-apps/plugin-opener';
import { useTranslation } from 'react-i18next';
import MarkdownPreview from '@/components/common/MarkdownPreview';
import type { AntigravityLocalRuleFile } from '@/types/antigravity';
import styles from './AntigravityLocalRuleFiles.module.less';

const { Text } = Typography;

interface AntigravityLocalRuleFilesProps {
  files: AntigravityLocalRuleFile[];
}

function formatFileSize(bytes: number): string {
  if (bytes < 1024) {
    return `${bytes} B`;
  }
  return `${(bytes / 1024).toFixed(1)} KB`;
}

const AntigravityLocalRuleFiles: React.FC<AntigravityLocalRuleFilesProps> = ({ files }) => {
  const { t } = useTranslation();
  const [previewFile, setPreviewFile] = React.useState<AntigravityLocalRuleFile | null>(null);

  const otherFiles = React.useMemo(
    () => files.filter((file) => !file.isManagedTarget),
    [files],
  );

  if (otherFiles.length === 0) {
    return null;
  }

  const handleReveal = async (file: AntigravityLocalRuleFile) => {
    try {
      await revealItemInDir(file.fullPath);
    } catch (error) {
      console.error('Failed to reveal Antigravity rule file:', error);
      const errorMsg = error instanceof Error ? error.message : String(error);
      message.error(errorMsg || t('common.error'));
    }
  };

  const renderKindTag = (file: AntigravityLocalRuleFile) => {
    if (file.kind === 'shared_gemini') {
      return (
        <Tag color="warning" className={styles.tag}>
          {t('antigravity.promptRules.tagSharedGemini')}
        </Tag>
      );
    }
    if (file.kind === 'rules_dir') {
      return (
        <Tag className={styles.tag}>
          {t('antigravity.promptRules.tagRulesDir')}
        </Tag>
      );
    }
    return null;
  };

  return (
    <>
      <div className={styles.container}>
        <div className={styles.header}>
          <span className={styles.title}>
            {t('antigravity.promptRules.otherLocalFilesTitle', { count: otherFiles.length })}
          </span>
        </div>
        <div className={styles.list}>
          {otherFiles.map((file) => (
            <div key={file.relativePath} className={styles.row}>
              <div className={styles.rowLeft}>
                <Tooltip title={file.fullPath}>
                  <span className={styles.pathText}>{file.relativePath}</span>
                </Tooltip>
                {renderKindTag(file)}
                <Text className={styles.sizeText}>{formatFileSize(file.sizeBytes)}</Text>
              </div>
              <div className={styles.actions}>
                <Button
                  type="text"
                  size="small"
                  icon={<EyeOutlined />}
                  onClick={() => setPreviewFile(file)}
                >
                  {t('antigravity.promptRules.previewFile')}
                </Button>
                <Tooltip title={t('antigravity.openFolder')}>
                  <Button
                    type="text"
                    size="small"
                    icon={<FolderOpenOutlined />}
                    onClick={() => void handleReveal(file)}
                  />
                </Tooltip>
              </div>
            </div>
          ))}
        </div>
      </div>

      <Modal
        open={Boolean(previewFile)}
        title={previewFile ? previewFile.relativePath : undefined}
        onCancel={() => setPreviewFile(null)}
        footer={null}
        width={820}
      >
        {previewFile && (
          <div>
            <div className={styles.previewPath}>{previewFile.fullPath}</div>
            <div className={styles.previewBox}>
              <MarkdownPreview content={previewFile.content} />
            </div>
          </div>
        )}
      </Modal>
    </>
  );
};

export default AntigravityLocalRuleFiles;
