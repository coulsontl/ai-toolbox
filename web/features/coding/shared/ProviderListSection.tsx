import React from 'react';
import { Button, Collapse, Empty, Space, Spin, Typography } from 'antd';
import {
  AppstoreOutlined,
  CheckSquareOutlined,
  DatabaseOutlined,
  PlusOutlined,
  ThunderboltOutlined,
} from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import ProviderBatchToolbar from './providerList/ProviderBatchToolbar';
import ProviderSearchEmpty from './providerList/ProviderSearchEmpty';
import ProviderSearchInput from './providerList/ProviderSearchInput';
import ProviderSortDropdown from './providerList/ProviderSortDropdown';
import type { ProviderSortMode } from './providerList/sortProviders';
import type { ProviderBatchSelection } from './providerList/useProviderBatchSelection';

const { Text } = Typography;

/**
 * The hint block's visual contract: small, secondary-coloured text behind a
 * thin left rule.
 *
 * This lives here rather than in each caller because every caller used to
 * hand-copy the same style object — and the one that forgot silently rendered
 * its hint as full-size body text. The caller still owns the *wording*;
 * only the styling is the component's.
 */
const HINT_BLOCK_STYLE: React.CSSProperties = {
  fontSize: 12,
  color: 'var(--color-text-secondary)',
  borderLeft: '2px solid var(--color-border)',
  paddingLeft: 8,
  marginBottom: 12,
};

export interface ProviderListSectionProps {
  /** Sidebar section anchor id, e.g. `codex-providers`. */
  sectionId: string;

  collapsed: boolean;
  onCollapsedChange: (collapsed: boolean) => void;
  loading?: boolean;

  /** Provider count; drives the empty state. */
  providerCount: number;
  /** Count after the search filter; drives the search-empty state. */
  visibleCount: number;

  /** Provider-level batch selection state (see `useProviderBatchSelection`). */
  batch: ProviderBatchSelection<string>;
  /** Ids eligible for batch selection (already filtered by the caller). */
  batchSelectableIds: string[];

  keyword: string;
  onKeywordChange: (keyword: string) => void;
  sortMode: ProviderSortMode;
  sortModes: readonly ProviderSortMode[];
  onSortModeChange: (mode: ProviderSortMode) => void;
  /**
   * Whether the current sort mode is the one that disables dragging. Callers
   * already compute this to gate the drag grips; passing it on lets the sort
   * control say *why* they are gone.
   */
  dragDisabledBySort?: boolean;

  onBatchTest?: () => void;
  batchTesting?: boolean;
  onOpenCommonConfig?: () => void;
  onAddProvider: () => void;

  /** Extra nodes rendered in the section header label, next to the title (e.g. Gateway chips). */
  headerExtra?: React.ReactNode;

  /**
   * Extra empty-state sentence appended to `common.provider.emptyText`, for
   * tools that can import providers from somewhere specific ("…or import from
   * OpenCode"). Most tools need nothing here.
   */
  emptyTextHint?: React.ReactNode;

  /**
   * Hint block under the toolbar. Callers pass their own **wording only** —
   * the block's styling (small secondary text behind a left rule) is applied
   * by this component, so pass bare `<div>`s rather than a styled wrapper.
   */
  hint?: React.ReactNode;

  /**
   * Rendered in place of nothing when the list is empty or a search matches
   * nothing.
   *
   * The empty and search-empty branches replace `children` wholesale, which is
   * right for provider cards and wrong for anything that is merely *listed*
   * alongside them: a section the user can only reach by having at least one
   * provider, or by clearing their search, has silently gone missing.
   */
  alwaysVisible?: React.ReactNode;
  /** Rendered above the list (below the hint), e.g. the provider cards. */
  children: React.ReactNode;
  /** Import buttons rendered below the list (CC Switch / All API Hub / favorites). */
  footer?: React.ReactNode;
}

/**
 * Shared shell for the "provider list" section on every coding tab.
 *
 * Locks the skeleton the pages had each re-implemented: the Collapse section
 * header with the batch/search/sort/batch-test/common-config/add-provider
 * toolbar, the hint block, the empty and search-empty states, and the import
 * footer. Tool-specific wording and actions are passed in as props or slots so
 * a new CLI inherits the layout instead of copying it.
 */
const ProviderListSection: React.FC<ProviderListSectionProps> = ({
  sectionId,
  collapsed,
  onCollapsedChange,
  loading = false,
  providerCount,
  visibleCount,
  batch,
  batchSelectableIds,
  keyword,
  onKeywordChange,
  sortMode,
  sortModes,
  onSortModeChange,
  dragDisabledBySort = false,
  onBatchTest,
  batchTesting = false,
  onOpenCommonConfig,
  onAddProvider,
  headerExtra,
  emptyTextHint,
  hint,
  alwaysVisible,
  children,
  footer,
}) => {
  const { t } = useTranslation();

  const toolbarButtonStyle: React.CSSProperties = { fontSize: 12 };

  return (
    <div id={sectionId} data-sidebar-section="true" data-sidebar-title={t('common.provider.title')}>
      <Collapse
        style={{ marginBottom: 16 }}
        activeKey={collapsed ? [] : ['providers']}
        onChange={(keys) => onCollapsedChange(!keys.includes('providers'))}
        items={[
          {
            key: 'providers',
            label: (
              <Space size={8} wrap>
                <Text strong>
                  <DatabaseOutlined style={{ marginRight: 8 }} />
                  {t('common.provider.title')}
                </Text>
                {headerExtra}
              </Space>
            ),
            extra: (
              <Space size={4} wrap>
                <Button
                  type="link"
                  size="small"
                  style={{ ...toolbarButtonStyle, display: 'inline-flex', alignItems: 'center', gap: 4 }}
                  onClick={(event) => {
                    event.stopPropagation();
                    if (batch.selectionMode) {
                      batch.exitSelection();
                    } else {
                      batch.enterSelection();
                    }
                  }}
                >
                  <CheckSquareOutlined style={{ fontSize: 13, lineHeight: 1 }} />
                  <span>
                    {batch.selectionMode ? t('common.batch.exit') : t('common.batch.manage')}
                  </span>
                </Button>
                {batch.selectionMode && (
                  <ProviderBatchToolbar
                    hasSelection={batch.hasSelection}
                    visibleCount={batchSelectableIds.length}
                    isAllSelected={batch.isAllSelected}
                    indeterminate={batch.indeterminate}
                    onSelectAll={batch.selectAllFiltered}
                    onBatchDelete={batch.batchDelete}
                    disabled={loading}
                  />
                )}
                <ProviderSearchInput value={keyword} onChange={onKeywordChange} />
                <ProviderSortDropdown
                  mode={sortMode}
                  modes={sortModes}
                  onChange={onSortModeChange}
                  dragDisabledBySort={dragDisabledBySort}
                />
                {onBatchTest && (
                  <Button
                    type="link"
                    size="small"
                    style={toolbarButtonStyle}
                    icon={<ThunderboltOutlined />}
                    loading={batchTesting}
                    onClick={(event) => {
                      event.stopPropagation();
                      onBatchTest();
                    }}
                  >
                    {t('common.batchTest')}
                  </Button>
                )}
                {onOpenCommonConfig && (
                  <Button
                    type="link"
                    size="small"
                    style={toolbarButtonStyle}
                    icon={<AppstoreOutlined />}
                    onClick={(event) => {
                      event.stopPropagation();
                      onOpenCommonConfig();
                    }}
                  >
                    {t('common.provider.commonConfig')}
                  </Button>
                )}
                <Button
                  type="link"
                  size="small"
                  style={toolbarButtonStyle}
                  icon={<PlusOutlined />}
                  onClick={(event) => {
                    event.stopPropagation();
                    onAddProvider();
                  }}
                >
                  {t('common.provider.add')}
                </Button>
              </Space>
            ),
            children: (
              <Spin spinning={loading}>
                {hint && <div style={HINT_BLOCK_STYLE}>{hint}</div>}
                {(providerCount === 0 || visibleCount === 0) && alwaysVisible}
                {providerCount === 0 ? (
                  <Empty
                    description={
                      emptyTextHint ? (
                        <Space orientation="vertical" size={2}>
                          <span>{t('common.provider.emptyText')}</span>
                          <span>{emptyTextHint}</span>
                        </Space>
                      ) : (
                        t('common.provider.emptyText')
                      )
                    }
                    style={{ marginTop: 40 }}
                  />
                ) : visibleCount === 0 ? (
                  <ProviderSearchEmpty />
                ) : (
                  children
                )}
                {footer && <div style={{ marginTop: 12 }}>{footer}</div>}
              </Spin>
            ),
          },
        ]}
      />
    </div>
  );
};

export default ProviderListSection;
