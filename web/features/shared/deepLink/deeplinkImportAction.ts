import { router } from '@/app/routes';
import {
  importFromDeeplinkUnified,
  type DeepLinkImportRequest,
  type DeepLinkImportResult,
  type ImportConflictPolicy,
} from '@/services/deeplinkApi';
import { refreshTrayMenu } from '@/services/appApi';
import { DEEP_LINK_IMPORT_COMPLETED } from '@/constants/configEvents';
import { PROVIDER_SHARE_TOOLS } from './providerTransfer';

/**
 * Map a deep-link `app` to the router path of its CLI tab, so that after a
 * successful import we can switch to that tab and let the user see the result.
 */
const APP_ROUTE_PATH = Object.fromEntries(PROVIDER_SHARE_TOOLS.map((tool) => [tool.app, tool.path]));

/**
 * Persist a deep-link provider request and run the shared post-import
 * follow-ups: switch to the imported tool's tab, notify the matching tool page
 * (if kept-alive) to refresh its provider list, and refresh the tray menu.
 *
 * Shared by the deep-link import confirmation dialog (external `aitoolbox://`
 * links) and the provider share modal's "import to this device" action.
 */
export async function importDeepLinkRequest(request: DeepLinkImportRequest, conflictPolicy: ImportConflictPolicy = 'skip'): Promise<DeepLinkImportResult> {
  const result = await importFromDeeplinkUnified(request, conflictPolicy);

  // Switch to the imported tool's tab so the user sees the result. The
  // matching page (kept alive under KeepAliveOutlet) refreshes its provider
  // list on the dispatched event below; if it was never mounted, navigating
  // to it triggers its initial loadConfig on mount.
  const targetPath = APP_ROUTE_PATH[result.app];
  if (targetPath) {
    try {
      await router.navigate(targetPath);
    } catch (navigationError) {
      console.error('Failed to navigate after provider import:', navigationError);
    }
  }
  window.dispatchEvent(
    new CustomEvent(DEEP_LINK_IMPORT_COMPLETED, {
      detail: { app: result.app, id: result.id },
    }),
  );

  try {
    await refreshTrayMenu();
  } catch (trayError) {
    console.error('Failed to refresh tray menu after deep-link import:', trayError);
  }
  return result;
}
