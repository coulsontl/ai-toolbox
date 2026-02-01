import { useEffect, useRef } from 'react';
import { useMcpStore } from '../stores/mcpStore';

export const useMcp = () => {
  const { servers, tools, loading, showInTray, scanResult, fetchServers, fetchTools, fetchShowInTray, loadScanResult } = useMcpStore();
  const hasLoadedRef = useRef(false);

  useEffect(() => {
    // Prevent duplicate loading on re-renders and StrictMode double-mount
    if (hasLoadedRef.current) return;
    hasLoadedRef.current = true;

    // Load essential data first (sequentially to reduce lock contention)
    const loadData = async () => {
      await fetchServers();
      await fetchTools();
      await fetchShowInTray();
    };
    loadData();
  }, [fetchServers, fetchTools, fetchShowInTray]);

  // Note: Scan is NOT triggered automatically on page load.
  // It should only be triggered manually from the ImportMcpModal.

  return {
    servers,
    tools,
    loading,
    showInTray,
    scanResult,
    refresh: fetchServers,
    refreshTools: fetchTools,
    triggerScan: loadScanResult, // Expose for manual triggering in import modal
  };
};

export default useMcp;
