import type { TFunction } from "i18next";

type SyncMode = "ssh" | "wsl";

/** Locale keys live under settings.wsl.defaultMappings.* (shared SSH + WSL UI). */
const I18N_DEFAULT_MAPPINGS_BASE = "settings.wsl.defaultMappings";

/**
 * Built-in file mappings: single source per row. Keep in sync with
 * `default_file_mappings()` in `tauri/src/coding/ssh/commands.rs` and `wsl/commands.rs`
 * (`id` + `name` / rustDefaultName).
 */
const BUILTIN_FILE_MAPPINGS = [
	{
		id: "opencode-main",
		rustDefaultName: "OpenCode 主配置",
		i18nSuffix: "opencodeMain",
	},
	{
		id: "opencode-oh-my",
		rustDefaultName: "Oh My OpenCode 配置",
		i18nSuffix: "opencodeOhMy",
	},
	{
		id: "opencode-oh-my-slim",
		rustDefaultName: "Oh My OpenCode Slim 配置",
		i18nSuffix: "opencodeOhMySlim",
	},
	{
		id: "opencode-auth",
		rustDefaultName: "OpenCode 认证信息",
		i18nSuffix: "opencodeAuth",
	},
	{
		id: "opencode-plugins",
		rustDefaultName: "OpenCode 插件文件",
		i18nSuffix: "opencodePlugins",
	},
	{
		id: "opencode-prompt",
		rustDefaultName: "OpenCode 全局提示词",
		i18nSuffix: "opencodePrompt",
	},
	{
		id: "claude-settings",
		rustDefaultName: "Claude Code 设置",
		i18nSuffix: "claudeSettings",
	},
	{
		id: "claude-config",
		rustDefaultName: "Claude Code 配置",
		i18nSuffix: "claudeConfig",
	},
	{
		id: "claude-prompt",
		rustDefaultName: "Claude Code 全局提示词",
		i18nSuffix: "claudePrompt",
	},
	{ id: "codex-auth", rustDefaultName: "Codex 认证", i18nSuffix: "codexAuth" },
	{
		id: "codex-config",
		rustDefaultName: "Codex 配置",
		i18nSuffix: "codexConfig",
	},
	{
		id: "codex-prompt",
		rustDefaultName: "Codex 全局提示词",
		i18nSuffix: "codexPrompt",
	},
	{
		id: "openclaw-config",
		rustDefaultName: "OpenClaw 配置",
		i18nSuffix: "openclawConfig",
	},
] as const;

/** Map stable `id` and Rust default `name` (zh) → i18n key suffix. */
const BUILTIN_MAPPING_SUFFIX_BY_LABEL: ReadonlyMap<string, string> = (() => {
	const m = new Map<string, string>();
	for (const row of BUILTIN_FILE_MAPPINGS) {
		m.set(row.id, row.i18nSuffix);
		m.set(row.rustDefaultName, row.i18nSuffix);
	}
	return m;
})();

const LEGACY_SSH_DEFAULT_MAPPING_PREFIX = "settings.ssh.defaultMappings.";

function tDefaultMappingLabel(suffix: string, t: TFunction): string {
	return t(`${I18N_DEFAULT_MAPPINGS_BASE}.${suffix}`);
}

/** Returns translated label for a known built-in mapping, or null (caller passes through). */
function translateBuiltinMappingLabel(
	value: string,
	t: TFunction,
): string | null {
	if (value.startsWith(LEGACY_SSH_DEFAULT_MAPPING_PREFIX)) {
		return t(
			`${I18N_DEFAULT_MAPPINGS_BASE}.${value.slice(LEGACY_SSH_DEFAULT_MAPPING_PREFIX.length)}`,
		);
	}
	const suffix = BUILTIN_MAPPING_SUFFIX_BY_LABEL.get(value);
	return suffix !== undefined ? tDefaultMappingLabel(suffix, t) : null;
}

const translateJoinedParts = (value: string, mode: SyncMode, t: TFunction) => {
	if (!value.includes("; ")) {
		return translateSyncMessage(value, mode, t);
	}

	return value
		.split("; ")
		.map((item) => translateSyncMessage(item, mode, t))
		.join("; ");
};

const withDetail = (
	key: string,
	detail: string,
	mode: SyncMode,
	t: TFunction,
	vars?: Record<string, unknown>,
) =>
	t(key, {
		...vars,
		detail: translateSyncMessage(detail, mode, t),
	});

export const translateDefaultMappingName = (value: string, t: TFunction) => {
	if (!value) {
		return value;
	}
	return translateBuiltinMappingLabel(value, t) ?? value;
};

export const isBuiltInDefaultMappingName = (id: string, name: string) => {
	if (!id || !name) {
		return false;
	}
	const suffix = BUILTIN_MAPPING_SUFFIX_BY_LABEL.get(id);
	if (suffix === undefined) {
		return false;
	}
	return BUILTIN_MAPPING_SUFFIX_BY_LABEL.get(name) === suffix;
};

export const translateSyncMessage = (
	value: string,
	mode: SyncMode,
	t: TFunction,
): string => {
	if (!value) {
		return value;
	}

	const trimmed = value.trim();

	if (trimmed.includes("; ")) {
		return translateJoinedParts(trimmed, mode, t);
	}

	const builtinLine = translateBuiltinMappingLabel(trimmed, t);
	if (builtinLine !== null) {
		return builtinLine;
	}

	const mappingErrorMatch = trimmed.match(/^(.+?): (.+)$/);
	if (mappingErrorMatch) {
		const headTranslated = translateBuiltinMappingLabel(
			mappingErrorMatch[1],
			t,
		);
		if (headTranslated !== null) {
			return `${headTranslated}: ${translateSyncMessage(mappingErrorMatch[2], mode, t)}`;
		}
	}

	const patterns: Array<[RegExp, (...args: string[]) => string]> = [
		[/^SSH 同步未启用$/, () => t("settings.syncMessages.sshSyncDisabled")],
		[
			/^另一个同步操作正在进行中$/,
			() => t("settings.syncMessages.syncInProgress"),
		],
		[
			/^SSH 连接失败: (.+)$/,
			(detail) =>
				withDetail(
					"settings.syncMessages.sshConnectionFailed",
					detail,
					mode,
					t,
				),
		],
		[
			/^连接超时: ([^:]+):(\d+)$/,
			(host, port) =>
				t("settings.syncMessages.connectionTimeout", { host, port }),
		],
		[
			/^连接到 ([^:]+):(\d+) 失败: (.+)$/,
			(host, port, detail) =>
				withDetail(
					"settings.syncMessages.connectToHostFailed",
					detail,
					mode,
					t,
					{ host, port },
				),
		],
		[
			/^没有可用的 SSH 连接配置$/,
			() => t("settings.syncMessages.noSshConnectionConfig"),
		],
		[
			/^SSH 会话未建立$/,
			() => t("settings.syncMessages.sshSessionNotEstablished"),
		],
		[
			/^密码认证失败: 用户名或密码错误$/,
			() => t("settings.syncMessages.passwordAuthRejected"),
		],
		[
			/^密码认证失败: (.+)$/,
			(detail) =>
				withDetail("settings.syncMessages.passwordAuthFailed", detail, mode, t),
		],
		[
			/^公钥认证失败: 密钥不被服务器接受$/,
			() => t("settings.syncMessages.keyAuthRejected"),
		],
		[
			/^公钥认证失败: (.+)$/,
			(detail) =>
				withDetail("settings.syncMessages.keyAuthFailed", detail, mode, t),
		],
		[
			/^不支持的认证方式: (.+)$/,
			(method) => t("settings.syncMessages.unsupportedAuthMethod", { method }),
		],
		[
			/^解析私钥内容失败: (.+)$/,
			(detail) =>
				withDetail(
					"settings.syncMessages.privateKeyParseFailed",
					detail,
					mode,
					t,
				),
		],
		[
			/^加载私钥文件失败: (.+)$/,
			(detail) =>
				withDetail(
					"settings.syncMessages.privateKeyLoadFailed",
					detail,
					mode,
					t,
				),
		],
		[
			/^未提供私钥路径或私钥内容$/,
			() => t("settings.syncMessages.privateKeyMissing"),
		],
		[
			/^获取 RSA hash 算法失败: (.+)$/,
			(detail) =>
				withDetail("settings.syncMessages.rsaHashFailed", detail, mode, t),
		],
		[
			/^打开 SSH channel 失败: (.+)$/,
			(detail) =>
				withDetail(
					"settings.syncMessages.openSshChannelFailed",
					detail,
					mode,
					t,
				),
		],
		[
			/^打开 channel 失败: (.+)$/,
			(detail) =>
				withDetail("settings.syncMessages.openChannelFailed", detail, mode, t),
		],
		[
			/^执行远程命令失败: (.+)$/,
			(detail) =>
				withDetail(
					"settings.syncMessages.execRemoteCommandFailed",
					detail,
					mode,
					t,
				),
		],
		[
			/^执行命令失败: (.+)$/,
			(detail) =>
				withDetail("settings.syncMessages.execCommandFailed", detail, mode, t),
		],
		[
			/^远程命令退出码 (\d+): (.+)$/,
			(code, detail) =>
				withDetail(
					"settings.syncMessages.remoteExitCodeWithDetail",
					detail,
					mode,
					t,
					{ code },
				),
		],
		[
			/^远程命令退出码 (\d+)$/,
			(code) => t("settings.syncMessages.remoteExitCode", { code }),
		],
		[
			/^写入 stdin 失败: (.+)$/,
			(detail) =>
				withDetail("settings.syncMessages.writeStdinFailed", detail, mode, t),
		],
		[
			/^发送 EOF 失败: (.+)$/,
			(detail) =>
				withDetail("settings.syncMessages.sendEofFailed", detail, mode, t),
		],
		[
			/^打开 SFTP channel 失败: (.+)$/,
			(detail) =>
				withDetail(
					"settings.syncMessages.openSftpChannelFailed",
					detail,
					mode,
					t,
				),
		],
		[
			/^请求 SFTP 子系统失败: (.+)$/,
			(detail) =>
				withDetail(
					"settings.syncMessages.requestSftpSubsystemFailed",
					detail,
					mode,
					t,
				),
		],
		[
			/^初始化 SFTP 会话失败: (.+)$/,
			(detail) =>
				withDetail(
					"settings.syncMessages.initSftpSessionFailed",
					detail,
					mode,
					t,
				),
		],
		[
			/^获取远程 home 路径失败: (.+)$/,
			(detail) =>
				withDetail("settings.syncMessages.remoteHomeFailed", detail, mode, t),
		],
		[
			/^拒绝同步到危险路径: '(.+)'$/,
			(path) => t("settings.syncMessages.dangerousSyncPath", { path }),
		],
		[
			/^拒绝删除危险路径: '(.+)'$/,
			(path) => t("settings.syncMessages.dangerousDeletePath", { path }),
		],
		[
			/^目录替换失败: (.+)$/,
			(detail) =>
				withDetail(
					"settings.syncMessages.directoryReplaceFailed",
					detail,
					mode,
					t,
				),
		],
		[
			/^无效的 glob 模式: (.+)$/,
			(detail) =>
				withDetail("settings.syncMessages.invalidGlobPattern", detail, mode, t),
		],
		[
			/^文件 (.+) 编码不是 UTF-8（可能是 GBK\/GB2312），请手动转换后重试。\n修复方法: (.+)$/,
			(path, fix) => t("settings.syncMessages.fileNotUtf8", { path, fix }),
		],
		[
			/^文件 (.+) 内容疑似二进制或已损坏，请检查文件内容是否正确$/,
			(path) => t("settings.syncMessages.fileLooksBinary", { path }),
		],
		[
			/^MCP sync: (.+)$/,
			(detail) =>
				withDetail("settings.syncMessages.mcpSyncFailed", detail, mode, t),
		],
		[
			/^Skills sync: (.+)$/,
			(detail) =>
				withDetail("settings.syncMessages.skillsSyncFailed", detail, mode, t),
		],
		[
			/^Onboarding sync: (.+)$/,
			(detail) =>
				withDetail(
					"settings.syncMessages.onboardingSyncFailed",
					detail,
					mode,
					t,
				),
		],
		[
			/^WSL MCP 同步已跳过：(.+)$/,
			(detail) =>
				withDetail("settings.syncMessages.wslMcpSyncSkipped", detail, mode, t),
		],
		[
			/^WSL ~\/\.claude\.json 同步已跳过：文件解析失败，请检查该文件格式是否正确。\((.+)\)$/,
			(detail) =>
				withDetail(
					"settings.syncMessages.wslClaudeJsonSyncSkipped",
					detail,
					mode,
					t,
				),
		],
		[
			/^OpenCode\/Codex 配置同步部分失败：(.+)$/,
			(detail) =>
				withDetail(
					"settings.syncMessages.openCodeCodexConfigSyncPartialFailed",
					detail,
					mode,
					t,
				),
		],
		[
			/^OpenCode\/Codex MCP 同步已跳过：(.+)$/,
			(detail) =>
				withDetail(
					"settings.syncMessages.openCodeCodexMcpSyncSkipped",
					detail,
					mode,
					t,
				),
		],
	];

	for (const [pattern, formatter] of patterns) {
		const match = trimmed.match(pattern);
		if (match) {
			return formatter(...match.slice(1));
		}
	}

	return trimmed;
};
