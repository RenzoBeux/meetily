// Wraps sherpa-onnx OfflineSpeakerDiarization. Synchronous and blocking — call
// from `tokio::task::spawn_blocking`.

use anyhow::{anyhow, Result};
use log::{info, warn};
use sherpa_onnx::{
    FastClusteringConfig, OfflineSpeakerDiarization, OfflineSpeakerDiarizationConfig,
    OfflineSpeakerSegmentationModelConfig, OfflineSpeakerSegmentationPyannoteModelConfig,
    SpeakerEmbeddingExtractorConfig,
};

use crate::audio::decoder::decode_audio_file;
use crate::diarization::models::DiarizationModelPaths;

#[derive(Debug, Clone)]
pub struct DiarSegment {
    pub start: f64,
    pub end: f64,
    pub speaker: i32,
}

pub struct DiarizationEngine {
    inner: OfflineSpeakerDiarization,
    sample_rate: i32,
    provider: &'static str,
}

fn provider_str() -> &'static str {
    if cfg!(feature = "diarization-cuda") {
        "cuda"
    } else {
        "cpu"
    }
}

pub fn detect_gpu() -> bool {
    cfg!(feature = "diarization-cuda")
}

impl DiarizationEngine {
    /// `num_speakers`: when `Some(n)` with `n >= 1`, forces exactly `n` speaker
    /// clusters — the most reliable cure for over-/under-segmentation when the
    /// caller knows the head-count. When `None` (or `< 1`), the speaker count is
    /// inferred automatically via the clustering threshold.
    pub fn new(paths: &DiarizationModelPaths, num_speakers: Option<i32>) -> Result<Self> {
        let provider = provider_str();
        info!(
            "Initialising speaker diarization (provider={}, segmentation={}, embedding={})",
            provider,
            paths.segmentation.display(),
            paths.embedding.display()
        );

        // num_threads governs CPU parallelism; on GPU it's a no-op for the heavy
        // ops but still controls the pre/post-processing path. Pick a small
        // sensible default rather than the crate default (1) since diarization
        // happens after recording stops and can use the box.
        let num_threads = std::cmp::max(2, num_cpus_like()) as i32;

        // A supplied head-count forces exactly that many clusters (threshold
        // ignored); otherwise auto-detect with -1.
        let num_clusters = num_speakers.filter(|n| *n >= 1).unwrap_or(-1);
        if num_clusters >= 1 {
            info!("Diarization clustering: forced num_speakers={num_clusters}");
        } else {
            info!("Diarization clustering: auto (threshold=0.7)");
        }

        let config = OfflineSpeakerDiarizationConfig {
            segmentation: OfflineSpeakerSegmentationModelConfig {
                pyannote: OfflineSpeakerSegmentationPyannoteModelConfig {
                    model: Some(paths.segmentation.to_string_lossy().into_owned()),
                },
                num_threads,
                debug: false,
                provider: Some(provider.to_string()),
            },
            embedding: SpeakerEmbeddingExtractorConfig {
                model: Some(paths.embedding.to_string_lossy().into_owned()),
                num_threads,
                debug: false,
                provider: Some(provider.to_string()),
            },
            clustering: FastClusteringConfig {
                // -1 = auto (threshold decides); >= 1 = forced head-count.
                num_clusters,
                // HIGHER threshold merges more → FEWER speakers. sherpa-onnx's
                // default (0.5) over-segments real meetings (200+ "speakers");
                // 0.7 is the production value for AHC with wespeaker / 3D-Speaker
                // embeddings. Ignored when num_clusters >= 1.
                threshold: 0.7,
            },
            // Slightly more lenient minimums so a hesitant speaker doesn't get
            // chopped into many short bursts that each cluster separately.
            min_duration_on: 0.5,
            min_duration_off: 0.5,
        };

        let inner = OfflineSpeakerDiarization::create(&config)
            .ok_or_else(|| anyhow!("Failed to initialise sherpa-onnx OfflineSpeakerDiarization"))?;
        let sample_rate = inner.sample_rate();
        info!(
            "Speaker diarization ready (provider={}, sample_rate={}Hz)",
            provider, sample_rate
        );
        Ok(Self {
            inner,
            sample_rate,
            provider,
        })
    }

    pub fn provider(&self) -> &'static str {
        self.provider
    }

    /// Diarize a saved audio file. Decodes via symphonia, resamples to the
    /// model's expected sample rate (16 kHz for pyannote-3.0), then runs the
    /// pipeline and returns timestamped segments.
    pub fn run_on_file(&self, wav_path: &std::path::Path) -> Result<Vec<DiarSegment>> {
        self.run_on_file_excluding(wav_path, &[])
    }

    /// Like [`run_on_file`](Self::run_on_file) but zeroes out the given time
    /// ranges (in seconds) before clustering. We use this to silence the local
    /// microphone ("you") regions so the clusterer only ever sees the
    /// remote/"them" audio — that is what makes diarization split *them* into
    /// `speaker_1..N` while the mic stream stays anchored to "You" in the
    /// aligner.
    ///
    /// Tradeoff: during cross-talk (you and a remote speaker talking at once),
    /// masking the mic window also silences the overlapping remote speech in
    /// that window. That is acceptable — cross-talk clusters unreliably anyway,
    /// and the mic segment is still kept as "You" downstream.
    pub fn run_on_file_excluding(
        &self,
        wav_path: &std::path::Path,
        exclude_ranges: &[(f64, f64)],
    ) -> Result<Vec<DiarSegment>> {
        let decoded = decode_audio_file(wav_path)
            .map_err(|e| anyhow!("Failed to decode audio for diarization: {e}"))?;
        info!(
            "Diarization input: {} samples, {} Hz, {} ch, {:.2}s",
            decoded.samples.len(),
            decoded.sample_rate,
            decoded.channels,
            decoded.duration_seconds
        );
        // `to_whisper_format` already does mono + 16 kHz f32 in [-1, 1].
        let mut samples = decoded.to_whisper_format();
        if samples.is_empty() {
            warn!("Diarization input is empty — returning no segments");
            return Ok(Vec::new());
        }
        if self.sample_rate != 16_000 {
            return Err(anyhow!(
                "Diarization model expects {} Hz but pipeline produced 16 kHz",
                self.sample_rate
            ));
        }
        if !exclude_ranges.is_empty() {
            let total = samples.len() as i64;
            let mut masked_samples: usize = 0;
            for &(start, end) in exclude_ranges {
                if end <= start {
                    continue;
                }
                let from = ((start * 16_000.0).floor() as i64).clamp(0, total) as usize;
                let to = ((end * 16_000.0).ceil() as i64).clamp(0, total) as usize;
                if to > from {
                    samples[from..to].iter_mut().for_each(|s| *s = 0.0);
                    masked_samples += to - from;
                }
            }
            info!(
                "Masked {:.2}s of mic audio across {} range(s) before clustering",
                masked_samples as f64 / 16_000.0,
                exclude_ranges.len()
            );
        }
        let result = self
            .inner
            .process(&samples)
            .ok_or_else(|| anyhow!("OfflineSpeakerDiarization::process returned None"))?;
        let segments: Vec<DiarSegment> = result
            .sort_by_start_time()
            .into_iter()
            .map(|s| DiarSegment {
                start: s.start as f64,
                end: s.end as f64,
                speaker: s.speaker,
            })
            .collect();
        info!(
            "Diarization produced {} segments across {} speakers",
            segments.len(),
            segments
                .iter()
                .map(|s| s.speaker)
                .collect::<std::collections::BTreeSet<_>>()
                .len()
        );
        Ok(segments)
    }
}

fn num_cpus_like() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
}
