'use client';

import { useState } from 'react';
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover';
import { Input } from '@/components/ui/input';
import { Button } from '@/components/ui/button';
import { formatSpeaker } from '@/lib/speakerLabel';

const CANONICAL_SPEAKERS: Array<{ value: string; label: string }> = [
  { value: 'mic', label: 'You' },
  { value: 'system', label: 'Others' },
];

interface SpeakerPickerProps {
  knownSpeakers: string[];
  currentSpeaker?: string | null;
  trigger: React.ReactNode;
  /** Picked speaker. `null` clears the speaker tag entirely. */
  onPick: (value: string | null) => void;
  align?: 'start' | 'center' | 'end';
}

export function SpeakerPicker({
  knownSpeakers,
  currentSpeaker,
  trigger,
  onPick,
  align = 'start',
}: SpeakerPickerProps) {
  const [open, setOpen] = useState(false);
  const [custom, setCustom] = useState('');

  const choose = (value: string | null) => {
    onPick(value);
    setCustom('');
    setOpen(false);
  };

  const otherKnown = knownSpeakers.filter(
    (s) => s !== 'mic' && s !== 'system',
  );

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>{trigger}</PopoverTrigger>
      <PopoverContent align={align} className="w-56 p-2">
        <div className="text-xs font-medium text-gray-500 px-1 pb-1">Set speaker</div>

        <div className="flex flex-col gap-1">
          {CANONICAL_SPEAKERS.map((s) => {
            const label = formatSpeaker(s.value);
            const isCurrent = currentSpeaker === s.value;
            return (
              <button
                key={s.value}
                type="button"
                onClick={() => choose(s.value)}
                className={`flex items-center justify-between text-left px-2 py-1 rounded text-sm hover:bg-gray-100 ${isCurrent ? 'bg-gray-100 font-semibold' : ''}`}
              >
                <span className="flex items-center gap-2">
                  {label && (
                    <span className={`text-xs px-1.5 py-0.5 rounded ${label.className}`}>
                      {label.label}
                    </span>
                  )}
                  <span className="text-gray-500 text-xs">{s.value}</span>
                </span>
                {isCurrent && <span className="text-xs text-gray-500">current</span>}
              </button>
            );
          })}

          {otherKnown.length > 0 && (
            <>
              <div className="text-xs font-medium text-gray-500 px-1 pt-2 pb-1">
                In this meeting
              </div>
              {otherKnown.map((s) => {
                const label = formatSpeaker(s);
                const isCurrent = currentSpeaker === s;
                return (
                  <button
                    key={s}
                    type="button"
                    onClick={() => choose(s)}
                    className={`flex items-center justify-between text-left px-2 py-1 rounded text-sm hover:bg-gray-100 ${isCurrent ? 'bg-gray-100 font-semibold' : ''}`}
                  >
                    <span className="flex items-center gap-2">
                      {label && (
                        <span className={`text-xs px-1.5 py-0.5 rounded ${label.className}`}>
                          {label.label}
                        </span>
                      )}
                    </span>
                    {isCurrent && <span className="text-xs text-gray-500">current</span>}
                  </button>
                );
              })}
            </>
          )}
        </div>

        <div className="border-t border-gray-200 mt-2 pt-2">
          <div className="text-xs font-medium text-gray-500 px-1 pb-1">Custom label</div>
          <form
            onSubmit={(e) => {
              e.preventDefault();
              const trimmed = custom.trim();
              if (trimmed) choose(trimmed);
            }}
            className="flex gap-1"
          >
            <Input
              value={custom}
              onChange={(e) => setCustom(e.target.value)}
              placeholder="e.g. Alice"
              className="h-7 text-sm"
              autoFocus={false}
            />
            <Button type="submit" size="sm" variant="outline" disabled={!custom.trim()}>
              Set
            </Button>
          </form>
        </div>

        {currentSpeaker && (
          <div className="border-t border-gray-200 mt-2 pt-2">
            <button
              type="button"
              onClick={() => choose(null)}
              className="text-xs text-gray-500 hover:text-gray-900 hover:underline"
            >
              Clear speaker
            </button>
          </div>
        )}
      </PopoverContent>
    </Popover>
  );
}
