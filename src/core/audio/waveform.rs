use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::time::{Duration as StdDuration, SystemTime};

use crate::paths::APP_DIR_NAME;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

// Version 1: 200 peaks
// Version 2: 400 peaks
// Version 3: 400 peaks + accurate duration + sample rate
const WAVEFORM_CACHE_VERSION: u32 = 3;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WaveformData {
    pub version: u32,
    pub peaks: Vec<(f32, f32)>,        // (min, max) for each bucket
    pub duration: Option<StdDuration>, // Accurate duration from decoded samples
    pub sample_rate: Option<u32>,
}

impl WaveformData {
    #[allow(dead_code)]
    pub fn generate(path: &Path, target_width: usize) -> Result<Self, String> {
        let file = File::open(path).map_err(|e| e.to_string())?;
        let mss = MediaSourceStream::new(Box::new(file), Default::default());

        let mut hint = Hint::new();
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            hint.with_extension(ext);
        }

        let meta_opts: MetadataOptions = Default::default();
        let fmt_opts: FormatOptions = Default::default();

        let probed = symphonia::default::get_probe()
            .format(&hint, mss, &fmt_opts, &meta_opts)
            .map_err(|e| format!("Failed to probe audio format: {}", e))?;

        let mut format = probed.format;
        let track = format
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
            .ok_or_else(|| "No suitable audio track found".to_string())?;

        let track_id = track.id;
        let sample_rate = track.codec_params.sample_rate;
        let channels = track.codec_params.channels.map(|c| c.count());

        let mut decoder = symphonia::default::get_codecs()
            .make(&track.codec_params, &DecoderOptions { verify: false })
            .map_err(|e| format!("Failed to create decoder: {}", e))?;

        let mut samples = Vec::new();
        let mut sample_buf = None;

        // Decode all samples
        while let Ok(packet) = format.next_packet() {
            if packet.track_id() != track_id {
                continue;
            }

            match decoder.decode(&packet) {
                Ok(decoded) => {
                    if sample_buf.is_none() {
                        let spec = *decoded.spec();
                        let duration = decoded.capacity() as u64;
                        sample_buf = Some(SampleBuffer::<f32>::new(duration, spec));
                    }

                    if let Some(buf) = &mut sample_buf {
                        buf.copy_interleaved_ref(decoded);
                        samples.extend_from_slice(buf.samples());
                    }
                }
                Err(_) => break,
            }
        }

        if samples.is_empty() {
            return Ok(Self {
                version: WAVEFORM_CACHE_VERSION,
                peaks: Vec::new(),
                duration: None,
                sample_rate,
            });
        }

        // Calculate duration from sample count
        let duration = if let (Some(sr), Some(ch)) = (sample_rate, channels) {
            if sr > 0 && ch > 0 {
                let total_frames = samples.len() / ch;
                let secs = total_frames as f64 / sr as f64;
                Some(StdDuration::from_secs_f64(secs))
            } else {
                None
            }
        } else {
            None
        };

        // Downsample to target width - distribute samples evenly across buckets
        let mut peaks = Vec::with_capacity(target_width);
        let total_samples = samples.len();

        for bucket_idx in 0..target_width {
            // Calculate the exact range of samples for this bucket
            let start = (bucket_idx * total_samples) / target_width;
            let end = ((bucket_idx + 1) * total_samples) / target_width;

            if start >= end {
                continue;
            }

            let mut min = f32::MAX;
            let mut max = f32::MIN;

            for &sample in &samples[start..end] {
                min = min.min(sample);
                max = max.max(sample);
            }

            peaks.push((min, max));
        }

        Ok(Self {
            version: WAVEFORM_CACHE_VERSION,
            peaks,
            duration,
            sample_rate,
        })
    }

    pub fn is_empty(&self) -> bool {
        self.peaks.is_empty()
    }

    /// Get the maximum absolute peak value in the waveform
    pub fn get_max_peak(&self) -> f32 {
        self.peaks
            .iter()
            .map(|(min, max)| max.abs().max(min.abs()))
            .fold(0.0f32, f32::max)
    }

    /// Calculate normalization gain to bring peaks to target level
    /// Typical usage: target_level = 10^(-1/20) ≈ 0.8913 for -1 dB
    pub fn calculate_normalize_gain(&self, target_level: f32) -> f32 {
        let max_peak = self.get_max_peak();
        if max_peak > 0.0001 {
            // Avoid division by near-zero
            target_level / max_peak
        } else {
            1.0 // No gain if essentially silent
        }
    }

    /// Check if the given position ratio (0.0-1.0) is in a silent region
    pub fn is_silent_at(&self, pos_ratio: f32, silence_threshold: f32) -> bool {
        if self.peaks.is_empty() {
            return true;
        }

        // Convert position ratio to peak index
        let idx = ((pos_ratio * self.peaks.len() as f32) as usize)
            .min(self.peaks.len().saturating_sub(1));

        // Check current position and a few surrounding peaks for silence
        let check_range = 3; // Check +/-3 peaks around current position
        let start_idx = idx.saturating_sub(check_range);
        let end_idx = (idx + check_range).min(self.peaks.len());

        // If most peaks in this range are silent, consider it a silent region
        let mut silent_count = 0;
        let mut total_count = 0;

        for i in start_idx..end_idx {
            let (min, max) = self.peaks[i];
            let amplitude = max.abs().max(min.abs());

            if amplitude < silence_threshold {
                silent_count += 1;
            }
            total_count += 1;
        }

        // Consider it silent if >70% of checked peaks are silent
        silent_count as f32 / total_count as f32 > 0.7
    }

    /// Find the immediate next sound from current position (for auto-skip during playback)
    /// Returns the position ratio of the next non-silent sample, or None if not found
    pub fn find_immediate_next_sound(
        &self,
        current_pos_ratio: f32,
        silence_threshold: f32,
    ) -> Option<f32> {
        if self.peaks.is_empty() {
            return None;
        }

        // Convert position ratio to peak index
        let current_idx = ((current_pos_ratio * self.peaks.len() as f32) as usize)
            .min(self.peaks.len().saturating_sub(1));

        // Skip ahead beyond the silence detection window to avoid seek loops
        // is_silent_at checks +/- 3 peaks, so we need to skip at least 7 peaks ahead
        let min_skip_peaks = 7;
        let start_idx = (current_idx + min_skip_peaks).min(self.peaks.len().saturating_sub(1));

        // Search for the first non-silent peak
        for i in start_idx..self.peaks.len() {
            let (min, max) = self.peaks[i];
            let amplitude = max.abs().max(min.abs());

            if amplitude >= silence_threshold {
                // Found sound - return this position
                return Some(i as f32 / self.peaks.len() as f32);
            }
        }

        None
    }

    /// Find the next region after a silent section, starting from the given position ratio (0.0-1.0)
    /// Returns the position ratio of the next non-silent region, or None if not found
    pub fn find_next_region(&self, current_pos_ratio: f32, silence_threshold: f32) -> Option<f32> {
        if self.peaks.is_empty() {
            return None;
        }

        // Convert position ratio to peak index, clamping to valid range
        let current_idx = ((current_pos_ratio * self.peaks.len() as f32) as usize)
            .min(self.peaks.len().saturating_sub(1));

        // Skip at least 5% forward to avoid finding the current region
        let min_skip = (self.peaks.len() as f32 * 0.05) as usize;
        let start_idx = (current_idx + min_skip).min(self.peaks.len() - 1);

        // Don't search in the last 10% of the file to avoid detecting tail silence as a region
        let max_search_idx = (self.peaks.len() as f32 * 0.90) as usize;

        // Require at least 10 consecutive silent peaks to consider it real silence (hysteresis)
        let min_silence_peaks = 10;
        let mut silence_count = 0;

        for i in start_idx..max_search_idx.min(self.peaks.len()) {
            let (min, max) = self.peaks[i];
            let amplitude = max.abs().max(min.abs());

            if amplitude < silence_threshold {
                silence_count += 1;
            } else {
                // Sound detected
                if silence_count >= min_silence_peaks {
                    // We had sustained silence and now found sound - this is the start of next region
                    return Some(i as f32 / self.peaks.len() as f32);
                }
                // Reset if we didn't have enough sustained silence
                silence_count = 0;
            }
        }

        None
    }

    /// Find the previous region before a silent section, starting from the given position ratio (0.0-1.0)
    /// Returns the position ratio of the previous non-silent region, or None if not found
    pub fn find_previous_region(
        &self,
        current_pos_ratio: f32,
        silence_threshold: f32,
    ) -> Option<f32> {
        if self.peaks.is_empty() {
            return None;
        }

        // Convert position ratio to peak index, clamping to valid range
        let current_idx = ((current_pos_ratio * self.peaks.len() as f32) as usize)
            .min(self.peaks.len().saturating_sub(1));

        if current_idx == 0 {
            return Some(0.0);
        }

        // Require at least 10 consecutive silent peaks to consider it real silence (hysteresis)
        let min_silence_peaks = 10;

        // Phase 1: Skip backwards through current sound region (if we're in one)
        let mut i = current_idx;
        let mut silence_count = 0;

        while i > 0 {
            let (min, max) = self.peaks[i];
            let amplitude = max.abs().max(min.abs());
            if amplitude < silence_threshold {
                silence_count += 1;
                if silence_count >= min_silence_peaks {
                    // Found sustained silence, move to phase 2
                    break;
                }
            } else {
                silence_count = 0;
            }
            i -= 1;
        }

        if i == 0 {
            // We're in the first region
            return Some(0.0);
        }

        // Phase 2: Continue skipping backwards through silence
        while i > 0 {
            let (min, max) = self.peaks[i];
            let amplitude = max.abs().max(min.abs());
            if amplitude >= silence_threshold {
                // Found sound again - this is the previous region, now find its start
                break;
            }
            i -= 1;
        }

        if i == 0 {
            return Some(0.0);
        }

        // Phase 3: Find the start of this previous region
        let mut region_start = i;
        silence_count = 0;

        while region_start > 0 {
            let (min, max) = self.peaks[region_start - 1];
            let amplitude = max.abs().max(min.abs());
            if amplitude < silence_threshold {
                silence_count += 1;
                if silence_count >= min_silence_peaks {
                    // Found sustained silence before this region
                    break;
                }
            } else {
                silence_count = 0;
            }
            region_start -= 1;
        }

        Some(region_start as f32 / self.peaks.len() as f32)
    }

    /// Find the last region in the waveform
    /// Returns the position ratio of the start of the last sound region
    pub fn find_last_region(&self, silence_threshold: f32) -> Option<f32> {
        if self.peaks.is_empty() {
            return None;
        }

        // Start from the end (90% to avoid tail silence) and search backwards
        let max_search_idx = (self.peaks.len() as f32 * 0.90) as usize;
        let min_silence_peaks = 10;
        let mut silence_count = 0;
        let mut i = max_search_idx;

        // Phase 1: Skip backwards through any trailing silence
        while i > 0 {
            let (min, max) = self.peaks[i];
            let amplitude = max.abs().max(min.abs());
            if amplitude >= silence_threshold {
                // Found sound, this is in the last region
                break;
            }
            i -= 1;
        }

        if i == 0 {
            return Some(0.0);
        }

        // Phase 2: Find the start of this last region
        let mut region_start = i;
        while region_start > 0 {
            let (min, max) = self.peaks[region_start];
            let amplitude = max.abs().max(min.abs());
            if amplitude < silence_threshold {
                silence_count += 1;
                if silence_count >= min_silence_peaks {
                    // Found sustained silence, the region starts after this
                    region_start += min_silence_peaks;
                    break;
                }
            } else {
                silence_count = 0;
            }
            region_start -= 1;
        }

        Some(region_start as f32 / self.peaks.len() as f32)
    }

    pub fn generate_progressive(
        path: &Path,
        target_width: usize,
        sender: &Sender<(PathBuf, WaveformData, StdDuration)>,
        progressive: bool,
    ) -> Result<Self, String> {
        let path_buf = path.to_path_buf();
        let file = File::open(path).map_err(|e| e.to_string())?;
        let mss = MediaSourceStream::new(Box::new(file), Default::default());

        let mut hint = Hint::new();
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            hint.with_extension(ext);
        }

        let meta_opts: MetadataOptions = Default::default();
        let fmt_opts: FormatOptions = Default::default();

        let probed = symphonia::default::get_probe()
            .format(&hint, mss, &fmt_opts, &meta_opts)
            .map_err(|e| format!("Failed to probe audio format: {}", e))?;

        let mut format = probed.format;
        let track = format
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
            .ok_or_else(|| "No suitable audio track found".to_string())?;

        let track_id = track.id;
        let sample_rate = track.codec_params.sample_rate;
        let channels = track.codec_params.channels.map(|c| c.count());

        let mut decoder = symphonia::default::get_codecs()
            .make(&track.codec_params, &DecoderOptions { verify: false })
            .map_err(|e| format!("Failed to create decoder: {}", e))?;

        let mut samples = Vec::new();
        let mut sample_buf = None;

        // Decode all samples
        while let Ok(packet) = format.next_packet() {
            if packet.track_id() != track_id {
                continue;
            }

            match decoder.decode(&packet) {
                Ok(decoded) => {
                    if sample_buf.is_none() {
                        let spec = *decoded.spec();
                        let duration = decoded.capacity() as u64;
                        sample_buf = Some(SampleBuffer::<f32>::new(duration, spec));
                    }

                    if let Some(buf) = &mut sample_buf {
                        buf.copy_interleaved_ref(decoded);
                        samples.extend_from_slice(buf.samples());
                    }
                }
                Err(_) => break,
            }
        }

        if samples.is_empty() {
            return Ok(Self {
                version: WAVEFORM_CACHE_VERSION,
                peaks: Vec::new(),
                duration: None,
                sample_rate,
            });
        }

        // Calculate duration from sample count
        let duration = if let (Some(sr), Some(ch)) = (sample_rate, channels) {
            if sr > 0 && ch > 0 {
                let total_frames = samples.len() / ch;
                let secs = total_frames as f64 / sr as f64;
                Some(StdDuration::from_secs_f64(secs))
            } else {
                None
            }
        } else {
            None
        };

        // Downsample to target width with progressive updates
        let mut peaks = Vec::with_capacity(target_width);
        let total_samples = samples.len();
        const UPDATE_EVERY: usize = 20; // Send update every 20 peaks

        for bucket_idx in 0..target_width {
            // Calculate the exact range of samples for this bucket
            let start = (bucket_idx * total_samples) / target_width;
            let end = ((bucket_idx + 1) * total_samples) / target_width;

            if start >= end {
                continue;
            }

            let mut min = f32::MAX;
            let mut max = f32::MIN;

            for &sample in &samples[start..end] {
                min = min.min(sample);
                max = max.max(sample);
            }

            peaks.push((min, max));

            // Send progressive update every UPDATE_EVERY buckets (only if progressive mode is enabled)
            if progressive
                && ((bucket_idx + 1) % UPDATE_EVERY == 0 || bucket_idx == target_width - 1)
            {
                let partial_waveform = WaveformData {
                    version: WAVEFORM_CACHE_VERSION,
                    peaks: peaks.clone(),
                    duration,
                    sample_rate,
                };
                let _ = sender.send((
                    path_buf.clone(),
                    partial_waveform,
                    duration.unwrap_or(StdDuration::from_secs(0)),
                ));
            }
        }

        Ok(Self {
            version: WAVEFORM_CACHE_VERSION,
            peaks,
            duration,
            sample_rate,
        })
    }

    fn get_cache_path(path: &Path) -> Result<PathBuf, String> {
        // Get file metadata for modification time
        let metadata = fs::metadata(path).map_err(|e| e.to_string())?;
        let modified = metadata.modified().map_err(|e| e.to_string())?;
        let modified_secs = modified
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or(StdDuration::from_secs(0))
            .as_secs();

        // Create hash of file path + modification time
        let path_str = path.to_string_lossy();
        let key = format!("{}{}", path_str, modified_secs);
        let hash = blake3::hash(key.as_bytes());
        let hash_hex = hash.to_hex();

        // Get the Peak File Manager waveform cache directory.
        let cache_dir = if let Some(home) =
            std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))
        {
            PathBuf::from(home)
                .join(".cache")
                .join(APP_DIR_NAME)
                .join("waveforms")
        } else {
            PathBuf::from(".cache").join(APP_DIR_NAME).join("waveforms")
        };

        // Create cache directory if it doesn't exist
        fs::create_dir_all(&cache_dir).map_err(|e| e.to_string())?;

        Ok(cache_dir.join(format!("{}.bin", &hash_hex[..16])))
    }

    pub fn load_from_cache(path: &Path) -> Option<Self> {
        let cache_path = Self::get_cache_path(path).ok()?;
        let cached_data = fs::read(&cache_path).ok()?;
        let waveform: Self = bincode::deserialize(&cached_data).ok()?;

        // Check version - if it doesn't match, reject the cached data
        if waveform.version != WAVEFORM_CACHE_VERSION {
            return None;
        }

        Some(waveform)
    }

    fn save_to_cache(&self, path: &Path) -> Result<(), String> {
        let cache_path = Self::get_cache_path(path)?;
        let encoded = bincode::serialize(self).map_err(|e| e.to_string())?;
        fs::write(&cache_path, encoded).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn generate_with_cache(
        path: &Path,
        target_width: usize,
        sender: &Sender<(PathBuf, WaveformData, StdDuration)>,
        progressive: bool,
    ) -> Result<Self, String> {
        // Try to load from cache first
        if let Some(cached) = Self::load_from_cache(path) {
            return Ok(cached);
        }

        // Cache miss - generate waveform
        let waveform = Self::generate_progressive(path, target_width, sender, progressive)?;

        // Save to cache (ignore errors)
        let _ = waveform.save_to_cache(path);

        Ok(waveform)
    }

    pub fn clear_cache() -> Result<(), String> {
        // Get cache directory
        let cache_dir = if let Some(home) =
            std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))
        {
            PathBuf::from(home)
                .join(".cache")
                .join(APP_DIR_NAME)
                .join("waveforms")
        } else {
            PathBuf::from(".cache").join(APP_DIR_NAME).join("waveforms")
        };

        // Remove the cache directory and all its contents
        if cache_dir.exists() {
            fs::remove_dir_all(&cache_dir).map_err(|e| e.to_string())?;
            // Recreate the empty directory
            fs::create_dir_all(&cache_dir).map_err(|e| e.to_string())?;
        }

        Ok(())
    }
}
