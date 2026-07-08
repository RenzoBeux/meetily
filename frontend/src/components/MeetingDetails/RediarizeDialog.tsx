import React, { useState, useEffect, useRef } from 'react';
import { Users, Loader2, AlertCircle, CheckCircle2, X, Download } from 'lucide-react';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '../ui/dialog';
import { Button } from '../ui/button';
import { invoke } from '@tauri-apps/api/core';
import { listen, UnlistenFn } from '@tauri-apps/api/event';
import { toast } from 'sonner';

interface RediarizeDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  meetingId: string;
  onComplete?: () => void;
}

interface DiarizationProgressPayload {
  status: 'starting' | 'running' | 'aligning' | 'done' | 'error';
  meeting_id?: string;
  speakers?: number;
  segments?: number;
  segments_updated?: number;
  reason?: string;
}

interface ModelDownloadProgress {
  name: string;
  downloaded: number;
  total: number;
  percent: number;
}

interface RediarizationResult {
  meeting_id: string;
  speakers: number;
  segments_updated: number;
}

const STATUS_COPY: Record<string, string> = {
  starting: 'Preparing…',
  running: 'Identifying speakers…',
  aligning: 'Applying speaker labels…',
  done: 'Done',
  error: 'Failed',
};

export function RediarizeDialog({
  open,
  onOpenChange,
  meetingId,
  onComplete,
}: RediarizeDialogProps) {
  const [isProcessing, setIsProcessing] = useState(false);
  const [stage, setStage] = useState<DiarizationProgressPayload['status'] | null>(null);
  const [download, setDownload] = useState<ModelDownloadProgress | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [numSpeakers, setNumSpeakers] = useState<string>('');

  const onCompleteRef = useRef(onComplete);
  const onOpenChangeRef = useRef(onOpenChange);
  useEffect(() => { onCompleteRef.current = onComplete; }, [onComplete]);
  useEffect(() => { onOpenChangeRef.current = onOpenChange; }, [onOpenChange]);

  // Reset on closed → open
  const prevOpenRef = useRef(false);
  useEffect(() => {
    const wasOpen = prevOpenRef.current;
    prevOpenRef.current = open;
    if (open && !wasOpen) {
      setIsProcessing(false);
      setStage(null);
      setDownload(null);
      setError(null);
      setNumSpeakers('');
    }
  }, [open]);

  // Listen for events while the dialog is mounted/open
  useEffect(() => {
    if (!open) return;
    const unlisteners: UnlistenFn[] = [];
    let cancelled = false;

    (async () => {
      const unlistenProgress = await listen<DiarizationProgressPayload>(
        'diarization-progress',
        (event) => {
          if (event.payload.meeting_id !== meetingId) return;
          setStage(event.payload.status);
          if (event.payload.status === 'done') {
            const n = event.payload.speakers ?? 0;
            toast.success(`Identified ${n} speaker${n === 1 ? '' : 's'}`);
          } else if (event.payload.status === 'error') {
            setError(event.payload.reason ?? 'Unknown error');
            setIsProcessing(false);
          }
        },
      );
      if (cancelled) { unlistenProgress(); return; }
      unlisteners.push(unlistenProgress);

      const unlistenDownload = await listen<ModelDownloadProgress>(
        'diarization-model-download-progress',
        (event) => {
          setDownload(event.payload);
        },
      );
      if (cancelled) { unlistenDownload(); unlisteners.forEach(u => u()); return; }
      unlisteners.push(unlistenDownload);
    })();

    return () => {
      cancelled = true;
      unlisteners.forEach((u) => u());
    };
  }, [open, meetingId]);

  const handleStart = async () => {
    setIsProcessing(true);
    setError(null);
    setStage('starting');
    setDownload(null);

    try {
      const parsed = parseInt(numSpeakers, 10);
      const parsedNumSpeakers = Number.isFinite(parsed) && parsed >= 1 ? parsed : undefined;
      await invoke<RediarizationResult>('rediarize_meeting', {
        meetingId,
        numSpeakers: parsedNumSpeakers,
      });
      onCompleteRef.current?.();
      onOpenChangeRef.current(false);
    } catch (err: unknown) {
      const msg = typeof err === 'string' ? err : err instanceof Error ? err.message : String(err);
      setError(msg);
      setIsProcessing(false);
    }
  };

  const handleOpenChange = (next: boolean) => {
    if (!next && isProcessing) return; // can't close while running
    onOpenChange(next);
  };

  const handleEscape = (e: KeyboardEvent) => {
    if (isProcessing) e.preventDefault();
  };
  const handleInteractOutside = (e: Event) => {
    if (isProcessing) e.preventDefault();
  };

  const showingDownload = download !== null && download.percent < 100;

  return (
    <Dialog open={open} onOpenChange={handleOpenChange}>
      <DialogContent
        className="sm:max-w-[450px]"
        onEscapeKeyDown={handleEscape}
        onInteractOutside={handleInteractOutside}
      >
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            {error ? (
              <>
                <AlertCircle className="h-5 w-5 text-red-600" />
                Speaker identification failed
              </>
            ) : isProcessing ? (
              <>
                <Loader2 className="h-5 w-5 animate-spin text-blue-600" />
                Identifying speakers…
              </>
            ) : (
              <>
                <Users className="h-5 w-5 text-blue-600" />
                Identify speakers
              </>
            )}
          </DialogTitle>
          <DialogDescription>
            {error
              ? 'An error occurred while identifying speakers.'
              : isProcessing
                ? STATUS_COPY[stage ?? 'starting']
                : 'Run speaker diarization on this meeting. Existing speaker labels (mic / system or earlier speaker_N) will be replaced based on the audio.'}
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4 py-4">
          {!isProcessing && !error && (
            <div className="space-y-3">
              <div className="text-sm text-muted-foreground">
                First run downloads ~115&nbsp;MB of speaker models. Subsequent runs are
                fast. Diarization runs locally — your audio never leaves the machine.
              </div>
              <div className="space-y-1.5">
                <label htmlFor="num-speakers" className="text-sm font-medium">
                  Number of speakers{' '}
                  <span className="font-normal text-muted-foreground">(optional)</span>
                </label>
                <input
                  id="num-speakers"
                  type="number"
                  min={1}
                  max={20}
                  inputMode="numeric"
                  placeholder="Auto-detect"
                  value={numSpeakers}
                  onChange={(e) => setNumSpeakers(e.target.value)}
                  className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500"
                />
                <p className="text-xs text-muted-foreground">
                  Leave blank to detect automatically. Set the exact count (e.g. 3) if
                  auto-detect splits one person into several speakers.
                </p>
              </div>
            </div>
          )}

          {isProcessing && (
            <div className="space-y-3">
              {showingDownload ? (
                <div className="space-y-2">
                  <div className="flex items-center gap-2 text-sm">
                    <Download className="h-4 w-4 text-blue-600" />
                    Downloading {download!.name} model…
                  </div>
                  <div className="w-full bg-gray-200 rounded-full h-2">
                    <div
                      className="bg-blue-600 h-2 rounded-full transition-all duration-200"
                      style={{ width: `${Math.min(download!.percent, 100)}%` }}
                    />
                  </div>
                  <div className="text-xs text-muted-foreground">
                    {download!.percent}% ({Math.round(download!.downloaded / 1_048_576)} MB
                    of {Math.round(download!.total / 1_048_576)} MB)
                  </div>
                </div>
              ) : (
                <>
                  <div className="flex items-center gap-2 text-sm">
                    <Loader2 className="h-4 w-4 animate-spin text-blue-600" />
                    {STATUS_COPY[stage ?? 'starting']}
                  </div>
                  <div className="w-full bg-gray-200 rounded-full h-2 overflow-hidden">
                    <div className="bg-blue-600 h-2 rounded-full animate-pulse w-1/2" />
                  </div>
                  <div className="text-xs text-muted-foreground">
                    Approximate runtime: ~1–2 min per 30 min of audio on CPU.
                  </div>
                </>
              )}
            </div>
          )}

          {error && (
            <div className="bg-red-50 border border-red-200 rounded-lg p-3">
              <p className="text-sm text-red-800">{error}</p>
            </div>
          )}
        </div>

        <DialogFooter>
          {!isProcessing && !error && (
            <>
              <Button variant="outline" onClick={() => onOpenChange(false)}>
                Cancel
              </Button>
              <Button onClick={handleStart} className="bg-blue-600 hover:bg-blue-700">
                <Users className="h-4 w-4 mr-2" />
                Identify speakers
              </Button>
            </>
          )}
          {isProcessing && (
            <Button variant="outline" disabled>
              <Loader2 className="h-4 w-4 mr-2 animate-spin" />
              Working…
            </Button>
          )}
          {error && (
            <>
              <Button variant="outline" onClick={() => onOpenChange(false)}>
                Close
              </Button>
              <Button
                variant="outline"
                onClick={() => {
                  setError(null);
                  setStage(null);
                }}
              >
                Try again
              </Button>
            </>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
