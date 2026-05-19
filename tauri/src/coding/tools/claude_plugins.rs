//! Claude Code plugin discovery helpers shared by MCP and Skills modules.

use std::path::PathBuf;

use crate::coding::claude_code::plugin_state;

/// Resolved plugin info returned to callers.
#[derive(Debug, Clone)]
pub struct PluginInfo {
    pub plugin_id: String,
    pub display_name: String,
    pub install_path: PathBuf,
}

/// Read installed Claude Code plugins using the current runtime location.
///
/// Returns an empty list when there are no installed plugins.
pub async fn get_installed_plugins(db: &crate::db::SqliteDbState) -> Vec<PluginInfo> {
    match plugin_state::list_claude_installed_plugins(db).await {
        Ok(plugins) => plugins
            .into_iter()
            .filter_map(|plugin| {
                let install_path = plugin.install_path?;
                let install_path = PathBuf::from(install_path);
                if !install_path.exists() {
                    return None;
                }

                Some(PluginInfo {
                    plugin_id: plugin.plugin_id,
                    display_name: plugin.name,
                    install_path,
                })
            })
            .collect(),
        Err(_) => Vec::new(),
    }
}
