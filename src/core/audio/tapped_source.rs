use rodio::Source;
use std::sync::mpsc::SyncSender;
use std::time::Duration;

/// Wrapper around any Source that taps samples for analysis
///
/// This wrapper passes through all audio samples unchanged while simultaneously
/// sending copies to an FFT analyzer thread via a channel. It's non-blocking -
/// if the channel is full, samples are dropped to prevent audio glitches.
pub struct TappedSource<S>
where
    S: Source<Item = f32>,
{
    inner: S,
    sample_sender: SyncSender<(Vec<f32>, u16, u32)>,
    buffer: Vec<f32>,
    buffer_size: usize,
    channels: u16,
}

impl<S> TappedSource<S>
where
    S: Source<Item = f32>,
{
    /// Create a new TappedSource
    ///
    /// # Arguments
    /// * `inner` - The source to wrap
    /// * `sample_sender` - Channel to send (samples, channels, sample_rate) for FFT analysis
    /// * `channels` - Number of audio channels
    pub fn new(inner: S, sample_sender: SyncSender<(Vec<f32>, u16, u32)>, channels: u16) -> Self {
        Self {
            inner,
            sample_sender,
            buffer: Vec::with_capacity(2048),
            buffer_size: 2048,
            channels,
        }
    }

    /// Attempt to send buffered samples to the analyzer
    fn try_send_buffer(&mut self) {
        if self.buffer.is_empty() {
            return;
        }

        // Clone the buffer for sending (we need to keep iterating)
        let samples = std::mem::take(&mut self.buffer);

        // Get the actual sample rate from the inner source (may be modified by speed/pitch)
        let actual_sample_rate = self.inner.sample_rate();

        // Try to send - if channel is full, just drop the samples
        // This is important: we never want to block the audio thread
        let _ = self
            .sample_sender
            .try_send((samples, self.channels, actual_sample_rate));

        // Clear buffer (already empty from mem::take, but be explicit)
        self.buffer.clear();
    }
}

impl<S> Iterator for TappedSource<S>
where
    S: Source<Item = f32>,
{
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        // Get the next sample from the inner source
        let sample = self.inner.next()?;

        // Accumulate in buffer
        self.buffer.push(sample);

        // When buffer is full, try to send it
        if self.buffer.len() >= self.buffer_size {
            self.try_send_buffer();
        }

        // Return the sample unchanged (passthrough)
        Some(sample)
    }
}

impl<S> Source for TappedSource<S>
where
    S: Source<Item = f32>,
{
    fn current_frame_len(&self) -> Option<usize> {
        self.inner.current_frame_len()
    }

    fn channels(&self) -> u16 {
        self.inner.channels()
    }

    fn sample_rate(&self) -> u32 {
        self.inner.sample_rate()
    }

    fn total_duration(&self) -> Option<Duration> {
        self.inner.total_duration()
    }
}

impl<S> Drop for TappedSource<S>
where
    S: Source<Item = f32>,
{
    fn drop(&mut self) {
        // Send any remaining buffered samples when the source is dropped
        if !self.buffer.is_empty() {
            self.try_send_buffer();
        }
    }
}
