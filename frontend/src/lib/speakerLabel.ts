/**
 * Map a source-faithful speaker tag (as written by the audio pipeline) to a
 * user-facing label and a Tailwind class for tinting it.
 *
 * The pipeline emits "mic" or "system" based on which audio stream produced the
 * speech segment. The mic stream is the local user and stays anchored to "You";
 * diarization only ever splits the system/"them" audio into per-speaker IDs
 * (e.g. "speaker_1"), which we render as "Speaker 1". Any other custom label
 * (e.g. a name the user typed) renders as-is.
 *
 * Colors come from the theme's speaker palette (globals.css): violet is
 * reserved for "You", cyan for the undiarized "Others" bucket, and diarized /
 * custom speakers cycle through six hue-stable slots that read on both themes.
 */
export interface SpeakerLabel {
  label: string;
  className: string;
}

// Literal class strings so Tailwind's scanner generates them (no dynamic names).
const SPEAKER_CYCLE = [
  'bg-speaker-1/15 text-speaker-1',
  'bg-speaker-2/15 text-speaker-2',
  'bg-speaker-3/15 text-speaker-3',
  'bg-speaker-4/15 text-speaker-4',
  'bg-speaker-5/15 text-speaker-5',
  'bg-speaker-6/15 text-speaker-6',
] as const;

const YOU_CLASS = 'bg-speaker-you/15 text-speaker-you';
const OTHERS_CLASS = 'bg-speaker-others/15 text-speaker-others';

/** Deterministic hash so a custom speaker name keeps its color across renders/sessions. */
function hashLabel(label: string): number {
  let hash = 0;
  for (let i = 0; i < label.length; i++) {
    hash = (hash * 31 + label.charCodeAt(i)) | 0;
  }
  return Math.abs(hash);
}

export function formatSpeaker(tag: string | undefined | null): SpeakerLabel | null {
  if (!tag) return null;
  switch (tag) {
    case 'mic':
      return { label: 'You', className: YOU_CLASS };
    case 'system':
      return { label: 'Others', className: OTHERS_CLASS };
    default: {
      // Diarized remote speakers arrive as "speaker_1", "speaker_2", … →
      // "Speaker 1", etc. Anything else is a user-supplied label, shown as-is
      // with a color derived from its name so it stays stable.
      const match = /^speaker_(\d+)$/.exec(tag);
      if (match) {
        const n = parseInt(match[1], 10);
        const slot = ((n - 1) % SPEAKER_CYCLE.length + SPEAKER_CYCLE.length) % SPEAKER_CYCLE.length;
        return { label: `Speaker ${match[1]}`, className: SPEAKER_CYCLE[slot] };
      }
      return { label: tag, className: SPEAKER_CYCLE[hashLabel(tag) % SPEAKER_CYCLE.length] };
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
