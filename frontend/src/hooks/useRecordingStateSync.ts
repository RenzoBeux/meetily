import { useState } from 'react';

interface UseRecordingStateSyncReturn {
  isRecordingDisabled: boolean;
  setIsRecordingDisabled: (value: boolean) => void;
}

/**
 * Owns the transient "recording controls disabled" flag.
 *
 * Backend recording state is the sole responsibility of `RecordingStateContext`,
 * which already polls at 500ms and listens to start/stop events. This hook used
 * to run its own redundant 1s poll gated on `window.__TAURI__` — a global that is
 * never set in Tauri v2, so the poll never actually ran. That dead branch (and
 * the now-unneeded recording-sync params) has been removed to avoid a duplicate
 * source of truth.
 */
export function useRecordingStateSync(): UseRecordingStateSyncReturn {
  const [isRecordingDisabled, setIsRecordingDisabled] = useState(false);

  return {
    isRecordingDisabled,
    setIsRecordingDisabled,
  };
}
