//! Audio browser mode (fsf integration)

use std::path::PathBuf;
use std::sync::mpsc::{channel, sync_channel, Receiver, Sender, SyncSender};
use std::time::{Duration, Instant};

use crate::core::audio::{
    AnalyzerConfig, AnalyzerFrame, AudioPlayer, Database, Metadata, SpectrumAnalyzer, WaveformData,
};
use crate::paths::APP_DIR_NAME;

/// Get the database path for a specific scan root
/// Uses blake3 hash of the canonical path to create unique DB per directory
fn get_db_path(scan_root: &std::path::Path) -> Option<PathBuf> {
    let canonical = scan_root.canonicalize().ok()?;
    let hash = blake3::hash(canonical.to_string_lossy().as_bytes());
    let hash_str = &hash.to_hex()[..16]; // First 16 chars for shorter filename

    dirs::config_dir().map(|p| {
        p.join(APP_DIR_NAME)
            .join("audio")
            .join(format!("{}.db", hash_str))
    })
}

/// A single audio file entry
#[derive(Clone, Debug)]
pub struct AudioFile {
    pub path: PathBuf,
    pub filename: String,
    pub description: Option<String>,
}

/// Parsed search query with boolean operators
struct ParsedQuery {
    /// OR groups - each group is a vec of AND terms
    or_groups: Vec<Vec<String>>,
    /// Exclusion terms (prefixed with -)
    exclusions: Vec<String>,
}

/// State for audio browser mode
pub struct AudioModeState {
    pub files: Vec<AudioFile>,
    pub filtered_indices: Vec<usize>,
    pub selected: usize,
    pub scroll_offset: usize,
    pub search_query: String,
    previous_query: String,
    pub browse_mode: bool,
    is_shuffled: bool,

    // Playback
    pub player: Option<AudioPlayer>,
    pub current_metadata: Option<Metadata>,
    pub current_waveform: Option<WaveformData>,
    pub selected_metadata: Option<Metadata>, // Metadata for selected file (not necessarily playing)
    pub autoplay: bool,
    pub show_waveform: bool,
    pub show_info: bool,
    pub normalize_waveform: bool,
    pub skip_silence: bool,
    last_silence_check: Option<Instant>,

    // Volume/pitch in dB/semitones
    pub volume_db: f32,
    pub pitch_semitones: f32,
    pub normalize_gain: f32, // Calculated gain for normalization

    // Waveform background generation
    waveform_sender: Sender<(PathBuf, WaveformData, Duration)>,
    waveform_receiver: Receiver<(PathBuf, WaveformData, Duration)>,

    // Status
    pub status_message: Option<String>,
    pub status_time: Option<Instant>,

    // Scan state
    pub scan_root: PathBuf,
    pub scan_complete: bool,
    scan_receiver: Option<Receiver<AudioFile>>,
    /// Adaptive batch size for polling (auto-tuned for performance)
    batch_size: usize,

    // Database
    pub db_loaded: bool,
    pub db_file_count: usize,
    pub db_building: bool,
    db_status_receiver: Option<Receiver<String>>,

    // Frequency analyzer
    pub show_analyzer: bool,
    pub analyzer_gradient: bool,
    pub current_analyzer_frame: Option<AnalyzerFrame>,
    analyzer_receiver: Option<Receiver<AnalyzerFrame>>,
    analyzer_fft_sender: Option<SyncSender<(Vec<f32>, u16, u32)>>,
    analyzer_fade_start: Option<Instant>,
}

impl AudioModeState {
    pub fn new(
        scan_root: PathBuf,
        autoplay: bool,
        normalize: bool,
        skip_silence: bool,
        volume: f32,
        analyzer_gradient: bool,
    ) -> Self {
        let (waveform_sender, waveform_receiver) = channel();
        let player = AudioPlayer::new();

        // Convert linear amplitude to dB: dB = 20 * log10(amplitude)
        let volume_db = if volume > 0.0 {
            20.0 * volume.log10()
        } else {
            -60.0 // Minimum volume
        };

        Self {
            files: Vec::new(),
            filtered_indices: Vec::new(),
            selected: 0,
            scroll_offset: 0,
            search_query: String::new(),
            previous_query: String::new(),
            browse_mode: false,
            is_shuffled: false,
            player,
            current_metadata: None,
            current_waveform: None,
            selected_metadata: None,
            autoplay,
            show_waveform: true,
            show_info: true,
            normalize_waveform: normalize,
            skip_silence,
            last_silence_check: None,
            volume_db,
            pitch_semitones: 0.0,
            normalize_gain: 1.0,
            waveform_sender,
            waveform_receiver,
            status_message: None,
            status_time: None,
            scan_root,
            scan_complete: false,
            scan_receiver: None,
            batch_size: 20_000, // Start at previous default, will auto-tune
            db_loaded: false,
            db_file_count: 0,
            db_building: false,
            db_status_receiver: None,
            show_analyzer: true,
            analyzer_gradient,
            current_analyzer_frame: None,
            analyzer_receiver: None,
            analyzer_fft_sender: None,
            analyzer_fade_start: None,
        }
    }

    /// Start scanning for audio files in the background
    /// First tries to load from database, falls back to filesystem scan
    pub fn start_scan(&mut self) {
        // Try to load from database first
        if let Some(db_path) = get_db_path(&self.scan_root) {
            if db_path.exists() {
                if let Ok(db) = Database::open(&db_path) {
                    if db.is_complete().unwrap_or(false) {
                        // Load from database in batches (non-blocking)
                        let (sender, receiver) = channel();
                        self.scan_receiver = Some(receiver);

                        let db_path_clone = db_path.clone();
                        std::thread::spawn(move || {
                            if let Ok(db) = Database::open(&db_path_clone) {
                                // Load in batches of 50000 for faster loading
                                const BATCH_SIZE: i64 = 50000;
                                let mut offset = 0;

                                loop {
                                    match db.get_files_batch(BATCH_SIZE, offset) {
                                        Ok(batch) if !batch.is_empty() => {
                                            for record in batch {
                                                let file = AudioFile {
                                                    path: PathBuf::from(&record.file_path),
                                                    filename: record.file_name,
                                                    description: record.description,
                                                };
                                                if sender.send(file).is_err() {
                                                    return;
                                                }
                                            }
                                            offset += BATCH_SIZE;
                                        }
                                        _ => break,
                                    }
                                }
                            }
                        });

                        self.db_loaded = true;
                        self.set_status("Loading from database...".to_string());
                        return;
                    }
                }
            }
        }

        // Fall back to filesystem scan
        let (sender, receiver) = channel();
        self.scan_receiver = Some(receiver);

        let root = self.scan_root.clone();
        std::thread::spawn(move || {
            scan_audio_files(&root, sender);
        });
    }

    /// Rebuild the database by rescanning all files
    pub fn rebuild_database(&mut self) {
        self.files.clear();
        self.filtered_indices.clear();
        self.selected = 0;
        self.scroll_offset = 0;
        self.scan_complete = false;
        self.db_loaded = false;
        self.db_building = true; // Mark DB building as in progress

        // Thread 1: Fast UI scan (parallel, no metadata, no DB)
        let (ui_sender, ui_receiver) = channel();
        self.scan_receiver = Some(ui_receiver);

        let root_for_ui = self.scan_root.clone();
        std::thread::spawn(move || {
            scan_audio_files(&root_for_ui, ui_sender);
        });

        // Thread 2: Silent database building (separate, doesn't block UI)
        let (db_status_sender, db_status_receiver) = channel();
        self.db_status_receiver = Some(db_status_receiver);

        let root_for_db = self.scan_root.clone();
        let progress_sender = db_status_sender.clone();
        std::thread::spawn(
            move || match build_database_silent(&root_for_db, progress_sender) {
                Ok((duration, file_count)) => {
                    let message = format!(
                        "COMPLETE:Database rebuilt in {:.2}s ({} files)",
                        duration.as_secs_f64(),
                        file_count
                    );
                    let _ = db_status_sender.send(message);
                }
                Err(e) => {
                    let _ = db_status_sender.send(format!("ERROR:{}", e));
                }
            },
        );

        self.set_status("Scanning files...".to_string());
    }

    /// Poll for scan results
    pub fn poll_scan(&mut self) {
        const TARGET_MS: u128 = 10;
        const MIN_BATCH: usize = 1000;
        const MAX_BATCH: usize = 50_000;

        // Check for database build status messages FIRST (before early return)
        // Collect all messages first to avoid borrow checker issues
        let mut messages = Vec::new();
        if let Some(ref receiver) = self.db_status_receiver {
            while let Ok(message) = receiver.try_recv() {
                messages.push(message);
            }
        }

        // Process collected messages
        for message in messages {
            if let Some(progress) = message.strip_prefix("PROGRESS:") {
                // Progress update - just update status
                if let Ok(count) = progress.parse::<usize>() {
                    self.set_status(format!("Building database... {} files", count));
                }
            } else if let Some(completion) = message.strip_prefix("COMPLETE:") {
                // Completion message - reload database
                self.db_building = false;

                // Reload database and update files with descriptions
                if let Some(db_path) = get_db_path(&self.scan_root) {
                    if let Ok(db) = Database::open(&db_path) {
                        if db.is_complete().unwrap_or(false) {
                            // Get descriptions from database
                            if let Ok(files_with_desc) = db.get_all_files_with_descriptions() {
                                // Create a map of path -> description for quick lookup
                                let desc_map: std::collections::HashMap<_, _> = files_with_desc
                                    .into_iter()
                                    .map(|(record, desc)| (record.file_path, desc))
                                    .collect();

                                // Update existing files with descriptions
                                for file in &mut self.files {
                                    if let Some(desc) =
                                        desc_map.get(file.path.to_string_lossy().as_ref())
                                    {
                                        file.description = desc.clone();
                                    }
                                }

                                self.db_loaded = true;
                                self.db_file_count = desc_map.len();
                            }
                        }
                    }
                }

                self.set_status(completion.to_string());
            } else if let Some(error) = message.strip_prefix("ERROR:") {
                // Error message
                self.db_building = false;
                self.set_status(format!("Database build failed: {}", error));
            }
        }

        if self.scan_complete {
            return;
        }

        if let Some(receiver) = &self.scan_receiver {
            let start = Instant::now();
            let mut added = 0;
            let batch_size = self.batch_size;

            loop {
                if added >= batch_size {
                    break;
                }

                match receiver.try_recv() {
                    Ok(file) => {
                        let idx = self.files.len();
                        self.files.push(file);
                        // If no search query, add to filtered indices
                        if self.search_query.is_empty() {
                            self.filtered_indices.push(idx);
                        }
                        added += 1;
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        self.scan_complete = true;
                        // Only sort if not loaded from database (database is already sorted)
                        if !self.db_loaded {
                            self.files.sort_by(|a, b| a.path.cmp(&b.path));
                        }
                        // Rebuild filtered indices
                        if self.search_query.is_empty() {
                            self.filtered_indices = (0..self.files.len()).collect();
                        } else {
                            self.update_filter();
                        }
                        // Load waveform for first item
                        self.generate_waveform_for_selected();

                        // Update status with final count
                        if self.db_loaded {
                            self.set_status(format!(
                                "Loaded {} files from database",
                                self.files.len()
                            ));
                        }
                        break;
                    }
                }
            }

            // Adapt batch size based on how long this took
            let elapsed_ms = start.elapsed().as_millis();
            if elapsed_ms < TARGET_MS / 2 && added == batch_size {
                // We hit the limit and had time to spare - increase
                self.batch_size = (self.batch_size * 2).min(MAX_BATCH);
            } else if elapsed_ms > TARGET_MS * 2 {
                // Too slow - decrease
                self.batch_size = (self.batch_size / 2).max(MIN_BATCH);
            }
        }
    }

    /// Poll for waveform generation results
    pub fn poll_waveform(&mut self) {
        while let Ok((path, waveform, _elapsed)) = self.waveform_receiver.try_recv() {
            // Apply if it matches the currently selected file
            if let Some(selected_file) = self.selected_file() {
                if selected_file.path == path {
                    // Calculate normalization gain from waveform (-1 dB target)
                    // -1 dB = 10^(-1/20) ≈ 0.8913
                    let target_db = -1.0;
                    let target_amplitude = 10.0_f32.powf(target_db / 20.0);
                    self.normalize_gain = waveform.calculate_normalize_gain(target_amplitude);
                    self.current_waveform = Some(waveform);

                    // Update volume to apply normalization if enabled
                    self.update_volume();
                }
            }
        }
    }

    /// Poll for analyzer frames (called from main event loop)
    pub fn poll_analyzer(&mut self) {
        let is_playing = self.is_playing();

        if let Some(ref receiver) = self.analyzer_receiver {
            // Get latest frame (discard old ones to stay current)
            let mut received_frame = false;
            while let Ok(frame) = receiver.try_recv() {
                self.current_analyzer_frame = Some(frame);
                received_frame = true;
            }

            // Reset fade timer if we received a new frame while playing
            if received_frame && is_playing {
                self.analyzer_fade_start = None;
            }
        }

        // Implement fade-to-zero when playback is stopped
        if !is_playing && !self.is_paused() {
            // Start fade timer if not already started
            if self.analyzer_fade_start.is_none() && self.current_analyzer_frame.is_some() {
                self.analyzer_fade_start = Some(Instant::now());
            }

            // Apply exponential decay to current frame
            if let (Some(fade_start), Some(frame)) =
                (self.analyzer_fade_start, &mut self.current_analyzer_frame)
            {
                let fade_elapsed = fade_start.elapsed().as_secs_f32();

                // Decay factor: exponential decay matching the analyzer's smoothing factor (0.7)
                // After ~0.5 seconds, the amplitude should be near zero
                // Using decay_rate that gives us ~0.3 per frame at 30 Hz
                const DECAY_PER_SECOND: f32 = 0.05; // 5% remaining after 1 second
                let decay_factor = DECAY_PER_SECOND.powf(fade_elapsed);

                // Apply decay to all bands
                for magnitude in &mut frame.bands {
                    *magnitude *= decay_factor;
                }

                // Clear frame if it's essentially zero
                if fade_elapsed > 1.5 {
                    self.current_analyzer_frame = None;
                    self.analyzer_fade_start = None;
                }
            }
        } else {
            // Reset fade timer when playing or paused
            self.analyzer_fade_start = None;
        }
    }

    /// Start the FFT analyzer thread
    pub fn start_analyzer(&mut self, _terminal_width: usize) {
        if self.analyzer_receiver.is_some() {
            return; // Already running
        }

        // Use a fixed high-resolution number of bands (200 bands covers 20Hz-20kHz nicely)
        // This gives ~10 bands per octave (20-20000 Hz is ~10 octaves)
        // The rendering function will map these to the display width
        let num_bands = 200;

        // Create channel from audio thread to FFT thread
        let (audio_sender, audio_receiver) = sync_channel::<(Vec<f32>, u16, u32)>(8);

        // Create channel from FFT thread to UI thread
        let (analyzer_sender, analyzer_receiver) = channel::<AnalyzerFrame>();

        // Store receivers and senders
        self.analyzer_receiver = Some(analyzer_receiver);
        self.analyzer_fft_sender = Some(audio_sender.clone());

        // Enable analyzer in player (this makes TappedSource send samples)
        if let Some(player) = &mut self.player {
            player.enable_analyzer(audio_sender);
        }

        // Spawn FFT processing thread
        std::thread::spawn(move || {
            let config = AnalyzerConfig {
                fft_size: 8192,     // Increased for better low-frequency resolution
                sample_rate: 48000, // Default, updated dynamically
                update_rate_hz: 30,
                min_freq: 20.0,
                max_freq: 20000.0,
                slope_db_per_octave: 4.5,
                reference_magnitude: 0.001, // Adjusted for constant-Q power averaging
            };

            let mut analyzer = SpectrumAnalyzer::new(config);

            // Process samples as they arrive
            while let Ok((samples, channels, sample_rate)) = audio_receiver.recv() {
                // Update sample rate if changed
                analyzer.set_sample_rate(sample_rate);

                // Mixdown to mono
                let mono_samples = SpectrumAnalyzer::mixdown_to_mono(&samples, channels);

                // Push samples and get frame if ready
                if let Some(frame) = analyzer.push_samples(&mono_samples, num_bands) {
                    // Try to send to UI (non-blocking)
                    let _ = analyzer_sender.send(frame);
                }
            }
        });
    }

    /// Stop analyzer thread
    pub fn stop_analyzer(&mut self) {
        self.analyzer_receiver = None;
        self.analyzer_fft_sender = None;
        self.current_analyzer_frame = None;

        if let Some(player) = &mut self.player {
            player.disable_analyzer();
        }
    }

    /// Toggle analyzer gradient mode
    pub fn toggle_analyzer_gradient(&mut self) {
        self.analyzer_gradient = !self.analyzer_gradient;
        self.set_status(format!(
            "Gradient Mode: {}",
            if self.analyzer_gradient { "ON" } else { "OFF" }
        ));
    }

    pub fn toggle_analyzer(&mut self, terminal_width: usize) {
        self.show_analyzer = !self.show_analyzer;

        if self.show_analyzer && self.analyzer_receiver.is_none() {
            self.start_analyzer(terminal_width);
        }
    }

    /// Update pitch semitones (called when pitch changes)
    /// Get the currently selected file
    pub fn selected_file(&self) -> Option<&AudioFile> {
        self.filtered_indices
            .get(self.selected)
            .and_then(|&idx| self.files.get(idx))
    }

    /// Move selection up
    pub fn move_up(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
            if self.autoplay {
                self.play_selected();
            } else {
                self.generate_waveform_for_selected();
            }
        }
    }

    /// Move selection down
    pub fn move_down(&mut self) {
        if self.selected < self.filtered_indices.len().saturating_sub(1) {
            self.selected += 1;
            if self.autoplay {
                self.play_selected();
            } else {
                self.generate_waveform_for_selected();
            }
        }
    }

    /// Move half page up
    pub fn move_half_page_up(&mut self, visible_height: usize) {
        let half = (visible_height / 2).max(1);
        self.selected = self.selected.saturating_sub(half);
        if self.autoplay {
            self.play_selected();
        } else {
            self.generate_waveform_for_selected();
        }
    }

    /// Move half page down
    pub fn move_half_page_down(&mut self, visible_height: usize) {
        let half = (visible_height / 2).max(1);
        let max = self.filtered_indices.len().saturating_sub(1);
        self.selected = (self.selected + half).min(max);
        if self.autoplay {
            self.play_selected();
        } else {
            self.generate_waveform_for_selected();
        }
    }

    /// Jump to top
    pub fn move_to_top(&mut self) {
        if !self.filtered_indices.is_empty() {
            self.selected = 0;
            if self.autoplay {
                self.play_selected();
            } else {
                self.generate_waveform_for_selected();
            }
        }
    }

    /// Jump to bottom
    pub fn move_to_bottom(&mut self) {
        if !self.filtered_indices.is_empty() {
            self.selected = self.filtered_indices.len().saturating_sub(1);
            if self.autoplay {
                self.play_selected();
            } else {
                self.generate_waveform_for_selected();
            }
        }
    }

    /// Shuffle the filtered results
    pub fn shuffle(&mut self) {
        use rand::seq::SliceRandom;
        let mut rng = rand::thread_rng();
        self.filtered_indices.shuffle(&mut rng);
        self.is_shuffled = true;
        self.selected = 0;
        self.scroll_offset = 0;
        if self.autoplay {
            self.play_selected();
        } else {
            self.generate_waveform_for_selected();
        }
    }

    /// Jump to a random result, or unshuffle if currently shuffled
    pub fn jump_to_random(&mut self) {
        if self.filtered_indices.is_empty() {
            return;
        }

        // If shuffled, unshuffle and keep current selection
        if self.is_shuffled {
            // Get currently selected file index
            let current_file_idx = self.filtered_indices.get(self.selected).copied();

            // Sort filtered indices back to natural order
            self.filtered_indices.sort_unstable();
            self.is_shuffled = false;

            // Find where the current file ended up after unshuffling
            if let Some(file_idx) = current_file_idx {
                if let Some(new_position) = self
                    .filtered_indices
                    .iter()
                    .position(|&idx| idx == file_idx)
                {
                    self.selected = new_position;
                }
            }

            self.scroll_offset = 0;
            if self.autoplay {
                self.play_selected();
            } else {
                self.generate_waveform_for_selected();
            }
        } else {
            // Not shuffled, jump to random
            use rand::Rng;
            let mut rng = rand::thread_rng();
            self.selected = rng.gen_range(0..self.filtered_indices.len());
            if self.autoplay {
                self.play_selected();
            } else {
                self.generate_waveform_for_selected();
            }
        }
    }

    /// Play the selected file
    pub fn play_selected(&mut self) {
        if let Some(file) = self.selected_file() {
            let path = file.path.clone();

            if let Some(player) = &self.player {
                if let Err(e) = player.play(path.clone()) {
                    self.set_status(format!("Failed to play: {}", e));
                    return;
                }

                // Load metadata for both selected and current
                let metadata = Metadata::from_file(&path).ok();
                self.selected_metadata = metadata.clone();
                self.current_metadata = metadata;

                // Generate waveform
                self.generate_waveform_for_file(&path);
            }
        }
    }

    /// Toggle play/pause
    pub fn toggle_play_pause(&mut self) {
        // If a different file is selected, play it
        if !self.is_selected_file_playing() {
            self.play_selected();
            return;
        }

        if let Some(player) = &self.player {
            if player.is_playing() || player.is_paused() {
                player.toggle_pause();
            } else if player.is_stopped() {
                self.play_selected();
            }
        }
    }

    /// Check if the selected file is the one currently playing
    fn is_selected_file_playing(&self) -> bool {
        if let Some(player) = &self.player {
            if let Some(current_file) = player.current_file() {
                if let Some(selected_file) = self.selected_file() {
                    return current_file == selected_file.path;
                }
            }
        }
        false
    }

    /// Stop playback
    pub fn stop(&mut self) {
        if let Some(player) = &self.player {
            player.stop();
        }
        self.current_metadata = None;
        // Fade-to-zero is now handled by poll_analyzer()
    }

    /// Volume up (1 dB)
    pub fn volume_up(&mut self) {
        self.volume_db = (self.volume_db + 1.0).min(12.0);
        self.update_volume();
    }

    /// Volume down (1 dB)
    pub fn volume_down(&mut self) {
        self.volume_db = (self.volume_db - 1.0).max(-60.0);
        self.update_volume();
    }

    /// Reset volume to 0 dB
    pub fn reset_volume(&mut self) {
        self.volume_db = 0.0;
        self.update_volume();
    }

    fn update_volume(&self) {
        if let Some(player) = &self.player {
            // Convert dB to linear: amplitude = 10^(dB/20)
            let mut amplitude = 10.0_f32.powf(self.volume_db / 20.0);

            // Apply normalization gain if enabled
            if self.normalize_waveform {
                amplitude *= self.normalize_gain;
            }

            player.set_volume(amplitude);
        }
    }

    /// Pitch up (1 semitone)
    pub fn pitch_up(&mut self) {
        self.pitch_semitones += 1.0;
        self.update_pitch();
    }

    /// Pitch down (1 semitone)
    pub fn pitch_down(&mut self) {
        self.pitch_semitones -= 1.0;
        self.update_pitch();
    }

    /// Pitch up (1 octave)
    pub fn pitch_up_octave(&mut self) {
        self.pitch_semitones += 12.0;
        self.update_pitch();
    }

    /// Pitch down (1 octave)
    pub fn pitch_down_octave(&mut self) {
        self.pitch_semitones -= 12.0;
        self.update_pitch();
    }

    /// Reset pitch to 0
    pub fn reset_pitch(&mut self) {
        self.pitch_semitones = 0.0;
        self.update_pitch();
    }

    fn update_pitch(&mut self) {
        if let Some(player) = &self.player {
            // Convert semitones to ratio: 2^(semitones/12)
            let ratio = 2.0_f32.powf(self.pitch_semitones / 12.0);

            // If playing or paused, restart playback immediately to apply pitch
            if player.is_playing() || player.is_paused() {
                let was_playing = player.is_playing();

                // Get current content position BEFORE pausing
                let content_position = player.get_position();

                // Pause to freeze position (prevents position from advancing during restart)
                if was_playing {
                    player.pause();
                }

                // Update pitch
                player.set_pitch(ratio);

                // Get original file duration and clamp to avoid seeking past the end
                let original_duration = if let Some(waveform) = &self.current_waveform {
                    waveform
                        .duration
                        .or_else(|| self.current_metadata.as_ref().and_then(|m| m.duration))
                } else if let Some(metadata) = &self.current_metadata {
                    metadata.duration
                } else {
                    None
                };

                let safe_position = if let Some(original_duration) = original_duration {
                    // Cap at 99% of original duration to avoid edge cases
                    let max_position =
                        Duration::from_secs_f32(original_duration.as_secs_f32() * 0.99);
                    if content_position > max_position {
                        max_position
                    } else {
                        content_position
                    }
                } else {
                    content_position
                };

                // Restart playback at the content position
                if let Some(file) = player.current_file() {
                    // Ignore errors during restart - just continue
                    let _ = player.play(file);
                    let _ = player.seek(safe_position);

                    // Resume if it was playing
                    if was_playing {
                        player.resume();
                    }
                }
            } else {
                // Not playing, just update pitch
                player.set_pitch(ratio);
            }
        }
    }

    /// Seek to next region (skip past silence), loop to start if at end
    pub fn seek_next_region(&mut self) {
        // Check if stopped before extracting references
        let was_stopped = !self.is_playing() && !self.is_paused();

        // Start playback if not playing
        if was_stopped {
            self.play_selected();
            // If it was stopped, just go to the beginning (first region)
            if let Some(player) = &self.player {
                let _ = player.seek(std::time::Duration::from_secs(0));
            }
            return;
        }

        let (waveform, metadata, player) =
            match (&self.current_waveform, &self.current_metadata, &self.player) {
                (Some(w), Some(m), Some(p)) => (w, m, p),
                _ => return,
            };

        let duration = match metadata.duration {
            Some(d) => d,
            None => return,
        };

        // Get current position as ratio (0.0 - 1.0)
        let current_pos = player.get_position();
        let current_ratio = current_pos.as_secs_f32() / duration.as_secs_f32();

        // Find next region
        let silence_threshold = 0.02;
        let next_ratio = waveform
            .find_next_region(current_ratio, silence_threshold)
            .unwrap_or(0.0); // Loop to beginning if no next region found

        let new_pos = std::time::Duration::from_secs_f32(next_ratio * duration.as_secs_f32());
        let _ = player.seek(new_pos);
    }

    /// Seek to previous region (skip back past silence), loop to end if at start
    pub fn seek_prev_region(&mut self) {
        // Check if stopped before extracting references
        let was_stopped = !self.is_playing() && !self.is_paused();

        // Start playback if not playing
        if was_stopped {
            self.play_selected();
            // If it was stopped, just go to the beginning (first region)
            if let Some(player) = &self.player {
                let _ = player.seek(std::time::Duration::from_secs(0));
            }
            return;
        }

        let (waveform, metadata, player) =
            match (&self.current_waveform, &self.current_metadata, &self.player) {
                (Some(w), Some(m), Some(p)) => (w, m, p),
                _ => return,
            };

        let duration = match metadata.duration {
            Some(d) => d,
            None => return,
        };

        // Get current position as ratio (0.0 - 1.0)
        let current_pos = player.get_position();
        let current_ratio = current_pos.as_secs_f32() / duration.as_secs_f32();

        // Find previous region
        let silence_threshold = 0.02;
        let prev_ratio =
            if let Some(prev) = waveform.find_previous_region(current_ratio, silence_threshold) {
                // If we got 0.0 and we're already near the start, loop to the last region
                if prev == 0.0 && current_ratio < 0.05 {
                    waveform.find_last_region(silence_threshold).unwrap_or(0.0)
                } else {
                    prev
                }
            } else {
                // Shouldn't happen, but loop to last region as fallback
                waveform.find_last_region(silence_threshold).unwrap_or(0.0)
            };

        let new_pos = std::time::Duration::from_secs_f32(prev_ratio * duration.as_secs_f32());
        let _ = player.seek(new_pos);
    }

    /// Check for silence and skip if in auto-skip mode
    /// Called periodically during playback (every ~16ms from UI loop)
    pub fn check_and_skip_silence(&mut self) {
        // Only check if enabled and playing
        if !self.skip_silence {
            return;
        }

        // Get required components
        let (waveform, metadata, player) =
            match (&self.current_waveform, &self.current_metadata, &self.player) {
                (Some(w), Some(m), Some(p)) => (w, m, p),
                _ => return,
            };

        // Only check during active playback
        if !player.is_playing() {
            return;
        }

        let duration = match metadata.duration {
            Some(d) => d,
            None => return,
        };

        // Throttle checks to every 100ms (avoid excessive seeking)
        if let Some(last_check) = self.last_silence_check {
            if last_check.elapsed() < std::time::Duration::from_millis(100) {
                return;
            }
        }
        self.last_silence_check = Some(std::time::Instant::now());

        // Get current position as ratio (0.0 - 1.0)
        let current_pos = player.get_position();
        let current_ratio = current_pos.as_secs_f32() / duration.as_secs_f32();

        // Check if current position is in silence
        let silence_threshold = 0.02;
        if waveform.is_silent_at(current_ratio, silence_threshold) {
            // Find next sound region immediately
            if let Some(next_ratio) =
                waveform.find_immediate_next_sound(current_ratio, silence_threshold)
            {
                // Don't seek past 99% to avoid end-of-file issues
                if next_ratio < 0.99 {
                    let new_pos =
                        std::time::Duration::from_secs_f32(next_ratio * duration.as_secs_f32());
                    let _ = player.seek(new_pos);
                    // Reset check timer after seeking
                    self.last_silence_check = Some(std::time::Instant::now());
                }
            }
        }
    }

    /// Toggle autoplay mode
    pub fn toggle_autoplay(&mut self) {
        self.autoplay = !self.autoplay;
    }

    /// Toggle info panel display
    pub fn toggle_info(&mut self) {
        self.show_info = !self.show_info;
    }

    /// Toggle waveform normalization
    pub fn toggle_normalize(&mut self) {
        self.normalize_waveform = !self.normalize_waveform;
        // Update volume to apply or remove normalization gain
        self.update_volume();
    }

    /// Toggle skip silence mode
    pub fn toggle_skip_silence(&mut self) {
        self.skip_silence = !self.skip_silence;
        self.set_status(format!(
            "Skip Silence: {}",
            if self.skip_silence { "ON" } else { "OFF" }
        ));
    }

    /// Add a character to the search query
    pub fn search_push_char(&mut self, c: char) {
        if self.search_query.is_empty() {
            self.selected = 0;
        }
        self.search_query.push(c);
        self.update_filter();
    }

    /// Remove the last character from the search query
    pub fn search_pop_char(&mut self) {
        self.search_query.pop();
        self.update_filter();
    }

    /// Clear the search query
    pub fn clear_search(&mut self) {
        self.search_query.clear();
        self.filtered_indices = (0..self.files.len()).collect();
        self.selected = 0;
        self.scroll_offset = 0;
        self.is_shuffled = false;
    }

    /// Delete last word from search
    pub fn search_delete_word(&mut self) {
        let trimmed = self.search_query.trim_end();
        if let Some(last_space) = trimmed.rfind(char::is_whitespace) {
            self.search_query.truncate(last_space + 1);
        } else {
            self.search_query.clear();
        }
        self.update_filter();
    }

    /// Update the filtered file list based on search query
    /// Supports boolean search:
    /// - Commas for OR: bird,dog,cat matches bird OR dog OR cat
    /// - Hyphens for NOT: -pig excludes pig
    /// - Quotes for exact phrases: "big bird" matches exactly
    fn update_filter(&mut self) {
        if self.search_query.is_empty() {
            self.filtered_indices = (0..self.files.len()).collect();
            self.previous_query.clear();
        } else {
            let query_lower = self.search_query.to_lowercase();

            // Parse the query into search terms
            let parsed = Self::parse_search_query(&query_lower);

            // Helper to check if a file matches the search criteria
            let matches_query = |file: &AudioFile| {
                let filename_lower = file.filename.to_lowercase();
                let desc_lower = file
                    .description
                    .as_ref()
                    .map(|d| d.to_lowercase())
                    .unwrap_or_default();
                let searchable = format!("{} {}", filename_lower, desc_lower);

                // Check exclusions first - if any exclusion matches, reject
                for excl in &parsed.exclusions {
                    if searchable.contains(excl.as_str()) {
                        return false;
                    }
                }

                // If no OR groups, match everything (after exclusions)
                if parsed.or_groups.is_empty() {
                    return true;
                }

                // Check OR groups - at least one group must match
                // Within each group, ALL terms must match (AND)
                parsed
                    .or_groups
                    .iter()
                    .any(|group| group.iter().all(|term| searchable.contains(term.as_str())))
            };

            // Always do full search for boolean queries (incremental is complex)
            self.filtered_indices = self
                .files
                .iter()
                .enumerate()
                .filter(|(_, file)| matches_query(file))
                .map(|(i, _)| i)
                .collect();

            self.previous_query = self.search_query.clone();
        }

        // Reset selection if out of bounds
        if self.selected >= self.filtered_indices.len() {
            self.selected = self.filtered_indices.len().saturating_sub(1);
        }
        self.scroll_offset = 0;
        self.is_shuffled = false; // Reset shuffle state when filter changes

        self.generate_waveform_for_selected();
    }

    /// Parse a search query into boolean terms
    /// - Commas separate OR groups
    /// - Spaces within groups are AND
    /// - Leading hyphen is exclusion
    /// - Quotes preserve spaces as exact phrase
    fn parse_search_query(query: &str) -> ParsedQuery {
        let mut or_groups: Vec<Vec<String>> = Vec::new();
        let mut exclusions: Vec<String> = Vec::new();

        // Split by comma for OR groups
        for group_str in query.split(',') {
            let group_str = group_str.trim();
            if group_str.is_empty() {
                continue;
            }

            let mut and_terms: Vec<String> = Vec::new();

            // Parse terms handling quotes
            let chars = group_str.chars().peekable();
            let mut current_term = String::new();
            let mut in_quotes = false;

            for c in chars {
                match c {
                    '"' => {
                        in_quotes = !in_quotes;
                    }
                    ' ' if !in_quotes => {
                        if !current_term.is_empty() {
                            // Check if exclusion
                            if current_term.starts_with('-') && current_term.len() > 1 {
                                exclusions.push(current_term[1..].to_string());
                            } else if !current_term.starts_with('-') {
                                and_terms.push(current_term.clone());
                            }
                            current_term.clear();
                        }
                    }
                    _ => {
                        current_term.push(c);
                    }
                }
            }

            // Handle last term
            if !current_term.is_empty() {
                if current_term.starts_with('-') && current_term.len() > 1 {
                    exclusions.push(current_term[1..].to_string());
                } else if !current_term.starts_with('-') {
                    and_terms.push(current_term);
                }
            }

            if !and_terms.is_empty() {
                or_groups.push(and_terms);
            }
        }

        ParsedQuery {
            or_groups,
            exclusions,
        }
    }

    /// Generate waveform and load metadata for the selected file
    fn generate_waveform_for_selected(&mut self) {
        if let Some(file) = self.selected_file() {
            let path = file.path.clone();

            // Always load metadata for the selected file (for info panel)
            self.selected_metadata = Metadata::from_file(&path).ok();

            // Don't update waveform if already playing
            let is_playing_or_paused = if let Some(player) = &self.player {
                player.is_playing() || player.is_paused()
            } else {
                false
            };

            if is_playing_or_paused {
                return;
            }

            // Load metadata for stopped file
            self.current_metadata = self.selected_metadata.clone();

            // Generate waveform if enabled
            if self.show_waveform {
                self.generate_waveform_for_file(&path);
            }
        }
    }

    fn generate_waveform_for_file(&mut self, path: &PathBuf) {
        // Try cache first
        if let Some(cached) = WaveformData::load_from_cache(path) {
            // Calculate normalization gain from cached waveform (-1 dB target)
            // -1 dB = 10^(-1/20) ≈ 0.8913
            let target_db = -1.0;
            let target_amplitude = 10.0_f32.powf(target_db / 20.0);
            self.normalize_gain = cached.calculate_normalize_gain(target_amplitude);
            self.current_waveform = Some(cached);
            // Update volume to apply normalization if enabled
            self.update_volume();
            return;
        }

        // Generate in background
        let path_clone = path.clone();
        let sender = self.waveform_sender.clone();
        std::thread::spawn(move || {
            let start = Instant::now();
            if let Ok(waveform) =
                WaveformData::generate_with_cache(&path_clone, 400, &sender, false)
            {
                let elapsed = start.elapsed();
                let _ = sender.send((path_clone, waveform, elapsed));
            }
        });
    }

    /// Adjust scroll offset to keep selection visible
    pub fn adjust_scroll(&mut self, visible_height: usize) {
        if visible_height == 0 {
            return;
        }

        let threshold = (visible_height as f32 * 0.2).ceil() as usize;
        let relative_pos = self.selected.saturating_sub(self.scroll_offset);

        if relative_pos < threshold {
            self.scroll_offset = self.selected.saturating_sub(threshold);
        }

        if relative_pos >= visible_height.saturating_sub(threshold) {
            self.scroll_offset = self.selected.saturating_sub(visible_height - threshold - 1);
        }

        let max_offset = self.filtered_indices.len().saturating_sub(visible_height);
        self.scroll_offset = self.scroll_offset.min(max_offset);
    }

    /// Set a status message
    pub fn set_status(&mut self, message: String) {
        self.status_message = Some(message);
        self.status_time = Some(Instant::now());
    }

    /// Clear expired status message
    pub fn clear_expired_status(&mut self) {
        if let Some(time) = self.status_time {
            if time.elapsed() >= Duration::from_millis(2000) {
                self.status_message = None;
                self.status_time = None;
            }
        }
    }

    /// Get playback position as ratio (0.0-1.0)
    pub fn get_progress(&self) -> f32 {
        if let (Some(player), Some(metadata)) = (&self.player, &self.current_metadata) {
            if let Some(duration) = metadata.duration {
                return player.progress(duration);
            }
        }
        0.0
    }

    /// Check if currently playing
    pub fn is_playing(&self) -> bool {
        self.player
            .as_ref()
            .map(|p| p.is_playing())
            .unwrap_or(false)
    }

    /// Check if paused
    pub fn is_paused(&self) -> bool {
        self.player.as_ref().map(|p| p.is_paused()).unwrap_or(false)
    }
}

/// Audio file extensions we support
const AUDIO_EXTENSIONS: &[&str] = &[
    "wav", "mp3", "flac", "ogg", "m4a", "aac", "aiff", "aif", "wma", "opus",
];

/// Check if a path is an audio file
fn is_audio_file(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|ext| AUDIO_EXTENSIONS.contains(&ext.to_lowercase().as_str()))
        .unwrap_or(false)
}

/// Scan a directory recursively for audio files using parallel walking
fn scan_audio_files(root: &PathBuf, sender: Sender<AudioFile>) {
    use ignore::{WalkBuilder, WalkState};
    use std::sync::Arc;

    let mut builder = WalkBuilder::new(root);
    builder
        .hidden(true) // Skip hidden files/dirs
        .git_ignore(false)
        .git_global(false)
        .git_exclude(false);

    // Use available CPU cores for parallel I/O
    let num_threads = std::thread::available_parallelism()
        .map(|p| p.get())
        .unwrap_or(4);
    builder.threads(num_threads);

    let parallel_walker = builder.build_parallel();
    let sender = Arc::new(sender);

    parallel_walker.run(|| {
        let sender = Arc::clone(&sender);

        Box::new(move |result| {
            let entry = match result {
                Ok(e) => e,
                Err(_) => return WalkState::Continue,
            };

            let path = entry.path();

            // Skip directories (use cached file_type to avoid extra syscall)
            let is_dir = entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false);
            if is_dir {
                return WalkState::Continue;
            }

            // Get filename and skip hidden files
            let filename = match path.file_name().and_then(|n| n.to_str()) {
                Some(name) if !name.starts_with('.') => name.to_string(),
                _ => return WalkState::Continue,
            };

            // Check if it's an audio file
            if !is_audio_file(path) {
                return WalkState::Continue;
            }

            let audio_file = AudioFile {
                path: path.to_path_buf(),
                filename,
                description: None,
            };

            if sender.send(audio_file).is_err() {
                return WalkState::Quit;
            }

            WalkState::Continue
        })
    });
}

/// Build database in background without sending to UI
/// Sends progress updates every 100 files through the status_sender
/// Uses parallel processing for metadata extraction
fn build_database_silent(
    root: &PathBuf,
    status_sender: Sender<String>,
) -> Result<(std::time::Duration, usize), String> {
    use crate::core::audio::{AudioFileRecord, MinimalMetadata};
    use ignore::WalkBuilder;
    use std::fs;
    use std::sync::mpsc::channel;
    use std::time::Instant;

    let start_time = Instant::now();

    // Open/create database
    let db = match get_db_path(root) {
        Some(db_path) => {
            if let Some(parent) = db_path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            match Database::open(&db_path) {
                Ok(db) => {
                    let _ = db.set_pragmas();
                    let _ = db.rebuild_database();
                    Some(db)
                }
                Err(_) => return Err("Failed to open database".into()),
            }
        }
        None => return Err("No database path".into()),
    };

    if let Some(ref db) = db {
        let _ = db.begin_transaction();
    }

    // Use parallel walker for faster file discovery and metadata extraction
    let (record_sender, record_receiver) = channel::<AudioFileRecord>();

    // Spawn walker thread that processes files in parallel
    let root_clone = root.clone();
    let walker_handle = std::thread::spawn(move || {
        let walker = WalkBuilder::new(&root_clone)
            .hidden(true)
            .git_ignore(false)
            .git_global(false)
            .git_exclude(false)
            .threads(8) // Use 8 parallel threads for processing
            .build_parallel();

        walker.run(|| {
            let tx = record_sender.clone();
            Box::new(move |entry| {
                use ignore::WalkState;

                let entry = match entry {
                    Ok(e) => e,
                    Err(_) => return WalkState::Continue,
                };

                let path = entry.path();

                // Skip directories
                if entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
                    return WalkState::Continue;
                }

                // Get filename and skip hidden files
                let filename = match path.file_name().and_then(|n| n.to_str()) {
                    Some(name) if !name.starts_with('.') => name.to_string(),
                    _ => return WalkState::Continue,
                };

                // Skip non-audio files
                if !is_audio_file(path) {
                    return WalkState::Continue;
                }

                // Extract metadata (WITH our WAV fast path optimization!)
                let metadata = MinimalMetadata::from_file(path).ok();
                let description = metadata.as_ref().and_then(|m| m.bwf_description.clone());

                let record = AudioFileRecord {
                    file_path: path.to_string_lossy().to_string(),
                    file_name: filename,
                    description,
                    sample_rate: metadata
                        .as_ref()
                        .and_then(|m| m.sample_rate)
                        .map(|s| s as i32),
                    channels: metadata.as_ref().and_then(|m| m.channels).map(|c| c as i32),
                };

                // Send to database inserter
                if tx.send(record).is_err() {
                    return WalkState::Quit;
                }

                WalkState::Continue
            })
        });
    });

    // Collect and batch insert records
    let mut file_count = 0;
    let mut batch: Vec<AudioFileRecord> = Vec::with_capacity(500);

    while let Ok(record) = record_receiver.recv() {
        batch.push(record);
        file_count += 1;

        // Send progress update every 100 files
        if file_count % 100 == 0 {
            let _ = status_sender.send(format!("PROGRESS:{}", file_count));
        }

        // Insert and commit every 500 files
        if batch.len() >= 500 {
            if let Some(ref db) = db {
                let _ = db.insert_files_batch(&batch);
                batch.clear();
                let _ = db.commit_transaction();
                let _ = db.begin_transaction();
            }
        }
    }

    // Wait for walker to finish
    let _ = walker_handle.join();

    // Insert remaining files and final commit
    if let Some(ref db) = db {
        if !batch.is_empty() {
            let _ = db.insert_files_batch(&batch);
        }
        let _ = db.commit_transaction();
        let _ = db.mark_complete();
    }

    Ok((start_time.elapsed(), file_count))
}
