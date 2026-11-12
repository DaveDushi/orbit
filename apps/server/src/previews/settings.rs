//! `ORBIT_MEDIA_*`: the limits and tools of file previews. The platform config ignores the prefix; this module
//! reads it and refuses an unknown key, as the platform does for its own keys.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

const MIB: u64 = 1024 * 1024;

/// What file previews may use on this server.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreviewSettings {
    /// Videos up to this size get a playable copy (`ORBIT_MEDIA_TRANSCODE_MAX_BYTES`, default 2 GiB). Larger ones
    /// get only a poster: the browser plays the original when it can, else people download it.
    pub transcode_max_bytes: u64,
    /// Videos that must be encoded again (not H.264) and are longer than this get no playable copy
    /// (`ORBIT_MEDIA_TRANSCODE_MAX_SECONDS`, default 20 minutes). An H.264 video is only remuxed, at any length.
    pub transcode_max_seconds: u64,
    /// The longest a video job may run (`ORBIT_MEDIA_VIDEO_TIMEOUT_SECONDS`, default 1 hour).
    pub video_timeout: Duration,
    /// Office files up to this size get a PDF (`ORBIT_MEDIA_DOCUMENT_MAX_BYTES`, default 100 MiB).
    pub document_max_bytes: u64,
    /// The longest LibreOffice may run for one file (`ORBIT_MEDIA_DOCUMENT_TIMEOUT_SECONDS`, default 120 s).
    pub document_timeout: Duration,
    /// The address space LibreOffice may use (`ORBIT_MEDIA_DOCUMENT_MEMORY_MB`, default 3072).
    pub document_memory_bytes: u64,
    /// Threads of one ffmpeg run (`ORBIT_MEDIA_FFMPEG_THREADS`, default 2 of the server's cores).
    pub ffmpeg_threads: u32,
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
    /// LibreOffice (`ORBIT_MEDIA_SOFFICE`, default `soffice`). Without it office files show "preview not available".
    pub soffice: PathBuf,
    /// Runs LibreOffice in a user and network namespace (`unshare`), so a document cannot reach the network
    /// (`ORBIT_MEDIA_SANDBOX`: `unshare`, the default, or `none` on a host without user namespaces).
    pub sandbox: bool,
}

impl Default for PreviewSettings {
    fn default() -> Self {
        Self {
            transcode_max_bytes: 2048 * MIB,
            transcode_max_seconds: 20 * 60,
            video_timeout: Duration::from_secs(60 * 60),
            document_max_bytes: 100 * MIB,
            document_timeout: Duration::from_secs(120),
            document_memory_bytes: 3072 * MIB,
            ffmpeg_threads: 2,
            ffmpeg: PathBuf::from("ffmpeg"),
            ffprobe: PathBuf::from("ffprobe"),
            soffice: PathBuf::from("soffice"),
            sandbox: true,
        }
    }
}

impl PreviewSettings {
    pub fn from_env() -> Result<Self, String> {
        Self::from_vars(std::env::vars().collect())
    }

    pub fn from_vars(vars: BTreeMap<String, String>) -> Result<Self, String> {
        let mut settings = Self::default();
        for (key, value) in vars {
            let Some(name) = key.strip_prefix("ORBIT_MEDIA_") else {
                continue;
            };
            let number = || {
                value
                    .trim()
                    .parse::<u64>()
                    .ok()
                    .filter(|number| *number > 0)
                    .ok_or_else(|| format!("{key} must be a positive whole number"))
            };
            match name {
                "TRANSCODE_MAX_BYTES" => settings.transcode_max_bytes = number()?,
                "TRANSCODE_MAX_SECONDS" => settings.transcode_max_seconds = number()?,
                "VIDEO_TIMEOUT_SECONDS" => settings.video_timeout = Duration::from_secs(number()?),
                "DOCUMENT_MAX_BYTES" => settings.document_max_bytes = number()?,
                "DOCUMENT_TIMEOUT_SECONDS" => {
                    settings.document_timeout = Duration::from_secs(number()?);
                }
                "DOCUMENT_MEMORY_MB" => settings.document_memory_bytes = number()? * MIB,
                "FFMPEG_THREADS" => {
                    settings.ffmpeg_threads =
                        u32::try_from(number()?).map_err(|_| format!("{key} is too large"))?;
                }
                "FFMPEG" => settings.ffmpeg = PathBuf::from(value),
                "FFPROBE" => settings.ffprobe = PathBuf::from(value),
                "SOFFICE" => settings.soffice = PathBuf::from(value),
                "SANDBOX" => {
                    settings.sandbox = match value.as_str() {
                        "unshare" => true,
                        "none" => false,
                        _ => return Err(format!("{key} must be `unshare` or `none`")),
                    };
                }
                _ => return Err(format!("unknown setting {key}")),
            }
        }
        Ok(settings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_have_safe_defaults_and_refuse_unknown_keys() {
        let defaults = PreviewSettings::from_vars(BTreeMap::new()).unwrap();
        assert_eq!(defaults.transcode_max_bytes, 2 * 1024 * MIB);
        assert!(defaults.sandbox);
        let vars = |pairs: &[(&str, &str)]| {
            pairs
                .iter()
                .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
                .collect()
        };
        let custom = PreviewSettings::from_vars(vars(&[
            ("ORBIT_MEDIA_TRANSCODE_MAX_BYTES", "1000"),
            ("ORBIT_MEDIA_SANDBOX", "none"),
            ("ORBIT_OTHER", "x"),
        ]))
        .unwrap();
        assert_eq!(custom.transcode_max_bytes, 1000);
        assert!(!custom.sandbox);
        assert!(
            PreviewSettings::from_vars(vars(&[("ORBIT_MEDIA_TRANSCODE_MAX_BYTE", "1")])).is_err()
        );
        assert!(
            PreviewSettings::from_vars(vars(&[("ORBIT_MEDIA_DOCUMENT_MAX_BYTES", "0")])).is_err()
        );
        assert!(PreviewSettings::from_vars(vars(&[("ORBIT_MEDIA_SANDBOX", "bwrap")])).is_err());
    }
}
