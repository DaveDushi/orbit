//! The converters: ffprobe and ffmpeg for videos, LibreOffice for office files. Every tool runs at the lowest CPU
//! priority (`nice -n 19`) with a time limit, and is killed when the job stops. LibreOffice also runs without
//! network (a user and network namespace), with its own HOME and profile in the job's directory, and with an
//! address-space limit.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde::Deserialize;
use tokio_util::sync::CancellationToken;

use super::settings::PreviewSettings;

/// Container formats ffmpeg may open: a file that is really a playlist (HLS, concat) is refused, so a video can
/// make ffmpeg fetch nothing but its own bytes.
const FORMATS: &str = "mov,mp4,m4a,3gp,3g2,mj2,matroska,webm,avi,ogg,mpegts,flv,mpeg";
/// Protocols for a local file and for a Drive file's presigned URL.
const PROTOCOLS: &str = "file,http,https,tcp,tls";
/// The longest side of a playable copy and of a poster.
const MAX_VIDEO_SIDE: u32 = 1920;
const MAX_POSTER_SIDE: u32 = 1280;
const PROBE_TIMEOUT: Duration = Duration::from_secs(60);
const POSTER_TIMEOUT: Duration = Duration::from_secs(120);
/// The end of a tool's error output kept for the log.
const ERROR_TAIL: usize = 400;

#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    #[error("{0} is not installed")]
    Missing(String),
    #[error("the converter failed: {0}")]
    Failed(String),
    #[error("the converter took too long")]
    TimedOut,
    #[error("the job stopped")]
    Cancelled,
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// What ffprobe found in a video.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VideoFacts {
    pub duration_seconds: Option<f64>,
    pub video_codec: Option<String>,
    pub pixel_format: Option<String>,
    pub audio_codec: Option<String>,
}

impl VideoFacts {
    /// Browsers play H.264 in 8-bit 4:2:0 everywhere: such a video is only copied into an MP4 (no new encode).
    #[must_use]
    pub fn remux_video(&self) -> bool {
        self.video_codec.as_deref() == Some("h264")
            && matches!(self.pixel_format.as_deref(), Some("yuv420p" | "yuvj420p"))
    }

    #[must_use]
    pub fn copy_audio(&self) -> bool {
        self.audio_codec.as_deref() == Some("aac")
    }
}

#[derive(Deserialize)]
struct Probe {
    #[serde(default)]
    streams: Vec<ProbeStream>,
    format: Option<ProbeFormat>,
}

#[derive(Deserialize)]
struct ProbeStream {
    codec_type: Option<String>,
    codec_name: Option<String>,
    pix_fmt: Option<String>,
}

#[derive(Deserialize)]
struct ProbeFormat {
    duration: Option<String>,
}

/// Reads ffprobe's JSON. A file without a video stream is not a video.
pub fn parse_probe(json: &[u8]) -> Result<VideoFacts, ToolError> {
    let probe: Probe = serde_json::from_slice(json)
        .map_err(|error| ToolError::Failed(format!("unreadable ffprobe output: {error}")))?;
    let video = probe
        .streams
        .iter()
        .find(|stream| stream.codec_type.as_deref() == Some("video"))
        .ok_or_else(|| ToolError::Failed("the file has no video stream".to_owned()))?;
    let audio = probe
        .streams
        .iter()
        .find(|stream| stream.codec_type.as_deref() == Some("audio"));
    Ok(VideoFacts {
        duration_seconds: probe
            .format
            .and_then(|format| format.duration)
            .and_then(|duration| duration.parse().ok()),
        video_codec: video.codec_name.clone(),
        pixel_format: video.pix_fmt.clone(),
        audio_codec: audio.and_then(|audio| audio.codec_name.clone()),
    })
}

pub async fn probe(
    settings: &PreviewSettings,
    input: &str,
    cancel: &CancellationToken,
) -> Result<VideoFacts, ToolError> {
    let program = find_program(&settings.ffprobe)?;
    let args: Vec<OsString> = [
        "-v",
        "error",
        "-protocol_whitelist",
        PROTOCOLS,
        "-format_whitelist",
        FORMATS,
        "-show_entries",
        "stream=codec_type,codec_name,pix_fmt:format=duration",
        "-of",
        "json",
        input,
    ]
    .iter()
    .map(OsString::from)
    .collect();
    let output = run(nice(&program, args), PROBE_TIMEOUT, cancel).await?;
    parse_probe(&output)
}

/// The ffmpeg arguments that make a playable MP4 (H.264 and AAC, `+faststart`) of `input`.
#[must_use]
pub fn transcode_args(
    facts: &VideoFacts,
    input: &str,
    output: &Path,
    threads: u32,
) -> Vec<OsString> {
    let mut args: Vec<OsString> = [
        "-nostdin",
        "-hide_banner",
        "-nostats",
        "-loglevel",
        "error",
        "-protocol_whitelist",
        PROTOCOLS,
        "-format_whitelist",
        FORMATS,
        "-i",
        input,
        "-map",
        "0:v:0",
        "-map",
        "0:a:0?",
        "-sn",
        "-dn",
    ]
    .iter()
    .map(OsString::from)
    .collect();
    let mut push = |values: &[&str]| args.extend(values.iter().map(OsString::from));
    if facts.remux_video() {
        push(&["-c:v", "copy"]);
    } else {
        let scale = format!(
            "scale='if(gt(iw\\,ih)\\,min({MAX_VIDEO_SIDE}\\,iw)\\,-2)':'if(gt(iw\\,ih)\\,-2\\,min({MAX_VIDEO_SIDE}\\,ih))'"
        );
        push(&[
            "-c:v", "libx264", "-preset", "veryfast", "-crf", "23", "-pix_fmt", "yuv420p", "-vf",
            &scale,
        ]);
    }
    if facts.copy_audio() {
        push(&["-c:a", "copy"]);
    } else {
        push(&["-c:a", "aac", "-b:a", "128k", "-ac", "2"]);
    }
    let threads = threads.to_string();
    push(&[
        "-threads",
        &threads,
        "-movflags",
        "+faststart",
        "-f",
        "mp4",
        "-y",
    ]);
    args.push(output.as_os_str().to_owned());
    args
}

pub async fn transcode(
    settings: &PreviewSettings,
    facts: &VideoFacts,
    input: &str,
    output: &Path,
    cancel: &CancellationToken,
) -> Result<(), ToolError> {
    let program = find_program(&settings.ffmpeg)?;
    let args = transcode_args(facts, input, output, settings.ffmpeg_threads);
    run(nice(&program, args), settings.video_timeout, cancel)
        .await
        .map(drop)
}

/// A JPEG frame of the video, one second in (or at half of a shorter video).
pub async fn poster(
    settings: &PreviewSettings,
    input: &str,
    duration_seconds: Option<f64>,
    output: &Path,
    cancel: &CancellationToken,
) -> Result<(), ToolError> {
    let program = find_program(&settings.ffmpeg)?;
    let at = duration_seconds.map_or(1.0, |duration| (duration / 2.0).min(1.0));
    let scale = format!(
        "scale='if(gt(iw\\,ih)\\,min({MAX_POSTER_SIDE}\\,iw)\\,-2)':'if(gt(iw\\,ih)\\,-2\\,min({MAX_POSTER_SIDE}\\,ih))'"
    );
    let at = format!("{at:.3}");
    let mut args: Vec<OsString> = [
        "-nostdin",
        "-hide_banner",
        "-nostats",
        "-loglevel",
        "error",
        "-protocol_whitelist",
        PROTOCOLS,
        "-format_whitelist",
        FORMATS,
        "-ss",
        &at,
        "-i",
        input,
        "-frames:v",
        "1",
        "-vf",
        &scale,
        "-q:v",
        "4",
        "-f",
        "image2",
        "-c:v",
        "mjpeg",
        "-threads",
        "1",
        "-y",
    ]
    .iter()
    .map(OsString::from)
    .collect();
    args.push(output.as_os_str().to_owned());
    run(nice(&program, args), POSTER_TIMEOUT, cancel).await?;
    if tokio::fs::metadata(output)
        .await
        .map(|meta| meta.len())
        .unwrap_or(0)
        == 0
    {
        return Err(ToolError::Failed("no frame for a poster".to_owned()));
    }
    Ok(())
}

/// The command that converts `input` (in `directory`) to `directory/out/<stem>.pdf`.
#[must_use]
pub fn document_command(
    settings: &PreviewSettings,
    soffice: &Path,
    directory: &Path,
    input: &Path,
) -> tokio::process::Command {
    let program = if settings.sandbox {
        "unshare"
    } else {
        "timeout"
    };
    let mut command = tokio::process::Command::new(program);
    if settings.sandbox {
        command.args(["--user", "--map-root-user", "--net", "--", "timeout"]);
    }
    let seconds = settings.document_timeout.as_secs().max(1).to_string();
    command
        .args(["--kill-after=10", &seconds, "prlimit"])
        .arg(format!("--as={}", settings.document_memory_bytes))
        .args(["--", "nice", "-n", "19"])
        .arg(soffice)
        .args([
            "--headless",
            "--invisible",
            "--norestore",
            "--nologo",
            "--nodefault",
            "--nolockcheck",
        ])
        .arg(format!(
            "-env:UserInstallation=file://{}",
            directory.join("profile").display()
        ));
    if input
        .extension()
        .is_some_and(|extension| extension == "tsv")
    {
        // Tab separated, double quotes, UTF-8, from line 1.
        command.arg("--infilter=CSV:9,34,76,1");
    }
    command
        .args(["--convert-to", "pdf", "--outdir"])
        .arg(directory.join("out"))
        .arg(input)
        .current_dir(directory)
        .env_clear()
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .env("HOME", directory.join("home"))
        .env("TMPDIR", directory.join("tmp"))
        .env("LANG", "C.UTF-8");
    command
}

/// Converts an office file to PDF with LibreOffice. Returns the PDF's path.
pub async fn document_to_pdf(
    settings: &PreviewSettings,
    directory: &Path,
    input: &Path,
    cancel: &CancellationToken,
) -> Result<PathBuf, ToolError> {
    let soffice = find_program(&settings.soffice)?;
    for folder in ["profile", "out", "home", "tmp"] {
        tokio::fs::create_dir_all(directory.join(folder)).await?;
    }
    let command = document_command(settings, &soffice, directory, input);
    // `timeout` inside the command stops LibreOffice first; this is the backstop.
    run(
        command,
        settings.document_timeout + Duration::from_secs(30),
        cancel,
    )
    .await?;
    let stem = input
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    let pdf = directory.join("out").join(format!("{stem}.pdf"));
    match tokio::fs::metadata(&pdf).await {
        Ok(meta) if meta.len() > 0 => Ok(pdf),
        _ => Err(ToolError::Failed("LibreOffice wrote no PDF".to_owned())),
    }
}

fn nice(program: &Path, args: Vec<OsString>) -> tokio::process::Command {
    let mut command = tokio::process::Command::new("nice");
    command.args(["-n", "19"]).arg(program).args(args);
    command
}

/// Runs a tool to its end and returns its standard output; a non-zero exit is [`ToolError::Failed`] with the end
/// of its error output. The tool is killed when the time is up or the job stops.
async fn run(
    mut command: tokio::process::Command,
    limit: Duration,
    cancel: &CancellationToken,
) -> Result<Vec<u8>, ToolError> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let child = command.spawn()?;
    let output = tokio::select! {
        output = tokio::time::timeout(limit, child.wait_with_output()) => match output {
            Ok(output) => output?,
            Err(_) => return Err(ToolError::TimedOut),
        },
        () = cancel.cancelled() => return Err(ToolError::Cancelled),
    };
    if output.status.success() {
        return Ok(output.stdout);
    }
    // `timeout` exits with 124 when the time is up.
    if output.status.code() == Some(124) {
        return Err(ToolError::TimedOut);
    }
    let error = String::from_utf8_lossy(&output.stderr);
    let error = error.trim();
    let tail: String = error
        .chars()
        .skip(error.chars().count().saturating_sub(ERROR_TAIL))
        .collect();
    Err(ToolError::Failed(if tail.is_empty() {
        format!("exit status {}", output.status)
    } else {
        tail
    }))
}

/// The tool's path: as given when it has a `/`, else found on `PATH`.
pub fn find_program(program: &Path) -> Result<PathBuf, ToolError> {
    let missing = || ToolError::Missing(program.display().to_string());
    if program.components().count() > 1 {
        return program
            .is_file()
            .then(|| program.to_owned())
            .ok_or_else(missing);
    }
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
        .map(|directory| directory.join(program))
        .find(|candidate| candidate.is_file())
        .ok_or_else(missing)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(video: &str, pixels: &str, audio: Option<&str>) -> VideoFacts {
        VideoFacts {
            duration_seconds: Some(12.0),
            video_codec: Some(video.to_owned()),
            pixel_format: Some(pixels.to_owned()),
            audio_codec: audio.map(str::to_owned),
        }
    }

    fn has(args: &[OsString], pair: [&str; 2]) -> bool {
        args.windows(2)
            .any(|window| window[0] == pair[0] && window[1] == pair[1])
    }

    #[test]
    fn h264_is_remuxed_and_everything_else_is_encoded_again() {
        let output = Path::new("/tmp/out.mp4");
        let remux = transcode_args(&facts("h264", "yuv420p", Some("aac")), "in.mov", output, 2);
        assert!(has(&remux, ["-c:v", "copy"]));
        assert!(has(&remux, ["-c:a", "copy"]));
        assert!(has(&remux, ["-movflags", "+faststart"]));
        assert!(has(&remux, ["-format_whitelist", FORMATS]));
        let hevc = transcode_args(
            &facts("hevc", "yuv420p", Some("pcm_s16le")),
            "in.mov",
            output,
            2,
        );
        assert!(has(&hevc, ["-c:v", "libx264"]));
        assert!(has(&hevc, ["-c:a", "aac"]));
        // 10-bit H.264 does not play in every browser.
        let ten_bit = transcode_args(&facts("h264", "yuv420p10le", None), "in.mp4", output, 1);
        assert!(has(&ten_bit, ["-c:v", "libx264"]));
        assert!(has(&ten_bit, ["-threads", "1"]));
    }

    #[test]
    fn ffprobe_output_is_read_and_a_file_without_video_is_refused() {
        let json = br#"{"streams":[{"codec_type":"audio","codec_name":"aac"},{"codec_type":"video","codec_name":"hevc","pix_fmt":"yuv420p10le"}],"format":{"duration":"8.533"}}"#;
        let facts = parse_probe(json).unwrap();
        assert_eq!(facts.video_codec.as_deref(), Some("hevc"));
        assert_eq!(facts.audio_codec.as_deref(), Some("aac"));
        assert_eq!(facts.duration_seconds, Some(8.533));
        assert!(!facts.remux_video());
        assert!(matches!(
            parse_probe(br#"{"streams":[{"codec_type":"audio","codec_name":"mp3"}]}"#),
            Err(ToolError::Failed(_))
        ));
    }

    #[test]
    fn libreoffice_runs_without_network_with_its_own_profile_and_limits() {
        let settings = PreviewSettings::default();
        let command = document_command(
            &settings,
            Path::new("/usr/bin/soffice"),
            Path::new("/tmp/job"),
            Path::new("/tmp/job/input.xlsx"),
        );
        let std = command.as_std();
        assert_eq!(std.get_program(), "unshare");
        let args: Vec<String> = std
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert!(args.starts_with(
            &["--user", "--map-root-user", "--net", "--", "timeout"].map(String::from)
        ));
        assert!(args.contains(&format!("--as={}", settings.document_memory_bytes)));
        assert!(args.contains(&"-env:UserInstallation=file:///tmp/job/profile".to_owned()));
        let home = std
            .get_envs()
            .find(|(key, _)| *key == "HOME")
            .and_then(|(_, value)| value);
        assert_eq!(home, Some(std::ffi::OsStr::new("/tmp/job/home")));
        let plain = document_command(
            &PreviewSettings {
                sandbox: false,
                ..PreviewSettings::default()
            },
            Path::new("soffice"),
            Path::new("/tmp/job"),
            Path::new("/tmp/job/input.tsv"),
        );
        assert_eq!(plain.as_std().get_program(), "timeout");
        assert!(
            plain
                .as_std()
                .get_args()
                .any(|arg| arg == "--infilter=CSV:9,34,76,1")
        );
    }

    #[test]
    fn a_missing_tool_is_reported_as_missing() {
        assert!(matches!(
            find_program(Path::new("orbit-no-such-tool")),
            Err(ToolError::Missing(_))
        ));
        assert!(matches!(
            find_program(Path::new("/no/such/soffice")),
            Err(ToolError::Missing(_))
        ));
    }
}
