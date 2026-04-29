import React, { useEffect, useState } from 'react';
import { Users, Loader2, ExternalLink } from 'lucide-react';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '../ui/dialog';
import { Button } from '../ui/button';
import { Input } from '../ui/input';
import { toast } from 'sonner';
import {
  loadHfToken,
  runDiarization,
  saveHfToken,
} from '@/services/diarizationService';
import Analytics from '@/lib/analytics';

interface DiarizeDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  meetingId: string;
  onComplete?: () => Promise<void> | void;
}

export function DiarizeDialog({
  open,
  onOpenChange,
  meetingId,
  onComplete,
}: DiarizeDialogProps) {
  const [hfToken, setHfToken] = useState('');
  const [numSpeakersInput, setNumSpeakersInput] = useState('');
  const [isRunning, setIsRunning] = useState(false);

  useEffect(() => {
    if (open) {
      setHfToken(loadHfToken() ?? '');
      setNumSpeakersInput('');
    }
  }, [open]);

  const handleRun = async () => {
    setIsRunning(true);
    saveHfToken(hfToken);
    Analytics.trackButtonClick('run_diarization', 'meeting_details');
    try {
      // Parse the speaker-count hint. Empty input means "let pyannote
      // auto-detect". Bad input (non-positive integer) is silently dropped.
      const parsed = parseInt(numSpeakersInput.trim(), 10);
      const numSpeakers =
        Number.isFinite(parsed) && parsed > 0 ? parsed : undefined;

      const result = await runDiarization(meetingId, {
        hfToken: hfToken || undefined,
        numSpeakers,
      });
      toast.success(
        `Identified ${result.total_speakers} speaker${result.total_speakers === 1 ? '' : 's'} across ${result.updated} segments`,
      );
      onOpenChange(false);
      if (onComplete) await onComplete();
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      toast.error('Diarization failed', { description: msg });
    } finally {
      setIsRunning(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={(o) => !isRunning && onOpenChange(o)}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <Users className="h-4 w-4" />
            Identify speakers
          </DialogTitle>
          <DialogDescription>
            Runs speaker diarization on the saved meeting audio and replaces the
            "You" / "Others" tags with per-speaker IDs (speaker_1, speaker_2…).
            This can take a few minutes for long meetings.
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-3">
          <div className="text-sm text-gray-700">
            Requires a HuggingFace access token with access to{' '}
            <a
              href="https://huggingface.co/pyannote/speaker-diarization-3.1"
              target="_blank"
              rel="noopener noreferrer"
              className="text-blue-600 underline inline-flex items-center gap-1"
            >
              pyannote/speaker-diarization-3.1
              <ExternalLink className="h-3 w-3" />
            </a>
            . Free, but you must accept the model terms once.
          </div>
          <div>
            <label className="text-sm font-medium" htmlFor="hf-token">
              HuggingFace token (optional if set as HF_TOKEN on the backend)
            </label>
            <Input
              id="hf-token"
              type="password"
              autoComplete="off"
              placeholder="hf_..."
              value={hfToken}
              onChange={(e) => setHfToken(e.target.value)}
              disabled={isRunning}
            />
            <p className="text-xs text-gray-500 mt-1">
              Stored locally in this browser only.
            </p>
          </div>

          <div>
            <label className="text-sm font-medium" htmlFor="num-speakers">
              Number of speakers (optional)
            </label>
            <Input
              id="num-speakers"
              type="number"
              min={1}
              max={20}
              placeholder="Auto-detect"
              value={numSpeakersInput}
              onChange={(e) => setNumSpeakersInput(e.target.value)}
              disabled={isRunning}
            />
            <p className="text-xs text-gray-500 mt-1">
              If you know how many people were on the call, set it here — usually
              improves accuracy. Leave blank to let pyannote decide.
            </p>
          </div>
        </div>

        <DialogFooter>
          <Button
            variant="outline"
            onClick={() => onOpenChange(false)}
            disabled={isRunning}
          >
            Cancel
          </Button>
          <Button onClick={handleRun} disabled={isRunning}>
            {isRunning ? (
              <>
                <Loader2 className="h-4 w-4 animate-spin mr-2" />
                Diarizing…
              </>
            ) : (
              'Run diarization'
            )}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
