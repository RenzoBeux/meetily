import { useCallback, useState } from 'react';
import { invoke, isTauri } from '@tauri-apps/api/core';

/** Mirrors the Rust `InterruptedRecording` returned by `scan_interrupted_recordings`. */
export interface InterruptedRecording {
  folder_path: string;
  title: string;
  created_at: string;
  segment_count: number;
  has_checkpoints: boolean;
  has_audio: boolean;
}

export interface ImportResult {
  meeting_id: string;
  segment_count: number;
  audio: { status?: string; message?: string } | null;
}

/**
 * Filesystem-based crash recovery, independent of the webview/IndexedDB path. Reads the
 * `transcripts.json` + `.checkpoints/` that Rust writes to disk during every recording,
 * so a meeting that never journaled to IndexedDB is still recoverable.
 */
export function useFilesystemRecovery() {
  const [interrupted, setInterrupted] = useState<InterruptedRecording[]>([]);
  const [isScanning, setIsScanning] = useState(false);
  const [isRecovering, setIsRecovering] = useState(false);

  const checkForInterruptedRecordings = useCallback(async (): Promise<InterruptedRecording[]> => {
    // Browser preview has no backend to scan.
    if (!isTauri()) return [];
    setIsScanning(true);
    try {
      const results = await invoke<InterruptedRecording[]>('scan_interrupted_recordings');
      setInterrupted(results);
      return results;
    } catch (e) {
      console.error('Filesystem recovery scan failed:', e);
      setInterrupted([]);
      return [];
    } finally {
      setIsScanning(false);
    }
  }, []);

  const recoverFromFolder = useCallback(async (folderPath: string): Promise<ImportResult> => {
    setIsRecovering(true);
    try {
      return await invoke<ImportResult>('import_interrupted_recording', {
        meetingFolder: folderPath,
      });
    } finally {
      setIsRecovering(false);
    }
  }, []);

  return {
    interrupted,
    isScanning,
    isRecovering,
    checkForInterruptedRecordings,
    recoverFromFolder,
  };
}
