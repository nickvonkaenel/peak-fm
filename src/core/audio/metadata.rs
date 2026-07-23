use lofty::file::AudioFile as LoftyAudioFile;
use lofty::probe::Probe;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::time::Duration;

#[derive(Clone, Debug, Default)]
pub struct Metadata {
    pub duration: Option<Duration>,
    pub sample_rate: Option<u32>,
    pub channels: Option<u8>,
    pub bwf_description: Option<String>,
}

#[derive(Clone, Debug)]
pub struct MinimalMetadata {
    pub sample_rate: Option<u32>,
    pub channels: Option<u8>,
    pub bwf_description: Option<String>,
}

impl Metadata {
    /// Sanitize a string by removing control characters and invalid UTF-8
    fn sanitize_string(s: &str) -> String {
        s.chars()
            .filter(|c| {
                // Keep printable ASCII, spaces, tabs, newlines, and valid Unicode
                c.is_ascii_graphic()
                    || c.is_ascii_whitespace()
                    || (!c.is_control() && !c.is_ascii())
            })
            .collect()
    }

    pub fn from_file(path: &Path) -> Result<Self, String> {
        // Try lofty first for full metadata (tags + properties)
        let result = Probe::open(path).and_then(|p| p.read());

        match result {
            Ok(tagged_file) => {
                let properties = tagged_file.properties();
                let duration = Some(properties.duration());
                let sample_rate = properties.sample_rate();
                let channels = properties.channels();

                // Try to parse BWF metadata for WAV files
                let bwf_description = if path.extension().and_then(|e| e.to_str()) == Some("wav") {
                    Self::parse_bwf_description(path).unwrap_or(None)
                } else {
                    None
                };

                Ok(Self {
                    duration,
                    sample_rate,
                    channels,
                    bwf_description,
                })
            }
            Err(_) => {
                // If lofty fails, try symphonia as fallback for basic properties
                Self::from_symphonia_full(path)
            }
        }
    }

    /// Fallback metadata extraction using symphonia decoder
    /// Returns basic metadata (no tags, just technical properties)
    fn from_symphonia_full(path: &Path) -> Result<Self, String> {
        use symphonia::core::codecs::CODEC_TYPE_NULL;
        use symphonia::core::formats::FormatOptions;
        use symphonia::core::io::MediaSourceStream;
        use symphonia::core::meta::MetadataOptions;
        use symphonia::core::probe::Hint;

        let file = File::open(path).map_err(|e| format!("Failed to open file: {}", e))?;
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

        let format = probed.format;
        let track = format
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
            .ok_or_else(|| "No suitable audio track found".to_string())?;

        // Calculate duration from track parameters if available
        let duration = if let (Some(n_frames), Some(sample_rate)) =
            (track.codec_params.n_frames, track.codec_params.sample_rate)
        {
            if sample_rate > 0 {
                let duration_secs = n_frames as f64 / sample_rate as f64;
                Some(Duration::from_secs_f64(duration_secs))
            } else {
                None
            }
        } else if let Some(time_base) = track.codec_params.time_base {
            // Try using time_base if n_frames not available
            if let Some(n_frames) = track.codec_params.n_frames {
                let duration_secs = time_base.calc_time(n_frames).seconds as f64
                    + time_base.calc_time(n_frames).frac;
                Some(Duration::from_secs_f64(duration_secs))
            } else {
                None
            }
        } else {
            None
        };

        // Try to get BWF if it's a WAV file
        let bwf_description = if path.extension().and_then(|e| e.to_str()) == Some("wav") {
            Self::parse_bwf_description(path).unwrap_or(None)
        } else {
            None
        };

        Ok(Self {
            duration,
            sample_rate: track.codec_params.sample_rate,
            channels: track.codec_params.channels.map(|c| c.count() as u8),
            bwf_description,
        })
    }

    fn parse_bwf_description(path: &Path) -> Result<Option<String>, String> {
        let mut file = File::open(path).map_err(|e| e.to_string())?;
        let mut buf = [0u8; 12];

        // Read RIFF header
        file.read_exact(&mut buf).map_err(|e| e.to_string())?;
        if &buf[0..4] != b"RIFF" || &buf[8..12] != b"WAVE" {
            return Ok(None);
        }

        // Search for 'bext' chunk
        loop {
            let mut chunk_header = [0u8; 8];
            if file.read_exact(&mut chunk_header).is_err() {
                break;
            }

            let chunk_id = &chunk_header[0..4];
            let chunk_size = u32::from_le_bytes([
                chunk_header[4],
                chunk_header[5],
                chunk_header[6],
                chunk_header[7],
            ]);

            if chunk_id == b"bext" {
                // The BWF description is the first 256 bytes of the bext chunk.
                let mut bext_data = vec![0u8; chunk_size as usize];
                file.read_exact(&mut bext_data).map_err(|e| e.to_string())?;
                let Some(description_data) = bext_data.get(..256) else {
                    return Ok(None);
                };

                let description = Self::sanitize_string(
                    String::from_utf8_lossy(description_data).trim_end_matches('\0'),
                );

                return Ok(if description.is_empty() {
                    None
                } else {
                    Some(description)
                });
            }

            // Skip to next chunk
            file.seek(SeekFrom::Current(chunk_size as i64))
                .map_err(|e| e.to_string())?;

            // WAV chunks are padded to even boundaries
            if chunk_size % 2 != 0 {
                file.seek(SeekFrom::Current(1)).map_err(|e| e.to_string())?;
            }
        }

        Ok(None)
    }
}

impl MinimalMetadata {
    /// Sanitize a string by removing control characters and invalid UTF-8
    fn sanitize_string(s: &str) -> String {
        s.chars()
            .filter(|c| {
                // Keep printable ASCII, spaces, tabs, newlines, and valid Unicode
                c.is_ascii_graphic()
                    || c.is_ascii_whitespace()
                    || (!c.is_control() && !c.is_ascii())
            })
            .collect()
    }

    pub fn from_file(path: &Path) -> Result<Self, String> {
        use lofty::file::AudioFile; // Ensure trait is in scope

        // Fast path for WAV files - skip lofty entirely
        if path.extension().and_then(|e| e.to_str()) == Some("wav") {
            match Self::from_wav_fast(path) {
                Ok(metadata) => return Ok(metadata),
                Err(_) => {
                    // Fall back to lofty if custom parser fails
                }
            }
        }

        // For non-WAV files or fallback, use lofty
        match Probe::open(path).and_then(|p| p.read()) {
            Ok(tagged_file) => {
                let properties = tagged_file.properties();
                Ok(Self {
                    sample_rate: properties.sample_rate(),
                    channels: properties.channels(),
                    bwf_description: None, // Only WAV files have BWF
                })
            }
            Err(_) => {
                // If lofty fails, try symphonia as final fallback
                Self::from_symphonia(path)
            }
        }
    }

    /// Fallback metadata extraction using symphonia decoder
    /// This is more reliable for some audio formats that lofty may not handle well
    fn from_symphonia(path: &Path) -> Result<Self, String> {
        use symphonia::core::codecs::CODEC_TYPE_NULL;
        use symphonia::core::formats::FormatOptions;
        use symphonia::core::io::MediaSourceStream;
        use symphonia::core::meta::MetadataOptions;
        use symphonia::core::probe::Hint;

        let file = File::open(path).map_err(|e| format!("Failed to open file: {}", e))?;
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

        let format = probed.format;
        let track = format
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
            .ok_or_else(|| "No suitable audio track found".to_string())?;

        Ok(Self {
            sample_rate: track.codec_params.sample_rate,
            channels: track.codec_params.channels.map(|c| c.count() as u8),
            bwf_description: None,
        })
    }

    fn from_wav_fast(path: &Path) -> Result<Self, String> {
        let mut file = File::open(path).map_err(|e| e.to_string())?;
        let mut header = [0u8; 12];
        file.read_exact(&mut header).map_err(|e| e.to_string())?;

        // Validate RIFF/WAVE header
        if &header[0..4] != b"RIFF" || &header[8..12] != b"WAVE" {
            return Err("Invalid WAV file".into());
        }

        let mut sample_rate = None;
        let mut channels = None;
        let mut bwf_description = None;

        // Single pass through chunks
        loop {
            let mut chunk_header = [0u8; 8];
            if file.read_exact(&mut chunk_header).is_err() {
                break;
            }

            let chunk_id = &chunk_header[0..4];
            let chunk_size = u32::from_le_bytes([
                chunk_header[4],
                chunk_header[5],
                chunk_header[6],
                chunk_header[7],
            ]);

            match chunk_id {
                b"fmt " => {
                    // Read fmt chunk for sample_rate and channels
                    let bytes_to_read = chunk_size.min(16) as usize;
                    let mut fmt_data = vec![0u8; bytes_to_read];
                    file.read_exact(&mut fmt_data).map_err(|e| e.to_string())?;

                    if fmt_data.len() >= 8 {
                        channels = Some(u16::from_le_bytes([fmt_data[2], fmt_data[3]]) as u8);
                        sample_rate = Some(u32::from_le_bytes([
                            fmt_data[4],
                            fmt_data[5],
                            fmt_data[6],
                            fmt_data[7],
                        ]));
                    }

                    // Skip remaining fmt data if any
                    if chunk_size > 16 {
                        file.seek(SeekFrom::Current((chunk_size - 16) as i64))
                            .map_err(|e| e.to_string())?;
                    }
                }
                b"bext" => {
                    // Read only description field (first 256 bytes)
                    let mut description_data = [0u8; 256];
                    file.read_exact(&mut description_data)
                        .map_err(|e| e.to_string())?;

                    let description = Self::sanitize_string(
                        String::from_utf8_lossy(&description_data).trim_end_matches('\0'),
                    );

                    bwf_description = if description.is_empty() {
                        None
                    } else {
                        Some(description)
                    };

                    // Skip rest of bext chunk (we only need description)
                    if chunk_size > 256 {
                        file.seek(SeekFrom::Current((chunk_size - 256) as i64))
                            .map_err(|e| e.to_string())?;
                    }
                }
                _ => {
                    // Skip unknown chunks
                    file.seek(SeekFrom::Current(chunk_size as i64))
                        .map_err(|e| e.to_string())?;
                }
            }

            // Handle padding byte for odd-sized chunks
            if chunk_size % 2 != 0 {
                file.seek(SeekFrom::Current(1)).map_err(|e| e.to_string())?;
            }

            // Early exit if we found both fmt and bext (or just fmt if no bext exists)
            if sample_rate.is_some() && channels.is_some() {
                // We have fmt, and if we've seen bext, we would have captured it
                // Continue reading to find bext if we haven't seen it yet
                // But break if we've read enough chunks without finding it
                // For simplicity, we'll keep reading until EOF or error
            }
        }

        Ok(Self {
            sample_rate,
            channels,
            bwf_description,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_bwf_file(path: &Path, description_bytes: &[u8]) {
        let chunk_size = description_bytes.len() as u32;
        let riff_size = 4 + 8 + chunk_size;
        let mut contents = Vec::new();
        contents.extend_from_slice(b"RIFF");
        contents.extend_from_slice(&riff_size.to_le_bytes());
        contents.extend_from_slice(b"WAVE");
        contents.extend_from_slice(b"bext");
        contents.extend_from_slice(&chunk_size.to_le_bytes());
        contents.extend_from_slice(description_bytes);
        std::fs::write(path, contents).expect("write BWF fixture");
    }

    #[test]
    fn parses_bwf_description_without_unused_provenance_fields() {
        let temp_dir = tempfile::tempdir().expect("create temp directory");
        let path = temp_dir.path().join("described.wav");
        let mut description = [0u8; 256];
        description[..11].copy_from_slice(b"Field note!");
        write_bwf_file(&path, &description);

        assert_eq!(
            Metadata::parse_bwf_description(&path).expect("parse BWF description"),
            Some("Field note!".to_string())
        );
    }

    #[test]
    fn short_bext_chunk_is_ignored_instead_of_panicking() {
        let temp_dir = tempfile::tempdir().expect("create temp directory");
        let path = temp_dir.path().join("short.wav");
        write_bwf_file(&path, b"short");

        assert_eq!(
            Metadata::parse_bwf_description(&path).expect("parse short bext chunk"),
            None
        );
    }
}
