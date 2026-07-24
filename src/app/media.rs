//! Media file handling: ffmpeg editor and audio playback

use std::io;
use std::path::PathBuf;
use std::process::Command;

use crossterm::execute;
use crossterm::terminal::{Clear, ClearType};

use super::App;

/// Media file info from ffprobe
pub(super) struct MediaFileInfo {
    pub duration: String,
    pub is_video: bool,
    pub has_audio: bool,
    pub fps: Option<String>,
}

impl App {
    /// Check if a file is a media file (audio or video)
    pub(super) fn is_media_file(path: &PathBuf) -> bool {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            .unwrap_or_default();

        matches!(
            ext.as_str(),
            // Video formats
            "mp4" | "mkv" | "avi" | "mov" | "wmv" | "flv" | "webm" | "m4v" | "mpeg" | "mpg" |
            // Audio formats
            "mp3" | "wav" | "flac" | "aac" | "ogg" | "m4a" | "wma" | "opus" | "aiff"
        )
    }

    /// Check if file extension is audio-only (no video possible)
    fn is_audio_only_ext(path: &PathBuf) -> bool {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            .unwrap_or_default();

        matches!(
            ext.as_str(),
            "mp3" | "wav" | "flac" | "aac" | "ogg" | "m4a" | "wma" | "opus" | "aiff"
        )
    }

    /// Get media file info using ffprobe (duration, fps, is_video, has_audio)
    fn get_media_info(path: &PathBuf) -> MediaFileInfo {
        let mut info = MediaFileInfo {
            duration: "unknown".to_string(),
            is_video: false,
            has_audio: false,
            fps: None,
        };

        // Audio-only formats always have audio, never video
        let audio_only = Self::is_audio_only_ext(path);
        if audio_only {
            info.has_audio = true;
        }

        // Get duration
        if let Ok(output) = Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-show_entries",
                "format=duration",
                "-of",
                "default=noprint_wrappers=1:nokey=1",
                "-sexagesimal",
            ])
            .arg(path)
            .output()
        {
            if output.status.success() {
                let duration = String::from_utf8_lossy(&output.stdout).trim().to_string();
                info.duration = duration.split('.').next().unwrap_or(&duration).to_string();
            }
        }

        // Skip video detection for audio-only formats
        if audio_only {
            return info;
        }

        // Check if file has a video stream by looking for video codec
        if let Ok(output) = Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-select_streams",
                "v:0",
                "-show_entries",
                "stream=codec_type",
                "-of",
                "default=noprint_wrappers=1:nokey=1",
            ])
            .arg(path)
            .output()
        {
            if output.status.success() {
                let codec_type = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if codec_type == "video" {
                    info.is_video = true;

                    // Now get the fps
                    if let Ok(fps_output) = Command::new("ffprobe")
                        .args([
                            "-v",
                            "error",
                            "-select_streams",
                            "v:0",
                            "-show_entries",
                            "stream=r_frame_rate",
                            "-of",
                            "default=noprint_wrappers=1:nokey=1",
                        ])
                        .arg(path)
                        .output()
                    {
                        if fps_output.status.success() {
                            let fps_str = String::from_utf8_lossy(&fps_output.stdout)
                                .trim()
                                .to_string();
                            // Parse fraction like "30000/1001" or "30/1"
                            if let Some(pos) = fps_str.find('/') {
                                if let (Ok(num), Ok(den)) = (
                                    fps_str[..pos].parse::<f64>(),
                                    fps_str[pos + 1..].parse::<f64>(),
                                ) {
                                    if den != 0.0 {
                                        let fps = num / den;
                                        // Round to common values
                                        let rounded = if (fps - 23.976).abs() < 0.01 {
                                            "23.976".to_string()
                                        } else if (fps - 29.97).abs() < 0.01 {
                                            "29.97".to_string()
                                        } else if (fps - 59.94).abs() < 0.01 {
                                            "59.94".to_string()
                                        } else {
                                            format!("{:.2}", fps)
                                        };
                                        info.fps = Some(rounded);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Check if file has an audio stream
        if let Ok(output) = Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-select_streams",
                "a:0",
                "-show_entries",
                "stream=codec_type",
                "-of",
                "default=noprint_wrappers=1:nokey=1",
            ])
            .arg(path)
            .output()
        {
            if output.status.success() {
                let codec_type = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if codec_type == "audio" {
                    info.has_audio = true;
                }
            }
        }

        info
    }

    /// Parse duration string and return formatted start/end times
    /// Returns (start, end) in appropriate format based on duration
    fn format_times_for_duration(duration: &str) -> (String, String) {
        // Parse duration format "H:MM:SS" or "M:SS" or "SS"
        let parts: Vec<&str> = duration.split(':').collect();

        let total_seconds = match parts.len() {
            3 => {
                // H:MM:SS format
                let hours: u32 = parts[0].parse().unwrap_or(0);
                let minutes: u32 = parts[1].parse().unwrap_or(0);
                let seconds: u32 = parts[2].parse().unwrap_or(0);
                hours * 3600 + minutes * 60 + seconds
            }
            2 => {
                // MM:SS format
                let minutes: u32 = parts[0].parse().unwrap_or(0);
                let seconds: u32 = parts[1].parse().unwrap_or(0);
                minutes * 60 + seconds
            }
            1 => {
                // SS format
                parts[0].parse().unwrap_or(0)
            }
            _ => 0,
        };

        if total_seconds < 60 {
            // Less than 1 minute: show 0 and SS (ffmpeg accepts plain seconds)
            ("0".to_string(), format!("{}", total_seconds))
        } else if total_seconds < 3600 {
            // Less than 1 hour: show MM:SS format
            let minutes = total_seconds / 60;
            let seconds = total_seconds % 60;
            (
                "00:00".to_string(),
                format!("{:02}:{:02}", minutes, seconds),
            )
        } else {
            // 1 hour or more: show full H:MM:SS
            let hours = total_seconds / 3600;
            let minutes = (total_seconds % 3600) / 60;
            let seconds = total_seconds % 60;
            (
                "0:00:00".to_string(),
                format!("{}:{:02}:{:02}", hours, minutes, seconds),
            )
        }
    }

    /// Open ffmpeg editor - creates YAML template in nvim
    /// Returns true if an external program was opened
    pub fn open_ffmpeg_editor(&mut self) -> io::Result<bool> {
        use serde::Deserialize;
        use std::fs;

        let Some(path) = self.current.selected_path() else {
            self.set_status("No file selected");
            return Ok(false);
        };

        if !Self::is_media_file(&path) {
            self.set_status("Not a media file");
            return Ok(false);
        }

        // Get media info via ffprobe
        let media_info = Self::get_media_info(&path);

        // Get file name and generate default output name
        let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("output");
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("mp4");
        let default_output = format!("{}_edit.{}", stem, ext);

        // Format times based on duration
        let (start_time, end_time) = Self::format_times_for_duration(&media_info.duration);

        // Create temp YAML with template
        let temp_path = std::env::temp_dir().join(crate::paths::FFMPEG_EDIT_FILE_NAME);

        // Build streams info comment
        let streams_info = match (media_info.is_video, media_info.has_audio) {
            (true, true) => "# streams: video + audio",
            (true, false) => "# streams: video only (no audio)",
            (false, true) => "# streams: audio only",
            (false, false) => "# streams: unknown",
        };

        // Build YAML content - include fps for video, extract only if both streams exist
        let yaml_content = if media_info.is_video {
            let fps_str = media_info.fps.as_deref().unwrap_or("30");
            // Only show extract option if file has both video and audio
            let extract_section = if media_info.has_audio {
                "\n# extract: \"video\" or \"audio\" to split streams (removes the other)\nextract: ~\n"
            } else {
                ""
            };
            format!(
                r#"# FFmpeg Editor - save to execute, quit without saving to cancel
# Only changed values will be applied
{}

source: "{}"
duration: "{}"  # read-only

start: "{}"
end: "{}"
fps: {}
{}
output: "{}"
"#,
                streams_info,
                filename,
                media_info.duration,
                start_time,
                end_time,
                fps_str,
                extract_section,
                default_output
            )
        } else {
            format!(
                r#"# FFmpeg Editor - save to execute, quit without saving to cancel
# Only changed values will be applied
{}

source: "{}"
duration: "{}"  # read-only

start: "{}"
end: "{}"

output: "{}"
"#,
                streams_info, filename, media_info.duration, start_time, end_time, default_output
            )
        };

        fs::write(&temp_path, &yaml_content)?;

        // Store original values to detect changes
        let orig_start = start_time.clone();
        let orig_end = end_time.clone();
        let orig_fps = media_info.fps.clone();

        // Record mtime before editing
        let before_mtime = fs::metadata(&temp_path)?.modified()?;

        // Restore terminal and open in nvim
        ratatui::restore();
        let _ = execute!(io::stdout(), Clear(ClearType::All));

        let status = Command::new("nvim")
            .arg("+7") // Position cursor at start line
            .arg(&temp_path)
            .current_dir(&self.cwd)
            .status();

        self.needs_reinit = true;

        if status.is_err() {
            let _ = fs::remove_file(&temp_path);
            self.set_status("Failed to open editor");
            return Ok(true);
        }

        // Check if file was saved (mtime changed)
        let after_mtime = fs::metadata(&temp_path)
            .and_then(|m| m.modified())
            .unwrap_or(before_mtime);

        if after_mtime > before_mtime {
            // Parse YAML and execute operations
            let yaml_str = fs::read_to_string(&temp_path)?;
            let _ = fs::remove_file(&temp_path);

            // Deserialize types
            #[derive(Deserialize)]
            struct FfmpegConfig {
                output: String,
                source: String,
                start: String,
                end: String,
                fps: Option<f64>,
                extract: Option<String>, // "video" or "audio" to extract single stream
            }

            match serde_yaml::from_str::<FfmpegConfig>(&yaml_str) {
                Ok(config) => {
                    let source_path = self.cwd.join(&config.source);
                    let output_path = self.cwd.join(&config.output);

                    // Check what operations are needed
                    let trim_changed = config.start != orig_start || config.end != orig_end;
                    let fps_changed =
                        if let (Some(new_fps), Some(ref orig)) = (config.fps, &orig_fps) {
                            let orig_val = orig.parse::<f64>().unwrap_or(0.0);
                            (new_fps - orig_val).abs() > 0.001
                        } else {
                            config.fps.is_some() && orig_fps.is_none()
                        };

                    // Check if extracting a specific stream
                    let extract_video = config.extract.as_deref() == Some("video");
                    let extract_audio = config.extract.as_deref() == Some("audio");
                    let extracting = extract_video || extract_audio;

                    // Determine if format conversion is needed
                    let source_ext = source_path
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("")
                        .to_lowercase();
                    let output_ext = output_path
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("")
                        .to_lowercase();
                    let format_changed = source_ext != output_ext;

                    // If nothing changed, just copy or do nothing
                    if !trim_changed && !fps_changed && !format_changed && !extracting {
                        self.set_status("No changes to apply");
                        return Ok(true);
                    }

                    if output_path.exists() {
                        self.set_status(format!("Output exists: {}", config.output));
                        return Ok(true);
                    }

                    // Restore terminal for ffmpeg output
                    ratatui::restore();
                    let _ = execute!(io::stdout(), Clear(ClearType::All));

                    // Build ffmpeg command
                    let mut args =
                        vec!["-i".to_string(), source_path.to_string_lossy().to_string()];

                    // Add trim args only if times changed
                    if trim_changed {
                        args.push("-ss".to_string());
                        args.push(config.start.clone());
                        args.push("-to".to_string());
                        args.push(config.end.clone());
                    }

                    // Handle stream extraction
                    if extract_video {
                        args.push("-an".to_string()); // Remove audio
                    } else if extract_audio {
                        args.push("-vn".to_string()); // Remove video
                    }

                    // Determine codec settings
                    let needs_reencode = fps_changed || format_changed;

                    if needs_reencode {
                        // Need to re-encode
                        if fps_changed {
                            if let Some(new_fps) = config.fps {
                                args.push("-r".to_string());
                                args.push(format!("{}", new_fps));
                            }
                        }
                        // Let ffmpeg choose appropriate codecs for format
                    } else if !extracting || (extract_audio && !format_changed) {
                        // Fast stream copy when no re-encoding needed
                        // For audio extraction without format change, we can copy
                        args.push("-c".to_string());
                        args.push("copy".to_string());
                    }
                    // Note: extracting video may still need copy if format supports it

                    args.push(output_path.to_string_lossy().to_string());

                    let result = Command::new("ffmpeg")
                        .args(&args)
                        .current_dir(&self.cwd)
                        .status();

                    self.refresh_current_dir()?;

                    if let Ok(exit_status) = result {
                        if exit_status.success() {
                            // Build status message showing what was done
                            let mut ops = Vec::new();
                            if trim_changed {
                                ops.push("trimmed");
                            }
                            if fps_changed {
                                ops.push("fps changed");
                            }
                            if format_changed {
                                ops.push("converted");
                            }
                            if extract_video {
                                ops.push("video extracted");
                            }
                            if extract_audio {
                                ops.push("audio extracted");
                            }
                            let ops_str = ops.join(", ");
                            self.set_status(format!("Created: {} ({})", config.output, ops_str));
                        } else {
                            self.set_status("ffmpeg failed".to_string());
                        }
                    } else {
                        self.set_status("ffmpeg command failed".to_string());
                    }
                }
                Err(e) => {
                    self.set_status(format!("YAML error: {}", e));
                }
            }
        } else {
            // File wasn't saved - cancelled
            let _ = fs::remove_file(&temp_path);
            self.set_status("Cancelled");
        }

        Ok(true)
    }

    // Audio playback methods

    fn ensure_audio_player(&mut self) -> bool {
        if self.audio_player.is_none() {
            self.audio_player = crate::core::player::AudioPlayer::new();
        }
        self.audio_player.is_some()
    }

    pub fn toggle_audio(&mut self) {
        self.ensure_audio_player();
        let Some(player) = self.audio_player.as_mut() else {
            self.set_status("Audio not available");
            return;
        };

        // If already playing/paused the same file, toggle pause
        if player.current_file().is_some() {
            if player.is_playing() {
                player.toggle_pause();
                self.audio_auto_play = false;
                self.set_status("Paused");
            } else if player.is_paused() {
                player.toggle_pause();
                self.audio_auto_play = true;
                self.set_status("Playing");
            } else {
                // File finished - try to play selected file
                self.play_selected_audio();
            }
        } else {
            // Nothing playing - play selected file
            self.play_selected_audio();
        }
    }

    fn play_selected_audio(&mut self) {
        let Some(path) = self.current.selected_path() else {
            self.set_status("No file selected");
            return;
        };

        if !path.is_file() {
            self.set_status("Not a file");
            return;
        }

        // Check if it's an audio file by extension
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        let audio_exts = [
            "mp3", "wav", "flac", "ogg", "m4a", "aac", "opus", "aiff", "wma",
        ];
        if !audio_exts.contains(&ext.as_str()) {
            self.set_status("Not an audio file");
            return;
        }

        self.ensure_audio_player();
        let Some(player) = self.audio_player.as_mut() else {
            self.set_status("Audio not available");
            return;
        };

        let filename = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        match player.play(path) {
            Ok(()) => {
                self.audio_auto_play = true;
                self.set_status(format!("Playing: {}", filename));
            }
            Err(e) => self.set_status(format!("Error: {}", e)),
        }
    }

    /// Play a specific audio file (used for auto-play)
    pub(super) fn play_audio_file(&mut self, path: &std::path::Path) {
        self.ensure_audio_player();
        let Some(player) = self.audio_player.as_mut() else {
            return;
        };

        let filename = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        if player.play(path.to_path_buf()).is_ok() {
            self.set_status(format!("Playing: {}", filename));
        }
    }

    pub fn stop_audio(&mut self) {
        if let Some(player) = self.audio_player.as_mut() {
            player.stop();
            self.audio_auto_play = false;
            self.set_status("Stopped");
        }
    }

    /// Stop audio without showing a status message (for selection changes)
    pub(super) fn stop_audio_silent(&mut self) {
        if let Some(player) = self.audio_player.as_mut() {
            if player.is_playing() || player.is_paused() {
                player.stop();
                // Clear the "Playing:" status immediately
                self.status.clear();
                self.status_time = None;
            }
        }
    }

    pub fn is_audio_playing(&self) -> bool {
        self.audio_player
            .as_ref()
            .map(|p| p.is_playing())
            .unwrap_or(false)
    }

    pub fn is_audio_active(&self) -> bool {
        self.audio_player
            .as_ref()
            .map(|p| p.is_playing() || p.is_paused())
            .unwrap_or(false)
    }

    /// Toggle audio playback from Find mode (uses find_state's selected path)
    pub fn toggle_audio_from_find(&mut self) {
        self.ensure_audio_player();
        let Some(player) = self.audio_player.as_mut() else {
            self.set_status("Audio not available");
            return;
        };

        // If already playing/paused, toggle pause
        if player.current_file().is_some() {
            if player.is_playing() {
                player.toggle_pause();
                self.set_status("Paused");
            } else if player.is_paused() {
                player.toggle_pause();
                self.set_status("Playing");
            } else {
                self.play_audio_from_find();
            }
        } else {
            self.play_audio_from_find();
        }
    }

    fn play_audio_from_find(&mut self) {
        let Some(path) = self
            .find_state
            .as_ref()
            .and_then(|s| s.selected_path().cloned())
        else {
            self.set_status("No file selected");
            return;
        };

        if !path.is_file() {
            self.set_status("Not a file");
            return;
        }

        // Check if it's an audio file by extension
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        let audio_exts = [
            "mp3", "wav", "flac", "ogg", "m4a", "aac", "opus", "aiff", "wma",
        ];
        if !audio_exts.contains(&ext.as_str()) {
            self.set_status("Not an audio file");
            return;
        }

        self.ensure_audio_player();
        let Some(player) = self.audio_player.as_mut() else {
            self.set_status("Audio not available");
            return;
        };

        let filename = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        match player.play(path) {
            Ok(()) => self.set_status(format!("Playing: {}", filename)),
            Err(e) => self.set_status(format!("Error: {}", e)),
        }
    }
}
