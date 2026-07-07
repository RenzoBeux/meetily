/**
 * Map a source-faithful speaker tag (as written by the audio pipeline) to a
 * user-facing label and a Tailwind class for tinting it.
 *
 * The pipeline emits "mic" or "system" based on which audio stream produced the
 * speech segment. The mic stream is the local user and stays anchored to "You";
 * diarization only ever splits the system/"them" audio into per-speaker IDs
 * (e.g. "speaker_1"), which we render as "Speaker 1". Any other custom label
 * (e.g. a name the user typed) renders as-is.
 */
export interface SpeakerLabel {
  label: string;
  className: string;
}

export function formatSpeaker(tag: string | undefined | null): SpeakerLabel | null {
  if (!tag) return null;
  switch (tag) {
    case 'mic':
      return { label: 'You', className: 'bg-blue-100 text-blue-700' };
    case 'system':
      return { label: 'Others', className: 'bg-purple-100 text-purple-700' };
    default: {
      // Diarized remote speakers arrive as "speaker_1", "speaker_2", … →
      // "Speaker 1", etc. Anything else is a user-supplied label, shown as-is.
      const match = /^speaker_(\d+)$/.exec(tag);
      if (match) {
        return { label: `Speaker ${match[1]}`, className: 'bg-amber-100 text-amber-700' };
      }
      return { label: tag, className: 'bg-gray-100 text-gray-700' };
    }
  }
}

/**
 * Plain display name for a speaker tag — same mapping as formatSpeaker but
 * returns just the string. Used for prompt construction (summary, chat) where
 * we want the LLM to see the same labels the user sees in the UI.
 */
export function speakerDisplayName(tag: string | undefined | null): string | null {
  return formatSpeaker(tag)?.label ?? null;
}
