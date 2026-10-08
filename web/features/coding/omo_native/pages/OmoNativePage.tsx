import React from 'react';
import { Alert, Spin } from 'antd';
import {
  ApiOutlined,
  AppstoreAddOutlined,
  FileTextOutlined,
  FolderOpenOutlined,
  RobotOutlined,
  SafetyCertificateOutlined,
  SettingOutlined,
} from '@ant-design/icons';
import { useTranslation } from 'react-i18next';
import { openPath } from '@tauri-apps/plugin-opener';
import SectionSidebarLayout, {
  type SidebarSectionMarker,
} from '@/components/layout/SectionSidebarLayout/SectionSidebarLayout';
import SidebarSettingsModal from '@/components/common/SidebarSettingsModal';
import CliManualPathSetting from '@/components/common/CliManualPathSetting';
import FileConfigPreviewModal from '@/components/common/FileConfigPreviewModal';
import CodingPageHeader from '@/features/coding/shared/CodingPageHeader';
import RootDirectoryModal from '@/features/coding/shared/RootDirectoryModal';
import useRootDirectoryConfig from '@/features/coding/shared/useRootDirectoryConfig';
import { GlobalPromptSettings } from '@/features/coding/shared/prompt';
import { SessionManagerPanel } from '@/features/coding/shared/sessionManager';
import { useSettingsStore } from '@/stores';
import { refreshTrayMenu } from '@/services/appApi';
import {
  getOmoNativeRootPathInfo,
  getOmoNativeSettingsConfig,
  listOmoNativeBuiltinProviders,
  listOmoNativeProviders,
  readOmoNativeRuntimeConfig,
  saveOmoNativeSettingsConfig,
} from '@/services/omoNativeApi';
import { omoNativePromptApi } from '@/services/omoNativePromptApi';
import {
  OMO_NATIVE_PROMPT_FILE,
  type OmoNativeBuiltinProvider,
  type OmoNativePathInfo,
  type OmoNativeProvider,
  type OmoNativeRuntimeConfig,
} from '@/types/omoNative';
import OmoNativeProvidersSection from '../components/OmoNativeProvidersSection';
import OmoNativeOtherConfigSection from '../components/OmoNativeOtherConfigSection';
import OmoNativeBuiltinProvidersSection from '../components/OmoNativeBuiltinProvidersSection';
import OmoNativeModelSettingsSection from '../components/OmoNativeModelSettingsSection';
import OmoNativeExtensionsSection from '../components/OmoNativeExtensionsSection';

const OMO_NATIVE_DOCS_URL =
  'https://github.com/code-yeongyu/oh-my-openagent/blob/dev/docs/reference/omo-json.md';

const SIDEBAR_ICON_BY_SECTION_ID: Record<string, React.ReactNode> = {
  'omo-native-model-settings': <RobotOutlined />,
  'omo-native-providers': <ApiOutlined />,
  'omo-native-builtin-channels': <SafetyCertificateOutlined />,
  'omo-native-extensions': <AppstoreAddOutlined />,
  'omo-native-global-prompt': <FileTextOutlined />,
  'omo-native-other-configuration': <SettingOutlined />,
  'omo-native-session-manager': <FolderOpenOutlined />,
};

/**
 * OmO Native 页面。
 *
 * 与其余 coding tab 同构：页头 + **三个区块**（供应商列表 / 全局提示词 / 会话管理）。
 * 早先这里还挂着「生效配置」「Agent·Category 方案」「MCP 与 Skills」「更多选项」
 * 四个 Native 专属区块，2026-10-07 按统一页面形态移除：
 *
 * - **生效配置** 与页头的「预览配置」重复（后者还多给了 settings/models/mcp 三份文件）；
 * - **Agent·Category 方案** 的编辑 UI 撤下，但**后端与托盘一律保留**——
 *   `list/create/apply_omo_native_agents_config*` 等命令、`omo_native_agents_config`
 *   表、托盘方案切换都还在跑，只是当前没有前端入口。**不要当死代码删**
 *   （先例见 `oh_my_pi/AGENTS.md` 对 `listOmpAgents` 的说明）；
 * - **MCP 与 Skills** 原本只是一段指引，两个能力各有顶层的独立页面；
 * - **更多选项** 的内容（CLI 手动路径）在页头 ⋯ 打开的侧栏设置弹窗里已有同一份。
 */
const OmoNativePage: React.FC = () => {
  const { t } = useTranslation();

  const [runtimeConfig, setRuntimeConfig] = React.useState<OmoNativeRuntimeConfig | null>(null);
  const [pathInfo, setPathInfo] = React.useState<OmoNativePathInfo | null>(null);
  // provider 列表由页面持有：供应商区块与「模型设置」区块的下拉共用同一份。
  const [providers, setProviders] = React.useState<OmoNativeProvider[]>([]);
  /** 已配置凭据的内建渠道（`null` = 还在查）。同上，两个区块共用。 */
  const [builtinProviders, setBuiltinProviders] = React.useState<
    OmoNativeBuiltinProvider[] | null
  >(null);
  const [loading, setLoading] = React.useState(true);
  const [loadError, setLoadError] = React.useState<string | null>(null);
  const [previewModalOpen, setPreviewModalOpen] = React.useState(false);
  const [settingsModalOpen, setSettingsModalOpen] = React.useState(false);
  const [promptExpandNonce, setPromptExpandNonce] = React.useState(0);
  const [sessionManagerExpandNonce, setSessionManagerExpandNonce] = React.useState(0);
  const { sidebarHiddenByPage, setSidebarHidden } = useSettingsStore();
  const sidebarHidden = sidebarHiddenByPage.omo_native;

  const loadConfig = React.useCallback(async (silent = false) => {
    if (!silent) setLoading(true);
    setLoadError(null);
    try {
      const [config, rootPathInfo] = await Promise.all([
        readOmoNativeRuntimeConfig(),
        getOmoNativeRootPathInfo(),
      ]);
      setRuntimeConfig(config);
      setPathInfo(rootPathInfo);
    } catch (error) {
      console.error('Failed to load OmO Native runtime config:', error);
      setLoadError((error as Error).message || String(error));
    } finally {
      if (!silent) setLoading(false);
    }
  }, []);

  React.useEffect(() => {
    void loadConfig();
  }, [loadConfig]);

  const loadProviders = React.useCallback(async () => {
    try {
      setProviders(await listOmoNativeProviders());
    } catch (error) {
      console.error('Failed to load OmO Native providers:', error);
      setProviders([]);
    }
  }, []);

  const reloadBuiltinProviders = React.useCallback(async () => {
    try {
      setBuiltinProviders(await listOmoNativeBuiltinProviders());
    } catch (error) {
      console.error('Failed to load OmO built-in providers:', error);
      setBuiltinProviders([]);
    }
  }, []);

  React.useEffect(() => {
    void loadProviders();
    // 内建渠道逐个跑 `omo auth check`（引擎没有批量接口），48 个实测约 8 秒——
    // 所以与配置读取分开跑，页面不必等它才渲染。
    void reloadBuiltinProviders();
  }, [loadProviders, reloadBuiltinProviders]);

  const {
    rootDirectoryModalOpen,
    setRootDirectoryModalOpen,
    getRootDirectoryModalProps,
    handleSaveRootDirectory,
    handleResetRootDirectory,
  } = useRootDirectoryConfig({
    t,
    translationKeyPrefix: 'omoNative',
    defaultConfig: '{}',
    loadConfig,
    getCommonConfig: getOmoNativeSettingsConfig,
    saveCommonConfig: async (input) => {
      await saveOmoNativeSettingsConfig(input);
      await refreshTrayMenu();
    },
  });

  const sidebarSections = React.useMemo<SidebarSectionMarker[]>(
    () => [
      { id: 'omo-native-model-settings', title: t('omoNative.modelSettings.title'), order: 1 },
      { id: 'omo-native-providers', title: t('omoNative.providers.title'), order: 2 },
      { id: 'omo-native-builtin-channels', title: t('omoNative.official.title'), order: 3 },
      { id: 'omo-native-extensions', title: t('common.cliExtensions.title'), order: 4 },
      { id: 'omo-native-global-prompt', title: t('common.prompt.title'), order: 5 },
      { id: 'omo-native-other-configuration', title: t('omoNative.otherConfig.title'), order: 6 },
      { id: 'omo-native-session-manager', title: t('sessionManager.title'), order: 7 },
    ],
    [t],
  );

  const handleOpenRootFolder = async () => {
    if (!pathInfo?.path) return;
    try {
      await openPath(pathInfo.path);
    } catch (error) {
      console.error('Failed to open root folder:', error);
    }
  };

  if (loading && !runtimeConfig) {
    return (
      <div style={{ padding: 48, textAlign: 'center' }}>
        <Spin />
      </div>
    );
  }

  return (
    <SectionSidebarLayout
      sidebarTitle={t('omoNative.title')}
      sidebarHidden={sidebarHidden}
      markerAttr="data-omo-native-sidebar-section"
      getIcon={(id: string) => SIDEBAR_ICON_BY_SECTION_ID[id] ?? null}
      sections={sidebarSections}
      onSectionSelect={(id) => {
        if (id === 'omo-native-global-prompt') {
          setPromptExpandNonce((value) => value + 1);
        }
        if (id === 'omo-native-session-manager') {
          setSessionManagerExpandNonce((value) => value + 1);
        }
      }}
    >
      <div>
        <CodingPageHeader
          title={t('omoNative.title')}
          docsUrl={OMO_NATIVE_DOCS_URL}
          // 与 OMP / Pi 同类语义：显示的是引擎状态目录，不是单个配置文件。
          configPath={pathInfo?.path ?? '~/.omo/agent'}
          onPreviewConfig={() => setPreviewModalOpen(true)}
          onCustomizeConfig={() => setRootDirectoryModalOpen(true)}
          onOpenFolder={() => void handleOpenRootFolder()}
          onRefresh={() => void loadConfig()}
          onMoreOptions={() => setSettingsModalOpen(true)}
        />

        {loadError && (
          <Alert
            type="error"
            showIcon
            style={{ marginBottom: 16 }}
            title={t('omoNative.loadError')}
            description={loadError}
          />
        )}

        <OmoNativeModelSettingsSection
          runtimeConfig={runtimeConfig}
          providers={providers}
          builtinProviders={builtinProviders ?? []}
          onSaved={setRuntimeConfig}
        />

        <OmoNativeProvidersSection
          providers={providers}
          setProviders={setProviders}
          loadProviders={loadProviders}
          // 「模型设置」的默认渠道不可删（与 Pi 同规则）：这个 key 由页面转交，
          // 两处读的是同一份 `settings.json` 视图，改完默认值立即生效。
          defaultProviderKey={runtimeConfig?.modelSettings.providerKey}
        />

        <OmoNativeBuiltinProvidersSection
          providers={builtinProviders}
          reloadProviders={reloadBuiltinProviders}
          authContent={runtimeConfig?.authContent}
          onSaved={loadConfig}
        />

        <div
          id="omo-native-extensions"
          data-sidebar-section="true"
          data-sidebar-title={t('common.cliExtensions.title')}
        >
          <OmoNativeExtensionsSection
            // 扩展变更会改 `settings.json`（packages / extensions 过滤器），
            // 也就是「模型设置」读的同一份文件。
            onChanged={loadConfig}
          />
        </div>

        <div
          id="omo-native-global-prompt"
          data-sidebar-section="true"
          data-sidebar-title={t('common.prompt.title')}
        >
          <GlobalPromptSettings
            key={`omo-native-prompt-${promptExpandNonce}`}
            toolName="OmO"
            promptFileName={OMO_NATIVE_PROMPT_FILE}
            service={omoNativePromptApi}
            collapseKey="omo-native-prompt"
            defaultExpanded={promptExpandNonce > 0}
            onUpdated={loadConfig}
          />
        </div>

        <OmoNativeOtherConfigSection
          sharedBase={runtimeConfig?.sharedBase ?? {}}
          onSaved={loadConfig}
        />

        <div
          id="omo-native-session-manager"
          data-sidebar-section="true"
          data-sidebar-title={t('sessionManager.title')}
        >
          <SessionManagerPanel
            tool="omo_native"
            expandNonce={sessionManagerExpandNonce}
            refreshNonce={0}
          />
        </div>
      </div>

      <RootDirectoryModal
        open={rootDirectoryModalOpen}
        {...getRootDirectoryModalProps(pathInfo)}
        onCancel={() => setRootDirectoryModalOpen(false)}
        onSubmit={handleSaveRootDirectory}
        onReset={handleResetRootDirectory}
      />

      <FileConfigPreviewModal
        open={previewModalOpen}
        onClose={() => setPreviewModalOpen(false)}
        title={t('omoNative.preview.title')}
        files={[
          {
            // 显示 `omo.jsonc` 的**整份原文**（含 `[opencode]` / `[native]` 块、
            // 控制键与注释）。
            //
            // ⚠️ 两次修正的经过：早先这一栏显示 `[native]` 块并在块为空时弹一段
            // 解释，但页面收敛后本页已没有任何入口能写 `[native]` 块，于是那句提示
            // 永远消不掉（2026-10-07 用户：「我本地已经有2个渠道了啊，这个提示是啥
            // 意思？新用户进来也会困惑」）。随后改成只显示共享键，仍然只是文件的一
            // 部分——用户要看的是文件本身（用户随后要求「直接展示完整的 json」）。
            key: 'config',
            label: runtimeConfig?.configPath?.split(/[\\/]/).pop() || 'omo.jsonc',
            content: runtimeConfig?.configContent,
            // Monaco 只注册了 `json` 这一个语言 id（没有 `jsonc`）——写 `jsonc`
            // 会静默退化成纯文本、没有语法高亮。`json` 语言的诊断默认
            // `allowComments: true`（`monaco.contribution.js`），注释与尾逗号
            // 都能正常高亮，不会报错。
            language: 'json',
          },
          {
            key: 'settings',
            label: 'settings.json',
            content: runtimeConfig?.settingsContent,
            language: 'json',
          },
          {
            key: 'models',
            label: 'models.json',
            content: runtimeConfig?.modelsContent,
            language: 'json',
          },
          {
            key: 'mcp',
            label: 'mcp.json',
            content: runtimeConfig?.mcpContent,
            language: 'json',
          },
          {
            // 凭据文件也放进来：本应用是配置管理器，密钥要能看得到。
            key: 'auth',
            label: 'auth.json',
            content: runtimeConfig?.authContent,
            language: 'json',
          },
        ]}
      />

      <SidebarSettingsModal
        open={settingsModalOpen}
        onClose={() => setSettingsModalOpen(false)}
        sidebarVisible={!sidebarHidden}
        onSidebarVisibleChange={async (visible) => {
          await setSidebarHidden('omo_native', !visible);
        }}
      >
        <CliManualPathSetting commandName="omo" labelKey="subModules.omoNative" />
      </SidebarSettingsModal>
    </SectionSidebarLayout>
  );
};

export default OmoNativePage;
