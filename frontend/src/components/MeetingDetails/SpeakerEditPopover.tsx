'use client';

import { useMemo, useState } from 'react';
import { toast } from 'sonner';
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { formatSpeaker } from '@/lib/speakerLabel';
import {
  renameSpeakerInMeeting,
  updateTranscriptSpeaker,
} from '@/services/diarizationService';

interface SpeakerEditPopoverProps {
  /** ID of the transcript segment being edited */
  transcriptId: string;
  /** ID of the meeting that owns the segment */
  meetingId: string;
  /** Current speaker tag on this segment */
  currentSpeaker: string | undefined;
  /** Distinct speaker tags present in this meeting (used to populate "reassign to") */
  knownSpeakers: string[];
  /** Called after any mutation so parent can refetch transcripts */
  onChanged: () => Promise<void> | void;
  /** The pill that triggers the popover */
  children: React.ReactNode;
}

export function SpeakerEditPopover({
  transcriptId,
  meetingId,
  currentSpeaker,
  knownSpeakers,
  onChanged,
  children,
}: SpeakerEditPopoverProps) {
  const [open, setOpen] = useState(false);
  const [renameValue, setRenameValue] = useState('');
  const [newSpeakerValue, setNewSpeakerValue] = useState('');
  const [isBusy, setIsBusy] = useState(false);

  const otherSpeakers = useMemo(
    () => knownSpeakers.filter((s) => s && s !== currentSpeaker),
    [knownSpeakers, currentSpeaker],
  );

  const handleReassign = async (speaker: string) => {
    setIsBusy(true);
    try {
      await updateTranscriptSpeaker(transcriptId, speaker);
      toast.success(`Reassigned to ${formatSpeaker(speaker)?.label ?? speaker}`);
      setOpen(false);
      await onChanged();
    } catch (e) {
      toast.error('Reassign failed', {
        description: e instanceof Error ? e.message : String(e),
      });
    } finally {
      setIsBusy(false);
    }
  };

  const handleAssignNew = async () => {
    const value = newSpeakerValue.trim();
    if (!value) return;
    await handleReassign(value);
    setNewSpeakerValue('');
  };

  const handleRename = async () => {
    const value = renameValue.trim();
    if (!value || !currentSpeaker) return;
    setIsBusy(true);
    try {
      const result = await renameSpeakerInMeeting(meetingId, currentSpeaker, value);
      toast.success(`Renamed across ${result.updated} segment${result.updated === 1 ? '' : 's'}`);
      setOpen(false);
      setRenameValue('');
      await onChanged();
    } catch (e) {
      toast.error('Rename failed', {
        description: e instanceof Error ? e.message : String(e),
      });
    } finally {
      setIsBusy(false);
    }
  };

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>{children}</PopoverTrigger>
      <PopoverContent className="w-64 p-3 space-y-3" align="start">
        {currentSpeaker && (
          <div className="space-y-1">
            <label className="text-xs font-medium text-gray-700">
              Rename "{formatSpeaker(currentSpeaker)?.label ?? currentSpeaker}" everywhere in this meeting
            </label>
            <div className="flex gap-1">
              <Input
                placeholder="Display name"
                value={renameValue}
                onChange={(e) => setRenameValue(e.target.value)}
                onKeyDown={(e) => e.key === 'Enter' && handleRename()}
                disabled={isBusy}
                className="h-7 text-xs"
              />
              <Button
                size="sm"
                onClick={handleRename}
                disabled={isBusy || !renameValue.trim()}
                className="h-7 px-2 text-xs"
              >
                Rename
              </Button>
            </div>
          </div>
        )}

        {otherSpeakers.length > 0 && (
          <div className="space-y-1">
            <label className="text-xs font-medium text-gray-700">
              Reassign just this segment to
            </label>
            <div className="flex flex-wrap gap-1">
              {otherSpeakers.map((s) => {
                const label = formatSpeaker(s);
                return (
                  <button
                    key={s}
                    onClick={() => handleReassign(s)}
                    disabled={isBusy}
                    className={`text-xs px-2 py-1 rounded ${label?.className ?? 'bg-gray-100 text-gray-700'} hover:opacity-80 disabled:opacity-40`}
                  >
                    {label?.label ?? s}
                  </button>
                );
              })}
            </div>
          </div>
        )}

        <div className="space-y-1">
          <label className="text-xs font-medium text-gray-700">
            …or to a new speaker
          </label>
          <div className="flex gap-1">
            <Input
              placeholder="New speaker name"
              value={newSpeakerValue}
              onChange={(e) => setNewSpeakerValue(e.target.value)}
              onKeyDown={(e) => e.key === 'Enter' && handleAssignNew()}
              disabled={isBusy}
              className="h-7 text-xs"
            />
            <Button
              size="sm"
              variant="outline"
              onClick={handleAssignNew}
              disabled={isBusy || !newSpeakerValue.trim()}
              className="h-7 px-2 text-xs"
            >
              Assign
            </Button>
          </div>
        </div>
      </PopoverContent>
    </Popover>
  );
}
