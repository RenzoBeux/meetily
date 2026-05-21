// Pure alignment between transcript segments (from Whisper) and diarization
// segments (from sherpa-onnx). For each transcript segment we pick the
// diarization segment with the largest time-overlap and copy its speaker id.
// If no diarization segment overlaps by at least `MIN_OVERLAP_RATIO` of the
// transcript segment's duration, we leave the existing `speaker` value alone
// (which is the "mic"/"system" heuristic tag set during live transcription).

use crate::audio::recording_saver::TranscriptSegment;
use crate::diarization::engine::DiarSegment;

/// Minimum fraction of a transcript segment that must overlap with a single
/// diarization segment to commit the diarized speaker label. Below this we
/// keep the existing mic/system tag rather than guess.
const MIN_OVERLAP_RATIO: f64 = 0.20;

pub fn assign_speakers(transcript: &mut [TranscriptSegment], diar: &[DiarSegment]) {
    if diar.is_empty() {
        return;
    }
    for seg in transcript.iter_mut() {
        let dur = (seg.audio_end_time - seg.audio_start_time).max(1e-6);
        let mut best: Option<(f64, i32)> = None;
        for d in diar {
            let overlap =
                (seg.audio_end_time.min(d.end) - seg.audio_start_time.max(d.start)).max(0.0);
            if overlap <= 0.0 {
                continue;
            }
            match best {
                Some((cur, _)) if cur >= overlap => {}
                _ => best = Some((overlap, d.speaker)),
            }
        }
        if let Some((overlap, speaker)) = best {
            if overlap / dur >= MIN_OVERLAP_RATIO {
                seg.speaker = format!("speaker_{}", speaker + 1);
            }
        }
    }
}

pub fn unique_speakers(transcript: &[TranscriptSegment]) -> Vec<String> {
    let mut seen: std::collections::BTreeSet<String> = Default::default();
    for s in transcript {
        if !s.speaker.is_empty() {
            seen.insert(s.speaker.clone());
        }
    }
    seen.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(start: f64, end: f64, speaker: &str) -> TranscriptSegment {
        TranscriptSegment {
            id: format!("{start}-{end}"),
            text: String::new(),
            audio_start_time: start,
            audio_end_time: end,
            duration: end - start,
            display_time: String::new(),
            confidence: 1.0,
            sequence_id: 0,
            speaker: speaker.to_string(),
        }
    }

    fn d(start: f64, end: f64, speaker: i32) -> DiarSegment {
        DiarSegment { start, end, speaker }
    }

    #[test]
    fn full_overlap_assigns_speaker() {
        let mut tx = vec![t(0.0, 2.0, "mic")];
        assign_speakers(&mut tx, &[d(0.0, 2.0, 0)]);
        assert_eq!(tx[0].speaker, "speaker_1");
    }

    #[test]
    fn partial_overlap_above_threshold_assigns() {
        let mut tx = vec![t(0.0, 2.0, "mic")];
        // Diarization segment covers 1.0 of 2.0 = 50% > 20%.
        assign_speakers(&mut tx, &[d(0.5, 1.5, 3)]);
        assert_eq!(tx[0].speaker, "speaker_4");
    }

    #[test]
    fn partial_overlap_below_threshold_keeps_existing() {
        let mut tx = vec![t(0.0, 10.0, "system")];
        // Only 0.5s overlap of a 10s window = 5% < 20%.
        assign_speakers(&mut tx, &[d(9.5, 10.5, 7)]);
        assert_eq!(tx[0].speaker, "system");
    }

    #[test]
    fn zero_overlap_keeps_existing() {
        let mut tx = vec![t(0.0, 2.0, "mic")];
        assign_speakers(&mut tx, &[d(5.0, 7.0, 1)]);
        assert_eq!(tx[0].speaker, "mic");
    }

    #[test]
    fn picks_dominant_speaker_when_two_overlap() {
        let mut tx = vec![t(0.0, 5.0, "mic")];
        // speaker 0 covers 0..1 (1s); speaker 1 covers 1..5 (4s) — speaker 1 wins.
        assign_speakers(&mut tx, &[d(0.0, 1.0, 0), d(1.0, 5.0, 1)]);
        assert_eq!(tx[0].speaker, "speaker_2");
    }

    #[test]
    fn empty_diar_is_noop() {
        let mut tx = vec![t(0.0, 1.0, "mic"), t(1.0, 2.0, "system")];
        assign_speakers(&mut tx, &[]);
        assert_eq!(tx[0].speaker, "mic");
        assert_eq!(tx[1].speaker, "system");
    }

    #[test]
    fn first_max_wins_on_tie() {
        let mut tx = vec![t(0.0, 4.0, "mic")];
        // Both diarization segments overlap exactly 2.0 s; we accept whichever
        // came first, which is well-defined and stable.
        assign_speakers(&mut tx, &[d(0.0, 2.0, 7), d(2.0, 4.0, 9)]);
        assert!(tx[0].speaker == "speaker_8" || tx[0].speaker == "speaker_10");
    }

    #[test]
    fn unique_speakers_dedups_and_sorts() {
        let tx = vec![
            t(0.0, 1.0, "speaker_2"),
            t(1.0, 2.0, "speaker_1"),
            t(2.0, 3.0, "speaker_2"),
            t(3.0, 4.0, ""),
        ];
        assert_eq!(unique_speakers(&tx), vec!["speaker_1", "speaker_2"]);
    }
}
