import React from 'react';
import { Button, Dropdown, Tooltip } from 'antd';
import { useTranslation } from 'react-i18next';
import { SortAscendingOutlined } from '@ant-design/icons';
import type { ProviderSortMode } from './sortProviders';

interface ProviderSortDropdownProps {
  mode: ProviderSortMode;
  /** Sort modes the current tab supports (e.g. no "created" in file-based tabs). */
  modes: readonly ProviderSortMode[];
  onChange: (mode: ProviderSortMode) => void;
  /**
   * Set when the list is sorted by something other than the stored order, which
   * also disables dragging. Explains why the drag grips are gone — without it
   * the only symptom is "I cannot drag any more", with nothing on screen
   * naming the cause.
   */
  dragDisabledBySort?: boolean;
}

/**
 * Sort-mode picker for provider list section headers, rendered inside an antd
 * Collapse `extra`. Clicks must never reach the Collapse header or they would
 * toggle the section:
 * - the shell span stops DOM-level bubbling from the trigger button;
 * - menu items live in a portal attached to `document.body`, so DOM-level
 *   bubbling never reaches the header — but React synthetic events bubble
 *   through the *component* tree (the portal's React ancestors), which ends
 *   at the header's onClick. The menu `onClick` therefore also stops the
 *   synthetic `domEvent`.
 */
const ProviderSortDropdown: React.FC<ProviderSortDropdownProps> = ({
  mode,
  modes,
  onChange,
  dragDisabledBySort = false,
}) => {
  const { t } = useTranslation();

  const items = modes.map((sortMode) => ({
    key: sortMode,
    label: t(`common.providerSort.${sortMode}`),
  }));

  return (
    <span onClick={(event) => event.stopPropagation()}>
      <Tooltip title={dragDisabledBySort ? t('common.providerSort.dragDisabledHint') : ''}>
        <Dropdown
          menu={{
            items,
            selectable: true,
            selectedKeys: [mode],
            onClick: ({ key, domEvent }) => {
              domEvent.stopPropagation();
              onChange(key as ProviderSortMode);
            },
          }}
          trigger={['click']}
        >
          <Button
            type="link"
            size="small"
            style={{ fontSize: 12 }}
            icon={<SortAscendingOutlined />}
          >
            {t('common.providerSort.label')}
          </Button>
        </Dropdown>
      </Tooltip>
    </span>
  );
};

export default ProviderSortDropdown;
