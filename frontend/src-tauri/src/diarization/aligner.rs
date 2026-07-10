// Pure alignment between transcript segments (from Whisper) and diarization
// segments (from sherpa-onnx). For each transcript segment we pick the
// diarization segment with the largest time-overlap and copy its speaker id.
// If no diarization segment overlaps by at least `MIN_OVERLAP_RATIO` of the
// transcript segment's duration, we leave the existing `speaker` value alone
// (which is the "mic"/"system" heuristic tag set during live transcription).
//
// The microphone stream is the local user ("You"). It is captured as a
// dedicated source, so its tag is ground truth — never a guess. We therefore
// NEVER reassign a `mic` segment to a clustered speaker; clustering only ever
// splits the "them"/system audio into `speaker_1..N`.

use std::collections::HashMap;

use crate::audio::recording_saver::TranscriptSegment;
use crate::diarization::engine::DiarSegment;

/// Minimum fraction of a transcript segment that must overlap with a single
/// diarization segment to commit the diarized speaker label. Below this we
/// keep the existing mic/system tag rather than guess.
const MIN_OVERLAP_RATIO: f64 = 0.20;

/// Source tag for the local microphone stream — the user. Locked to "You" in
/// the UI; diarization must never overwrite it.
const MIC_SPEAKER: &str = "mic";

/// Returns the number of distinct diarization clusters that were actually
/// assigned to at least one transcript segment. Comparing this against the
/// user-requested speaker count tells whether a cluster went entirely
/// unmatched (all of its speech fell below the overlap threshold).
pub fn assign_speakers(transcript: &mut [TranscriptSegment], diar: &[DiarSegment]) -> usize {
    if diar.is_empty() {
        return 0;
    }

    // Pass 1: for each eligible (non-mic) segment, resolve the dominant
    // overlapping diarization cluster id, or `None` if it stays as-is (a mic
    // segment, or a system segment with no overlap above the threshold).
    let raw_ids: Vec<Option<i32>> = transcript
        .iter()
        .map(|seg| {
            if seg.speaker == MIC_SPEAKER {
                return None;
            }
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
            best.and_then(|(overlap, speaker)| (overlap / dur >= MIN_OVERLAP_RATIO).then_some(speaker))
        })
        .collect();

    // Pass 2: remap the raw cluster ids that actually landed on a segment to
    // contiguous `speaker_1..N` labels, ordered by first appearance, so the
    // user sees gap-free numbering regardless of the clusterer's internal ids
    // (which skip the cluster(s) that fell on masked-out mic audio).
    let mut remap: HashMap<i32, usize> = HashMap::new();
    let mut next: usize = 1;
    for (seg, raw) in transcript.iter_mut().zip(raw_ids) {
        let Some(id) = raw else { continue };
        let label = *remap.entry(id).or_insert_with(|| {
            let n = next;
            next += 1;
            n
        });
        seg.speaker = format!("speaker_{}", label);
    }
    next - 1
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
    fn mic_is_never_reassigned_even_on_full_overlap() {
        // The mic stream is the local user; clustering must never relabel it.
        let mut tx = vec![t(0.0, 2.0, "mic")];
        assign_speakers(&mut tx, &[d(0.0, 2.0, 0)]);
        assert_eq!(tx[0].speaker, "mic");
    }

    #[test]
    fn system_full_overlap_assigns_speaker() {
        let mut tx = vec![t(0.0, 2.0, "system")];
        assign_speakers(&mut tx, &[d(0.0, 2.0, 0)]);
        assert_eq!(tx[0].speaker, "speaker_1");
    }

    #[test]
    fn partial_overlap_above_threshold_assigns() {
        let mut tx = vec![t(0.0, 2.0, "system")];
        // Diarization segment covers 1.0 of 2.0 = 50% > 20%. Raw cluster id 3
        // is the first (and only) assigned id, so it remaps to speaker_1.
        assign_speakers(&mut tx, &[d(0.5, 1.5, 3)]);
        assert_eq!(tx[0].speaker, "speaker_1");
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
        let mut tx = vec![t(0.0, 2.0, "system")];
        assign_speakers(&mut tx, &[d(5.0, 7.0, 1)]);
        assert_eq!(tx[0].speaker, "system");
    }

    #[test]
    fn picks_dominant_speaker_when_two_overlap() {
        let mut tx = vec![t(0.0, 5.0, "system")];
        // speaker 0 covers 0..1 (1s); speaker 1 covers 1..5 (4s) — speaker 1
        // wins; as the only assigned cluster it remaps to speaker_1.
        assign_speakers(&mut tx, &[d(0.0, 1.0, 0), d(1.0, 5.0, 1)]);
        assert_eq!(tx[0].speaker, "speaker_1");
    }

    #[test]
    fn empty_diar_is_noop() {
        let mut tx = vec![t(0.0, 1.0, "mic"), t(1.0, 2.0, "system")];
        assign_speakers(&mut tx, &[]);
        assert_eq!(tx[0].speaker, "mic");
        assert_eq!(tx[1].speaker, "system");
    }

    #[test]
    fn mic_preserved_while_system_is_diarized() {
        let mut tx = vec![
            t(0.0, 2.0, "mic"),
            t(2.0, 4.0, "system"),
            t(4.0, 6.0, "system"),
        ];
        // Clusters 3 and 8 land on the two system segments; mic is untouched.
        assign_speakers(&mut tx, &[d(0.0, 2.0, 1), d(2.0, 4.0, 3), d(4.0, 6.0, 8)]);
        assert_eq!(tx[0].speaker, "mic");
        assert_eq!(tx[1].speaker, "speaker_1");
        assert_eq!(tx[2].speaker, "speaker_2");
    }

    #[test]
    fn numbering_is_contiguous_by_first_appearance() {
        let mut tx = vec![
            t(0.0, 1.0, "system"),
            t(1.0, 2.0, "system"),
            t(2.0, 3.0, "system"),
        ];
        // Raw cluster ids [5, 2, 5] → remap 5->1, 2->2 by first appearance.
        assign_speakers(&mut tx, &[d(0.0, 1.0, 5), d(1.0, 2.0, 2), d(2.0, 3.0, 5)]);
        assert_eq!(tx[0].speaker, "speaker_1");
        assert_eq!(tx[1].speaker, "speaker_2");
        assert_eq!(tx[2].speaker, "speaker_1");
    }

    #[test]
    fn returns_count_of_clusters_that_landed() {
        let mut tx = vec![t(0.0, 2.0, "system"), t(2.0, 4.0, "system")];
        // Clusters 1 and 2 land on rows; cluster 9 overlaps nothing.
        let n = assign_speakers(&mut tx, &[d(0.0, 2.0, 1), d(2.0, 4.0, 2), d(10.0, 12.0, 9)]);
        assert_eq!(n, 2);
        // No cluster landing at all reports zero.
        let mut tx2 = vec![t(0.0, 10.0, "system")];
        assert_eq!(assign_speakers(&mut tx2, &[d(9.9, 10.0, 1)]), 0);
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
