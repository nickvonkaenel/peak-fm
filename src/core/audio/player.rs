use rodio::{Decoder, OutputStream, OutputStreamHandle, Sample, Sink, Source};
use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;
use std::sync::mpsc::SyncSender;
use std::sync::Arc;

use parking_lot::Mutex;
use std::time::{Duration, Instant};

use super::tapped_source::TappedSource;

/// Temporarily suppress stderr output (for rodio/cpal errors when audio device is lost)
#[cfg(unix)]
fn suppress_stderr<F, R>(f: F) -> R
where
    F: FnOnce() -> R,
{
    use std::os::unix::io::AsRawFd;
    unsafe {
        let stderr_fd = std::io::stderr().as_raw_fd();
        let saved_fd = libc::dup(stderr_fd);
        let dev_null = libc::open(
            "/dev/null\0".as_ptr() as *const libc::c_char,
            libc::O_WRONLY,
        );
        libc::dup2(dev_null, stderr_fd);
        libc::close(dev_null);

        let result = f();

        if saved_fd >= 0 {
            libc::dup2(saved_fd, stderr_fd);
            libc::close(saved_fd);
        }
        result
    }
}

#[cfg(windows)]
fn suppress_stderr<F, R>(f: F) -> R
where
    F: FnOnce() -> R,
{
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE};
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_GENERIC_WRITE, FILE_SHARE_WRITE, OPEN_EXISTING,
    };

    unsafe {
        // Get the current stderr handle
        const STD_ERROR_HANDLE: u32 = 0xFFFFFFF4_u32; // -12 in u32
        let get_std_handle = |n: u32| -> HANDLE {
            extern "system" {
                fn GetStdHandle(nStdHandle: u32) -> HANDLE;
            }
            GetStdHandle(n)
        };
        let set_std_handle = |n: u32, h: HANDLE| -> bool {
            extern "system" {
                fn SetStdHandle(nStdHandle: u32, hHandle: HANDLE) -> i32;
            }
            SetStdHandle(n, h) != 0
        };

        let original_stderr = get_std_handle(STD_ERROR_HANDLE);

        // Redirect stderr to NUL
        let nul_path: Vec<u16> = "NUL\0".encode_utf16().collect();
        let nul_handle = CreateFileW(
            PCWSTR(nul_path.as_ptr()),
            FILE_GENERIC_WRITE.0,
            FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            HANDLE(std::ptr::null_mut()),
        );

        if let Ok(nul) = nul_handle {
            let _ = set_std_handle(STD_ERROR_HANDLE, nul);
        }

        let result = f();

        // Restore stderr
        if original_stderr != INVALID_HANDLE_VALUE {
            let _ = set_std_handle(STD_ERROR_HANDLE, original_stderr);
        }

        result
    }
}

#[cfg(not(any(unix, windows)))]
fn suppress_stderr<F, R>(f: F) -> R
where
    F: FnOnce() -> R,
{
    f()
}

/// Wrapper that converts 5-channel audio to 6-channel by adding a silent LFE channel
struct FiveToSixChannel<S>
where
    S: Source,
    S::Item: rodio::Sample,
{
    source: S,
    buffer: Vec<S::Item>,
    buffer_pos: usize,
}

impl<S> FiveToSixChannel<S>
where
    S: Source,
    S::Item: rodio::Sample,
{
    fn new(source: S) -> Self {
        Self {
            source,
            buffer: Vec::new(),
            buffer_pos: 0,
        }
    }
}

impl<S> Iterator for FiveToSixChannel<S>
where
    S: Source,
    S::Item: rodio::Sample,
{
    type Item = S::Item;

    fn next(&mut self) -> Option<Self::Item> {
        // If we have buffered samples, return them first
        if self.buffer_pos < self.buffer.len() {
            let sample = self.buffer[self.buffer_pos];
            self.buffer_pos += 1;
            return Some(sample);
        }

        // Read 5 samples from source and add a silent 6th
        self.buffer.clear();
        self.buffer_pos = 0;

        for _ in 0..5 {
            if let Some(sample) = self.source.next() {
                self.buffer.push(sample);
            } else {
                // Source exhausted
                if self.buffer.is_empty() {
                    return None;
                }
                break;
            }
        }

        // Add silent LFE channel
        if !self.buffer.is_empty() {
            self.buffer.push(S::Item::zero_value());
        }

        // Return first sample from buffer
        if !self.buffer.is_empty() {
            self.buffer_pos = 1;
            Some(self.buffer[0])
        } else {
            None
        }
    }
}

impl<S> Source for FiveToSixChannel<S>
where
    S: Source,
    S::Item: rodio::Sample,
{
    fn current_frame_len(&self) -> Option<usize> {
        self.source.current_frame_len().map(|len| (len / 5) * 6)
    }

    fn channels(&self) -> u16 {
        6 // Always output 6 channels
    }

    fn sample_rate(&self) -> u32 {
        self.source.sample_rate()
    }

    fn total_duration(&self) -> Option<Duration> {
        self.source.total_duration()
    }
}

pub struct AudioPlayer {
    _stream: OutputStream,
    stream_handle: OutputStreamHandle,
    current_sink: Arc<Mutex<Option<Sink>>>,
    current_file: Arc<Mutex<Option<PathBuf>>>,
    volume: Arc<Mutex<f32>>,
    pitch: Arc<Mutex<f32>>,
    play_start_time: Arc<Mutex<Option<Instant>>>,
    paused_position: Arc<Mutex<Duration>>,
    analyzer_sender: Option<SyncSender<(Vec<f32>, u16, u32)>>,
}

impl AudioPlayer {
    pub fn new() -> Option<Self> {
        let (stream, stream_handle) = OutputStream::try_default().ok()?;

        Some(Self {
            _stream: stream,
            stream_handle,
            current_sink: Arc::new(Mutex::new(None)),
            current_file: Arc::new(Mutex::new(None)),
            volume: Arc::new(Mutex::new(1.0)),
            pitch: Arc::new(Mutex::new(1.0)),
            play_start_time: Arc::new(Mutex::new(None)),
            paused_position: Arc::new(Mutex::new(Duration::from_secs(0))),
            analyzer_sender: None,
        })
    }

    /// Enable analyzer - store the sender for tapping audio samples
    pub fn enable_analyzer(&mut self, sender: SyncSender<(Vec<f32>, u16, u32)>) {
        self.analyzer_sender = Some(sender);
    }

    /// Disable analyzer
    pub fn disable_analyzer(&mut self) {
        self.analyzer_sender = None;
    }

    pub fn play(&self, path: PathBuf) -> Result<(), String> {
        // Stop any currently playing sound first
        let mut sink_lock = self.current_sink.lock();
        if let Some(sink) = sink_lock.take() {
            sink.stop();
        }
        drop(sink_lock);

        // Get current volume and pitch
        let volume = *self.volume.lock();
        let pitch = *self.pitch.lock();

        // Open the audio file
        let file = File::open(&path).map_err(|e| format!("Failed to open audio file: {}", e))?;
        let decoded = Decoder::new(BufReader::new(file))
            .map_err(|e| format!("Failed to decode audio file: {}", e))?;

        // Check channel count and convert 5-channel to 6-channel if needed
        let channels = decoded.channels();
        let sample_rate = decoded.sample_rate();
        let converted = decoded.convert_samples::<f32>();

        // Create new sink - suppress stderr from rodio/cpal when device is unavailable
        let sink = suppress_stderr(|| Sink::try_new(&self.stream_handle))
            .map_err(|_| "Audio device unavailable".to_string())?;

        sink.set_volume(volume);

        // Wrap with TappedSource if analyzer is enabled
        let source: Box<dyn Source<Item = f32> + Send> =
            if let Some(ref sender) = self.analyzer_sender {
                // Analyzer enabled - apply pitch first, then tap the audio stream
                // This ensures the analyzer sees the actual pitched audio being played
                if channels == 5 {
                    let pitched = FiveToSixChannel::new(converted).speed(pitch);
                    let tapped = TappedSource::new(pitched, sender.clone(), 6, sample_rate);
                    Box::new(tapped)
                } else {
                    let pitched = converted.speed(pitch);
                    let tapped = TappedSource::new(pitched, sender.clone(), channels, sample_rate);
                    Box::new(tapped)
                }
            } else {
                // No analyzer - use original flow
                if channels == 5 {
                    Box::new(FiveToSixChannel::new(converted).speed(pitch))
                } else {
                    Box::new(converted.speed(pitch))
                }
            };

        sink.append(source);

        // Update state
        *self.current_sink.lock() = Some(sink);
        *self.current_file.lock() = Some(path);

        // Reset time tracking
        *self.play_start_time.lock() = Some(Instant::now());
        *self.paused_position.lock() = Duration::from_secs(0);

        Ok(())
    }

    pub fn pause(&self) {
        if let Some(sink) = self.current_sink.lock().as_ref() {
            // Save current position when pausing (convert real time to content time)
            if let Some(start_time) = *self.play_start_time.lock() {
                let elapsed = start_time.elapsed();
                let pitch = *self.pitch.lock();
                let current_paused = *self.paused_position.lock();
                // Convert real time to content time: content_time = real_time * pitch
                let content_elapsed = Duration::from_secs_f32(elapsed.as_secs_f32() * pitch);
                *self.paused_position.lock() = current_paused + content_elapsed;
            }
            *self.play_start_time.lock() = None;
            sink.pause();
        }
    }

    pub fn resume(&self) {
        if let Some(sink) = self.current_sink.lock().as_ref() {
            // Restart timer when resuming
            *self.play_start_time.lock() = Some(Instant::now());
            sink.play();
        }
    }

    pub fn toggle_pause(&self) {
        if self.is_paused() {
            self.resume();
        } else if self.is_playing() {
            self.pause();
        }
    }

    pub fn stop(&self) {
        let mut sink_lock = self.current_sink.lock();
        if let Some(sink) = sink_lock.take() {
            sink.stop();
        }
        drop(sink_lock);

        *self.current_file.lock() = None;
        *self.play_start_time.lock() = None;
        *self.paused_position.lock() = Duration::from_secs(0);
    }

    pub fn is_playing(&self) -> bool {
        self.current_sink
            .lock()
            .as_ref()
            .map(|sink| !sink.is_paused() && !sink.empty())
            .unwrap_or(false)
    }

    pub fn is_paused(&self) -> bool {
        self.current_sink
            .lock()
            .as_ref()
            .map(|sink| sink.is_paused())
            .unwrap_or(false)
    }

    pub fn is_stopped(&self) -> bool {
        self.current_sink
            .lock()
            .as_ref()
            .map(|sink| sink.empty())
            .unwrap_or(true)
    }

    pub fn set_volume(&self, volume: f32) {
        let volume = volume.clamp(0.0, 1.0);
        *self.volume.lock() = volume;

        if let Some(sink) = self.current_sink.lock().as_ref() {
            sink.set_volume(volume);
        }
    }

    pub fn get_volume(&self) -> f32 {
        *self.volume.lock()
    }

    pub fn set_pitch(&self, pitch: f32) {
        *self.pitch.lock() = pitch;
        // Note: Pitch change requires restarting playback with new source
        // The app will handle this by calling play() again when pitch changes
    }

    pub fn get_pitch(&self) -> f32 {
        *self.pitch.lock()
    }

    // Freeze the current position by committing elapsed time to paused_position
    // This should be called before changing pitch to avoid position jumps
    pub fn freeze_position(&self) {
        if let Some(start_time) = *self.play_start_time.lock() {
            let elapsed = start_time.elapsed();
            let pitch = *self.pitch.lock();
            let adjusted_secs = elapsed.as_secs_f32() * pitch;

            // Update paused position in a single lock scope
            {
                let mut paused_pos = self.paused_position.lock();
                *paused_pos += Duration::from_secs_f32(adjusted_secs);
            }

            // Reset start time to now
            *self.play_start_time.lock() = Some(Instant::now());
        }
    }

    pub fn current_file(&self) -> Option<PathBuf> {
        self.current_file.lock().clone()
    }

    pub fn get_position(&self) -> Duration {
        // Check if the sink is empty (file finished)
        let sink_lock = self.current_sink.lock();
        let is_empty = sink_lock.as_ref().map(|sink| sink.empty()).unwrap_or(true);
        drop(sink_lock);

        // If sink is empty, reset the timer
        if is_empty {
            if self.play_start_time.lock().is_some() {
                // File just finished, save the final position
                let paused_pos = *self.paused_position.lock();
                if let Some(start_time) = self.play_start_time.lock().take() {
                    let elapsed = start_time.elapsed();
                    let pitch = *self.pitch.lock();
                    // Convert real time to content time
                    let content_elapsed = Duration::from_secs_f32(elapsed.as_secs_f32() * pitch);
                    *self.paused_position.lock() = paused_pos + content_elapsed;
                }
            }
            return *self.paused_position.lock();
        }

        let paused_pos = *self.paused_position.lock();

        if let Some(start_time) = *self.play_start_time.lock() {
            // Currently playing - calculate content position
            // Content position = paused_pos + (real_time_elapsed * pitch)
            let elapsed = start_time.elapsed();
            let pitch = *self.pitch.lock();
            let content_elapsed = Duration::from_secs_f32(elapsed.as_secs_f32() * pitch);
            paused_pos + content_elapsed
        } else {
            // Paused or stopped - return the paused position (content time)
            paused_pos
        }
    }

    pub fn seek(&self, position: Duration) -> Result<(), String> {
        // Get the current file path
        let file_path = match self.current_file.lock().clone() {
            Some(path) => path,
            None => return Ok(()), // No file playing, nothing to seek
        };

        // Check if we were playing or paused
        let was_playing = self.is_playing();

        // Stop current playback
        let mut sink_lock = self.current_sink.lock();
        if let Some(sink) = sink_lock.take() {
            sink.stop();
        }
        drop(sink_lock);

        // Get current volume and pitch
        let volume = *self.volume.lock();
        let pitch = *self.pitch.lock();

        // Open the audio file
        let file =
            File::open(&file_path).map_err(|e| format!("Failed to open audio file: {}", e))?;
        let decoded = Decoder::new(BufReader::new(file))
            .map_err(|e| format!("Failed to decode audio file: {}", e))?;

        // Check channel count and convert 5-channel to 6-channel if needed
        let channels = decoded.channels();
        let sample_rate = decoded.sample_rate();
        let converted = decoded.convert_samples::<f32>();

        // Skip to the desired position
        let skipped = converted.skip_duration(position);

        // Create new sink - suppress stderr from rodio/cpal when device is unavailable
        let sink = suppress_stderr(|| Sink::try_new(&self.stream_handle))
            .map_err(|_| "Audio device unavailable".to_string())?;

        sink.set_volume(volume);

        // Wrap with TappedSource if analyzer is enabled
        let source: Box<dyn Source<Item = f32> + Send> =
            if let Some(ref sender) = self.analyzer_sender {
                // Analyzer enabled - apply pitch first, then tap the audio stream
                // This ensures the analyzer sees the actual pitched audio being played
                if channels == 5 {
                    let pitched = FiveToSixChannel::new(skipped).speed(pitch);
                    let tapped = TappedSource::new(pitched, sender.clone(), 6, sample_rate);
                    Box::new(tapped)
                } else {
                    let pitched = skipped.speed(pitch);
                    let tapped = TappedSource::new(pitched, sender.clone(), channels, sample_rate);
                    Box::new(tapped)
                }
            } else {
                // No analyzer - use original flow
                if channels == 5 {
                    Box::new(FiveToSixChannel::new(skipped).speed(pitch))
                } else {
                    Box::new(skipped.speed(pitch))
                }
            };

        sink.append(source);

        // If we were paused, pause the new sink immediately
        if !was_playing {
            sink.pause();
        }

        // Update state
        *self.current_sink.lock() = Some(sink);
        *self.paused_position.lock() = position;

        // Update time tracking
        if was_playing {
            *self.play_start_time.lock() = Some(Instant::now());
        } else {
            *self.play_start_time.lock() = None;
        }

        Ok(())
    }

    /// Get elapsed playback time (alias for get_position for compatibility)
    pub fn elapsed(&self) -> Duration {
        self.get_position()
    }

    /// Get progress as a fraction (0.0 to 1.0) given a duration
    pub fn progress(&self, duration: Duration) -> f32 {
        if duration.is_zero() {
            0.0
        } else {
            (self.get_position().as_secs_f32() / duration.as_secs_f32()).min(1.0)
        }
    }
}
