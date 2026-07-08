// Remote (cloud) speaker diarization via the pyannoteAI API.
//
// Flow: encode masked mono 16 kHz audio as PCM16 WAV → presigned upload to
// pyannote temporary media storage (auto-deleted ≤48 h) → submit a diarization
// job with the "precision-2" model → poll until the job finishes → map the
// returned segments into the same `DiarSegment` shape the local sherpa-onnx
// engine produces, so the aligner and DB update code downstream are shared.

use std::collections::HashMap;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use log::info;

use crate::diarization::engine::DiarSegment;

const API_BASE: &str = "https://api.pyannote.ai/v1";
const MODEL: &str = "precision-2";
const POLL_INTERVAL: Duration = Duration::from_secs(5);
const JOB_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const MAX_CONSECUTIVE_POLL_ERRORS: u32 = 3;

/// Encode mono f32 samples in [-1, 1] as a 16-bit PCM WAV file (44-byte RIFF
/// header + little-endian samples). Written manually — the project
/// deliberately dropped the `hound` dependency.
pub fn encode_wav_pcm16_mono(samples: &[f32], sample_rate: u32) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);

    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVE");

    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&(sample_rate * 2).to_le_bytes()); // byte rate
    out.extend_from_slice(&2u16.to_le_bytes()); // block align
    out.extend_from_slice(&16u16.to_le_bytes()); // bits per sample

    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for &s in samples {
        let v = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

#[derive(Debug, serde::Deserialize)]
struct MediaInputResponse {
    url: String,
}

#[derive(Debug, serde::Deserialize)]
struct JobCreated {
    #[serde(rename = "jobId")]
    job_id: String,
}

#[derive(Debug, serde::Deserialize)]
struct JobStatus {
    status: String,
    output: Option<JobOutput>,
    error: Option<serde_json::Value>,
}

#[derive(Debug, serde::Deserialize)]
struct JobOutput {
    diarization: Vec<RemoteSegment>,
}

#[derive(Debug, serde::Deserialize)]
pub struct RemoteSegment {
    pub speaker: String,
    pub start: f64,
    pub end: f64,
}

#[derive(Debug, serde::Serialize)]
struct DiarizeRequest<'a> {
    url: &'a str,
    model: &'a str,
    // Omit entirely when unset — a literal `"numSpeakers": null` may be rejected.
    #[serde(rename = "numSpeakers", skip_serializing_if = "Option::is_none")]
    num_speakers: Option<u32>,
}

/// Map pyannote segments ("SPEAKER_00", "SPEAKER_07", …) to the integer
/// cluster ids the aligner expects. Labels with a trailing integer keep it;
/// arbitrary labels get a stable first-appearance index. Output is sorted by
/// start time (the aligner assumes sorted segments, like sherpa's
/// `sort_by_start_time`).
pub fn map_remote_segments(remote: &[RemoteSegment]) -> Vec<DiarSegment> {
    let mut fallback: HashMap<&str, i32> = HashMap::new();
    let mut segments: Vec<DiarSegment> = remote
        .iter()
        .map(|seg| {
            let parsed = seg
                .speaker
                .rsplit('_')
                .next()
                .and_then(|tail| tail.parse::<i32>().ok());
            let speaker = parsed.unwrap_or_else(|| {
                let next = fallback.len() as i32;
                *fallback.entry(seg.speaker.as_str()).or_insert(next)
            });
            DiarSegment {
                start: seg.start,
                end: seg.end,
                speaker,
            }
        })
        .collect();
    segments.sort_by(|a, b| a.start.total_cmp(&b.start));
    segments
}

pub struct PyannoteClient {
    http: reqwest::Client,
    api_key: String,
}

impl PyannoteClient {
    pub fn new(api_key: String) -> Result<Self> {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .build()
            .context("Failed to build HTTP client")?;
        Ok(Self { http, api_key })
    }

    fn bearer(&self) -> String {
        format!("Bearer {}", self.api_key)
    }

    /// Full pipeline: presign → upload → submit job → poll until done.
    /// `on_progress` is called with coarse stages ("uploading", "processing")
    /// so the caller can emit UI events.
    pub async fn diarize(
        &self,
        wav_bytes: Vec<u8>,
        object_key: &str,
        num_speakers: Option<u32>,
        on_progress: &(dyn Fn(&str) + Send + Sync),
    ) -> Result<Vec<DiarSegment>> {
        let media_url = format!("media://{object_key}");

        let presigned = self.create_presigned_upload(&media_url).await?;

        on_progress("uploading");
        self.upload(&presigned, wav_bytes).await?;

        let job_id = self.submit_job(&media_url, num_speakers).await?;
        info!("pyannoteAI diarization job submitted: {job_id}");

        on_progress("processing");
        let remote = tokio::time::timeout(JOB_TIMEOUT, self.poll_job(&job_id))
            .await
            .map_err(|_| {
                anyhow!(
                    "pyannoteAI job {job_id} did not finish within {} minutes",
                    JOB_TIMEOUT.as_secs() / 60
                )
            })??;

        Ok(map_remote_segments(&remote))
    }

    async fn create_presigned_upload(&self, media_url: &str) -> Result<String> {
        let response = self
            .http
            .post(format!("{API_BASE}/media/input"))
            .header("Authorization", self.bearer())
            .json(&serde_json::json!({ "url": media_url }))
            .timeout(Duration::from_secs(30))
            .send()
            .await
            .context("Failed to reach pyannote.ai for upload URL")?;

        let response = Self::check_status(response, "create upload URL").await?;
        let parsed: MediaInputResponse = response
            .json()
            .await
            .context("Unexpected response creating pyannote.ai upload URL")?;
        Ok(parsed.url)
    }

    async fn upload(&self, presigned_url: &str, wav_bytes: Vec<u8>) -> Result<()> {
        // Scale the timeout with payload size (~115 MB per meeting hour at
        // 16 kHz mono PCM16); floor of 2 minutes, ~100 kB/s worst case.
        let timeout = Duration::from_secs(std::cmp::max(120, wav_bytes.len() as u64 / 100_000));
        let size_mb = wav_bytes.len() as f64 / (1024.0 * 1024.0);
        info!("Uploading {size_mb:.1} MB WAV to pyannote.ai (timeout {}s)", timeout.as_secs());

        let mut last_err: Option<anyhow::Error> = None;
        for attempt in 0..2 {
            let result = self
                .http
                .put(presigned_url)
                .header("Content-Type", "audio/wav")
                .body(wav_bytes.clone())
                .timeout(timeout)
                .send()
                .await;

            match result {
                Ok(response) if response.status().is_success() => return Ok(()),
                Ok(response) => {
                    let status = response.status();
                    let body = response.text().await.unwrap_or_default();
                    bail!("Audio upload failed with status {status}: {body}");
                }
                // Retry once on transient network errors only.
                Err(e) if (e.is_timeout() || e.is_connect()) && attempt == 0 => {
                    info!("Upload attempt failed ({e}); retrying once");
                    last_err = Some(anyhow!(e));
                }
                Err(e) => return Err(anyhow!(e).context("Audio upload to pyannote.ai failed")),
            }
        }
        Err(last_err
            .unwrap_or_else(|| anyhow!("Audio upload failed"))
            .context("Audio upload to pyannote.ai failed after retry"))
    }

    async fn submit_job(&self, media_url: &str, num_speakers: Option<u32>) -> Result<String> {
        let request = DiarizeRequest {
            url: media_url,
            model: MODEL,
            num_speakers,
        };
        let response = self
            .http
            .post(format!("{API_BASE}/diarize"))
            .header("Authorization", self.bearer())
            .json(&request)
            .timeout(Duration::from_secs(30))
            .send()
            .await
            .context("Failed to submit pyannote.ai diarization job")?;

        let response = Self::check_status(response, "submit diarization job").await?;
        let created: JobCreated = response
            .json()
            .await
            .context("Unexpected response submitting pyannote.ai job")?;
        Ok(created.job_id)
    }

    async fn poll_job(&self, job_id: &str) -> Result<Vec<RemoteSegment>> {
        let mut consecutive_errors: u32 = 0;
        loop {
            tokio::time::sleep(POLL_INTERVAL).await;

            let response = match self
                .http
                .get(format!("{API_BASE}/jobs/{job_id}"))
                .header("Authorization", self.bearer())
                .timeout(Duration::from_secs(30))
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    consecutive_errors += 1;
                    if consecutive_errors > MAX_CONSECUTIVE_POLL_ERRORS {
                        return Err(anyhow!(e).context("Lost connection while polling pyannote.ai job"));
                    }
                    continue;
                }
            };

            let job: JobStatus = match Self::check_status(response, "poll job").await {
                Ok(r) => r.json().await.context("Unexpected pyannote.ai job response")?,
                Err(e) => {
                    consecutive_errors += 1;
                    if consecutive_errors > MAX_CONSECUTIVE_POLL_ERRORS {
                        return Err(e);
                    }
                    continue;
                }
            };
            consecutive_errors = 0;

            match job.status.as_str() {
                "succeeded" => {
                    let output = job.output.ok_or_else(|| {
                        anyhow!("pyannote.ai job succeeded but returned no diarization output")
                    })?;
                    info!(
                        "pyannoteAI job {job_id} succeeded with {} segments",
                        output.diarization.len()
                    );
                    return Ok(output.diarization);
                }
                "failed" | "canceled" => {
                    let detail = job
                        .error
                        .map(|e| e.to_string())
                        .unwrap_or_else(|| "no error detail provided".to_string());
                    bail!("pyannote.ai job {}: {}", job.status, detail);
                }
                // pending | created | running → keep polling.
                _ => {}
            }
        }
    }

    /// Convert non-success HTTP statuses into user-meaningful errors.
    async fn check_status(
        response: reqwest::Response,
        action: &str,
    ) -> Result<reqwest::Response> {
        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }
        let body = response.text().await.unwrap_or_default();
        match status.as_u16() {
            401 | 403 => bail!("Invalid pyannoteAI API key (check Settings → Transcript)"),
            402 => bail!("pyannoteAI quota exhausted — check your plan at pyannote.ai"),
            429 => bail!("pyannoteAI rate limit hit — try again in a moment"),
            _ => bail!("pyannote.ai {action} failed with status {status}: {body}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- WAV encoder -------------------------------------------------------

    #[test]
    fn wav_header_for_empty_input_is_44_bytes() {
        let wav = encode_wav_pcm16_mono(&[], 16_000);
        assert_eq!(wav.len(), 44);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(&wav[12..16], b"fmt ");
        assert_eq!(&wav[36..40], b"data");
        assert_eq!(u32::from_le_bytes(wav[4..8].try_into().unwrap()), 36);
        assert_eq!(u32::from_le_bytes(wav[40..44].try_into().unwrap()), 0);
    }

    #[test]
    fn wav_lengths_and_format_fields_are_correct() {
        let samples = vec![0.0_f32; 100];
        let wav = encode_wav_pcm16_mono(&samples, 16_000);

        assert_eq!(wav.len(), 44 + 200);
        assert_eq!(u32::from_le_bytes(wav[4..8].try_into().unwrap()), 36 + 200);
        assert_eq!(u32::from_le_bytes(wav[40..44].try_into().unwrap()), 200);
        // PCM, mono, 16 kHz, byte rate 32000, block align 2, 16 bits
        assert_eq!(u16::from_le_bytes(wav[20..22].try_into().unwrap()), 1);
        assert_eq!(u16::from_le_bytes(wav[22..24].try_into().unwrap()), 1);
        assert_eq!(u32::from_le_bytes(wav[24..28].try_into().unwrap()), 16_000);
        assert_eq!(u32::from_le_bytes(wav[28..32].try_into().unwrap()), 32_000);
        assert_eq!(u16::from_le_bytes(wav[32..34].try_into().unwrap()), 2);
        assert_eq!(u16::from_le_bytes(wav[34..36].try_into().unwrap()), 16);
    }

    #[test]
    fn wav_samples_round_trip_and_clamp() {
        let wav = encode_wav_pcm16_mono(&[1.0, -1.0, 0.0, 2.0, -2.0], 16_000);
        let read = |i: usize| i16::from_le_bytes(wav[44 + i * 2..46 + i * 2].try_into().unwrap());

        assert_eq!(read(0), 32767);
        assert_eq!(read(1), -32767);
        assert_eq!(read(2), 0);
        assert_eq!(read(3), 32767); // clamped
        assert_eq!(read(4), -32767); // clamped
    }

    // --- Segment mapping ----------------------------------------------------

    fn seg(speaker: &str, start: f64, end: f64) -> RemoteSegment {
        RemoteSegment {
            speaker: speaker.to_string(),
            start,
            end,
        }
    }

    #[test]
    fn maps_speaker_nn_labels_to_their_integer() {
        let out = map_remote_segments(&[seg("SPEAKER_00", 0.0, 1.0), seg("SPEAKER_07", 1.0, 2.0)]);
        assert_eq!(out[0].speaker, 0);
        assert_eq!(out[1].speaker, 7);
    }

    #[test]
    fn arbitrary_labels_get_stable_first_appearance_indices() {
        let out = map_remote_segments(&[
            seg("alice", 0.0, 1.0),
            seg("bob", 1.0, 2.0),
            seg("alice", 2.0, 3.0),
        ]);
        assert_eq!(out[0].speaker, 0);
        assert_eq!(out[1].speaker, 1);
        assert_eq!(out[2].speaker, 0);
    }

    #[test]
    fn output_is_sorted_by_start_time() {
        let out = map_remote_segments(&[seg("SPEAKER_01", 5.0, 6.0), seg("SPEAKER_00", 0.0, 1.0)]);
        assert!(out[0].start < out[1].start);
    }

    // --- Serde --------------------------------------------------------------

    #[test]
    fn job_status_parses_succeeded_and_failed_fixtures() {
        let succeeded: JobStatus = serde_json::from_str(
            r#"{"jobId":"j1","status":"succeeded","output":{"diarization":[{"speaker":"SPEAKER_00","start":0.5,"end":5.2}]}}"#,
        )
        .unwrap();
        assert_eq!(succeeded.status, "succeeded");
        let diar = &succeeded.output.unwrap().diarization;
        assert_eq!(diar.len(), 1);
        assert_eq!(diar[0].speaker, "SPEAKER_00");
        assert!((diar[0].end - 5.2).abs() < f64::EPSILON);

        let failed: JobStatus = serde_json::from_str(
            r#"{"jobId":"j2","status":"failed","error":{"message":"boom"}}"#,
        )
        .unwrap();
        assert_eq!(failed.status, "failed");
        assert!(failed.output.is_none());
        assert!(failed.error.is_some());
    }

    #[test]
    fn diarize_request_omits_num_speakers_when_none() {
        let without = serde_json::to_string(&DiarizeRequest {
            url: "media://x",
            model: MODEL,
            num_speakers: None,
        })
        .unwrap();
        assert!(!without.contains("numSpeakers"));

        let with = serde_json::to_string(&DiarizeRequest {
            url: "media://x",
            model: MODEL,
            num_speakers: Some(3),
        })
        .unwrap();
        assert!(with.contains(r#""numSpeakers":3"#));
    }
}
