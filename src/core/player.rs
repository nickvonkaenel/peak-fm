use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink, Source};
use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;
use std::time::{Duration, Instant};

pub struct AudioPlayer {
    _stream: OutputStream,
    stream_handle: OutputStreamHandle,
    sink: Option<Sink>,
    current_file: Option<PathBuf>,
    duration: Option<Duration>,
    play_start: Option<Instant>,
    elapsed_before_pause: Duration,
}

impl AudioPlayer {
    pub fn new() -> Option<Self> {
        let (stream, stream_handle) = OutputStream::try_default().ok()?;
        Some(Self {
            _stream: stream,
            stream_handle,
            sink: None,
            current_file: None,
            duration: None,
            play_start: None,
            elapsed_before_pause: Duration::ZERO,
        })
    }

    pub fn play(&mut self, path: PathBuf) -> Result<(), String> {
        // Stop any currently playing sound
        self.stop();

        // Open the audio file
        let file = File::open(&path).map_err(|e| format!("Failed to open audio file: {}", e))?;
        let source = Decoder::new(BufReader::new(file))
            .map_err(|e| format!("Failed to decode audio file: {}", e))?;

        // Try to get duration
        let duration = source.total_duration();

        // Create new sink
        let sink = Sink::try_new(&self.stream_handle)
            .map_err(|e| format!("Failed to create audio sink: {}", e))?;

        sink.append(source);
        self.sink = Some(sink);
        self.current_file = Some(path);
        self.duration = duration;
        self.play_start = Some(Instant::now());
        self.elapsed_before_pause = Duration::ZERO;

        Ok(())
    }

    pub fn toggle_pause(&mut self) {
        if let Some(sink) = &self.sink {
            if sink.is_paused() {
                // Resuming - restart the timer
                self.play_start = Some(Instant::now());
                sink.play();
            } else {
                // Pausing - accumulate elapsed time
                if let Some(start) = self.play_start {
                    self.elapsed_before_pause += start.elapsed();
                }
                self.play_start = None;
                sink.pause();
            }
        }
    }

    pub fn stop(&mut self) {
        if let Some(sink) = self.sink.take() {
            sink.stop();
        }
        self.current_file = None;
        self.duration = None;
        self.play_start = None;
        self.elapsed_before_pause = Duration::ZERO;
    }

    pub fn is_playing(&self) -> bool {
        self.sink
            .as_ref()
            .map(|sink| !sink.is_paused() && !sink.empty())
            .unwrap_or(false)
    }

    pub fn is_paused(&self) -> bool {
        self.sink
            .as_ref()
            .map(|sink| sink.is_paused() && !sink.empty())
            .unwrap_or(false)
    }

    pub fn current_file(&self) -> Option<&PathBuf> {
        // Only return if we actually have something playing or paused
        if self.sink.as_ref().map(|s| !s.empty()).unwrap_or(false) {
            self.current_file.as_ref()
        } else {
            None
        }
    }

    /// Get elapsed playback time
    pub fn elapsed(&self) -> Duration {
        let current = self
            .play_start
            .map(|s| s.elapsed())
            .unwrap_or(Duration::ZERO);
        self.elapsed_before_pause + current
    }

    /// Get total duration if known
    pub fn duration(&self) -> Option<Duration> {
        self.duration
    }

    /// Get progress as a fraction (0.0 to 1.0)
    pub fn progress(&self) -> Option<f32> {
        self.duration.map(|d| {
            if d.is_zero() {
                0.0
            } else {
                (self.elapsed().as_secs_f32() / d.as_secs_f32()).min(1.0)
            }
        })
    }
}
