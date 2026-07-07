// Speaker diarization (offline, post-recording).
//
// Built on the official k2-fsa `sherpa-onnx` Rust crate (Apache-2.0). Default
// build runs CPU via auto-downloaded prebuilt static libs. Enabling the
// `diarization-cuda` Cargo feature switches to dynamic linking and expects
// SHERPA_ONNX_LIB_DIR to point at a manually-fetched CUDA archive
// (sherpa-onnx-v1.13.0-cuda-12.x-cudnn-9.x-...).

pub mod aligner;
pub mod commands;
pub mod engine;
pub mod models;
