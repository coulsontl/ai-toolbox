import React from 'react';
import { Input, Modal, Typography } from 'antd';
import { useTranslation } from 'react-i18next';
import {
  calculateImageSize,
  normalizeImageSize,
  parseRatio,
  type SizeTier,
} from '../utils/sizeUtils';
import styles from './SizePickerModal.module.less';

const { Text } = Typography;

const TIERS: SizeTier[] = ['1K', '2K', '4K'];
const RATIOS = [
  { label: '1:1', value: '1:1' },
  { label: '3:2', value: '3:2' },
  { label: '2:3', value: '2:3' },
  { label: '16:9', value: '16:9' },
  { label: '9:16', value: '9:16' },
  { label: '4:3', value: '4:3' },
  { label: '3:4', value: '3:4' },
  { label: '21:9', value: '21:9' },
] as const;

type SizeMode = 'auto' | 'ratio' | 'resolution';

interface SizePickerModalProps {
  currentSize: string;
  open: boolean;
  onClose: () => void;
  onSelect: (size: string) => void;
}

const parseSize = (size: string): { width: string; height: string } | null => {
  const match = size.match(/^\s*(\d+)\s*[xX×]\s*(\d+)\s*$/);
  if (!match) return null;
  return { width: match[1], height: match[2] };
};

const findPresetForSize = (size: string): { tier: SizeTier; ratio: string } | null => {
  const normalized = normalizeImageSize(size);
  for (const tier of TIERS) {
    for (const ratio of RATIOS) {
      if (calculateImageSize(tier, ratio.value) === normalized) {
        return { tier, ratio: ratio.value };
      }
    }
  }
  return null;
};

const SizePickerModal: React.FC<SizePickerModalProps> = ({
  currentSize,
  open,
  onClose,
  onSelect,
}) => {
  const { t } = useTranslation();

  const currentPreset = React.useMemo(() => findPresetForSize(currentSize), [currentSize]);
  const currentParsedSize = React.useMemo(() => parseSize(currentSize), [currentSize]);

  const [mode, setMode] = React.useState<SizeMode>('auto');
  const [tier, setTier] = React.useState<SizeTier>('1K');
  const [ratio, setRatio] = React.useState<string>('1:1');
  const [customRatio, setCustomRatio] = React.useState('16:9');
  const [customWidth, setCustomWidth] = React.useState('1024');
  const [customHeight, setCustomHeight] = React.useState('1024');

  React.useEffect(() => {
    if (!open) return;

    if (!currentSize || currentSize === 'auto') {
      setMode('auto');
    } else if (currentPreset) {
      setMode('ratio');
    } else {
      setMode('resolution');
    }

    setTier(currentPreset?.tier ?? '1K');
    setRatio(currentPreset?.ratio ?? '1:1');
    setCustomWidth(currentParsedSize?.width ?? '1024');
    setCustomHeight(currentParsedSize?.height ?? '1024');
  }, [currentParsedSize, currentPreset, currentSize, open]);

  const activeRatio = ratio === 'custom' ? customRatio : ratio;
  const customRatioValid = ratio !== 'custom' || Boolean(parseRatio(customRatio));

  const previewSize = React.useMemo(() => {
    if (mode === 'auto') return 'auto';

    if (mode === 'ratio') {
      const size = calculateImageSize(tier, activeRatio);
      return size ? normalizeImageSize(size) : '';
    }

    const width = parseInt(customWidth, 10);
    const height = parseInt(customHeight, 10);
    if (Number.isFinite(width) && Number.isFinite(height) && width > 0 && height > 0) {
      return normalizeImageSize(`${width}x${height}`);
    }
    return '';
  }, [activeRatio, customHeight, customWidth, mode, tier]);

  const handleApply = () => {
    if (!previewSize) return;
    onSelect(previewSize);
    onClose();
  };

  return (
    <Modal
      open={open}
      onCancel={onClose}
      onOk={handleApply}
      okButtonProps={{ className: styles.primaryActionButton, disabled: !previewSize }}
      cancelButtonProps={{ className: styles.secondaryActionButton }}
      okText={t('common.confirm')}
      cancelText={t('common.cancel')}
      title={(
        <div className={styles.titleBlock}>
          <div className={styles.titleText}>{t('image.sizePicker.title')}</div>
          <div className={styles.titleHint}>
            {t('image.sizePicker.current')}：{currentSize || 'auto'}
          </div>
        </div>
      )}
      className={styles.modal}
      destroyOnHidden
    >
      <div className={styles.content}>
        <div className={styles.modeTabs}>
          {(['auto', 'ratio', 'resolution'] as const).map((value) => (
            <button
              key={value}
              type="button"
              className={`${styles.modeTab} ${mode === value ? styles.modeTabActive : ''}`}
              onClick={() => setMode(value)}
            >
              {t(`image.sizePicker.modes.${value}`)}
            </button>
          ))}
        </div>

        <section className={styles.sectionCard}>
          {mode === 'auto' && (
            <div className={styles.infoBlock}>
              <Text>{t('image.sizePicker.autoHint')}</Text>
            </div>
          )}

          {mode === 'ratio' && (
            <div className={styles.modeBody}>
              <div className={styles.fieldRow}>
                <div className={styles.fieldLabel}>{t('image.sizePicker.tier')}</div>
                <div className={styles.optionGrid}>
                  {TIERS.map((item) => (
                    <button
                      key={item}
                      type="button"
                      className={`${styles.choiceButton} ${tier === item ? styles.choiceButtonActive : ''}`}
                      onClick={() => setTier(item)}
                    >
                      {item}
                    </button>
                  ))}
                </div>
              </div>

              <div className={styles.fieldRow}>
                <div className={styles.fieldLabel}>{t('image.sizePicker.ratio')}</div>
                <div className={styles.optionGrid}>
                  {RATIOS.map((item) => (
                    <button
                      key={item.value}
                      type="button"
                      className={`${styles.choiceButton} ${ratio === item.value ? styles.choiceButtonActive : ''}`}
                      onClick={() => setRatio(item.value)}
                    >
                      {item.label}
                    </button>
                  ))}
                  <button
                    type="button"
                    className={`${styles.choiceButton} ${ratio === 'custom' ? styles.choiceButtonActive : ''} ${styles.choiceButtonWide}`}
                    onClick={() => setRatio('custom')}
                  >
                    {t('image.sizePicker.customRatio')}
                  </button>
                </div>
              </div>

              {ratio === 'custom' && (
                <div className={styles.fieldRow}>
                  <div className={styles.fieldLabel}>{t('image.sizePicker.customRatio')}</div>
                  <Input
                    value={customRatio}
                    status={customRatioValid ? undefined : 'error'}
                    placeholder="5:4 / 2.39:1"
                    onChange={(event) => setCustomRatio(event.target.value)}
                  />
                </div>
              )}
            </div>
          )}

          {mode === 'resolution' && (
            <div className={styles.modeBody}>
              <div className={styles.fieldRow}>
                <div className={styles.fieldLabel}>{t('image.sizePicker.resolution')}</div>
                <div className={styles.resolutionGrid}>
                  <Input
                    value={customWidth}
                    onChange={(event) => setCustomWidth(event.target.value)}
                    placeholder="1024"
                  />

                  <div className={styles.resolutionDivider}>x</div>

                  <Input
                    value={customHeight}
                    onChange={(event) => setCustomHeight(event.target.value)}
                    placeholder="1024"
                  />
                </div>
              </div>

              <div className={styles.infoBlock}>
                <Text>{t('image.sizePicker.resolutionHint')}</Text>
              </div>
            </div>
          )}
        </section>

        <section className={styles.previewCard}>
          <div className={styles.fieldRow}>
            <div className={styles.fieldLabel}>{t('image.sizePicker.preview')}</div>
            <div className={styles.previewValue}>{previewSize || t('image.sizePicker.invalid')}</div>
          </div>
        </section>
      </div>
    </Modal>
  );
};

export default SizePickerModal;
