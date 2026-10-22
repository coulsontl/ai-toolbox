/**
 * The Tauri command table `CodexPage` needs, shared by the fixtures that mount
 * it.
 *
 * Two fixtures mount the real page now (the drag/list one and the card-view
 * one), and a copied command table is the failure this repo keeps hitting: the
 * page gains a command, one copy grows a stub, and the other fixture dies with
 * "Unstubbed fixture command" — or worse, silently renders a different page
 * than the one under test. The `default` branch throws on purpose, so a missing
 * stub is loud rather than a mysterious render failure.
 *
 * `state` is the fixture's own object: the table reads the provider list from
 * it, writes reorders back into it, and appends every call to `state.requests`
 * so checks can assert what the page asked the backend for.
 */
export const CODEX_GATEWAY_CLI_STATUS = {
  cli_key: 'codex',
  state: 'direct',
  dot: 'idle',
  can_takeover: true,
  can_restore_direct: false,
  gateway_origin: null,
  runtime_root: null,
  managed_targets: [],
  mode: null,
  primary_provider_id: null,
  provider_priorities: [],
  message: null,
};

export const createCodexPageInvoke = (state) => async (command, args) => {
  state.requests.push({ command, args: structuredClone(args ?? null) });
  switch (command) {
    // --- provider list ---------------------------------------------------
    case 'get_codex_config_file_path': return 'C:\\Users\\tester\\.codex\\config.toml';
    case 'get_codex_root_path_info': return { path: 'C:\\Users\\tester\\.codex', source: 'default' };
    case 'list_codex_providers': return structuredClone(state.providers);
    // Per provider, like the real command: the page calls this once for every
    // row, and handing each of them the same list would give a custom provider
    // an account block it does not have — a fixture that renders a page the app
    // never produces.
    case 'list_codex_official_accounts':
      return structuredClone((state.officialAccounts ?? [])
        .filter((account) => account.providerId === args.providerId));
    case 'reorder_codex_providers': {
      const byId = new Map(state.providers.map(provider => [provider.id, provider]));
      state.providers = args.ids.map(id => byId.get(id)).filter(Boolean);
      return null;
    }
    case 'list_opencode_favorite_providers': return [];
    case 'refresh_tray_menu': return null;
    case 'get_codex_common_config': return null;
    case 'read_codex_settings': return {};

    // --- provider list UI state -----------------------------------------
    case 'get_provider_list_state':
      return {
        sort_modes: { codex: state.sortMode },
        last_used: state.lastUsed ?? {},
        view_modes: { codex: state.viewMode },
      };
    case 'save_provider_sort_mode': {
      state.sortMode = args.mode;
      return null;
    }
    case 'save_provider_view_mode': {
      state.viewMode = args.mode;
      return null;
    }
    case 'record_provider_last_used': return null;

    // --- gateway ---------------------------------------------------------
    case 'proxy_gateway_cli_status': return structuredClone(CODEX_GATEWAY_CLI_STATUS);
    case 'proxy_gateway_cli_statuses': return [structuredClone(CODEX_GATEWAY_CLI_STATUS)];
    case 'proxy_gateway_supported_cli_keys': return ['codex'];
    case 'proxy_gateway_status': return { running: false, mode: null };
    case 'proxy_gateway_get_settings': return {};

    // --- plugins / memories / sessions ----------------------------------
    case 'get_codex_plugin_runtime_status':
      return { feature_enabled: false, runtime_available: false, runtime_error: null, plugin_count: 0 };
    case 'list_codex_installed_plugins': return [];
    case 'list_codex_plugin_workspace_roots': return [];
    case 'list_codex_memories': return { entries: [], root_path: 'C:\\Users\\tester\\.codex\\memories' };
    case 'list_tool_sessions':
      return { sessions: [], total: 0, has_more: false, next_cursor: null };

    // --- global prompt list ---------------------------------------------
    case 'list_codex_prompt_configs': return [];

    // --- feature-availability probes ------------------------------------
    case 'has_all_api_hub_extension': return false;
    case 'has_cc_switch_db': return false;

    // --- event plumbing --------------------------------------------------
    case 'plugin:event|listen': return 1;
    case 'plugin:event|unlisten': return null;
    case 'plugin:opener|reveal_item_in_dir': return null;
    case 'plugin:opener|open_url': {
      (state.openedUrls ??= []).push(args.url);
      return null;
    }

    default:
      throw new Error('Unstubbed fixture command: ' + command);
  }
};