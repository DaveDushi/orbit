//! Windows: voice input with a local Whisper model. No cloud service, no API key and no Windows setting.
//!
//! WebView2's `webkitSpeechRecognition` does not work in the app (0.1.4: `network`, 0.1.6: `not-allowed` without
//! opening the microphone), and Windows' own dictation (0.1.7/0.1.8) needs the "Online speech recognition" privacy
//! switch. So the bridge script gives the page a `SpeechRecognition` that runs here: start records the default
//! microphone (cpal, WASAPI), stop transcribes the recording with whisper.cpp and sends the text as one final result.
//!
//! The model (Whisper base, multilingual, 5-bit: [`MODEL`]) is downloaded once, at the first use, from a pinned URL and
//! checked against its SHA-256; it records meanwhile. The page gets `progress` events (download, transcribing), the
//! text (`final`), an `error` (a Web Speech error code and a text for the person) and `end`. Every step goes to the
//! diagnostics log, never the words.

// The page's commands are read everywhere; only Windows runs them.
#![cfg_attr(not(windows), allow(dead_code))]

/// A command from the page's `SpeechRecognition` (`assets/bridge.js`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Start { id: u64, lang: String },
    Stop { id: u64 },
    Abort { id: u64 },
}

/// The Whisper model: a file of ggerganov/whisper.cpp on Hugging Face, at a pinned commit.
pub struct Model {
    pub file: &'static str,
    pub url: &'static str,
    pub sha256: &'static str,
    pub size: u64,
}

pub const MODEL: Model = Model {
    file: "ggml-base-q5_1.bin",
    url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1/ggml-base-q5_1.bin",
    sha256: "422f1ae452ade6f30a004d7e5c6a43195e4433bc370bf23fac9cc591f01a8898",
    size: 59_707_625,
};

/// Whisper's input rate.
pub const RATE: u32 = 16_000;

/// The longest recording kept (Whisper works through it in 30-second windows).
pub const MAX_SECONDS: u32 = 300;

/// Mono audio at `rate` to Whisper's 16 kHz: each output sample is the mean of the input samples it covers
/// (a simple low-pass for 44.1/48 kHz microphones); a slower input repeats its samples.
pub fn to_whisper_rate(samples: &[f32], rate: u32) -> Vec<f32> {
    if rate == RATE || samples.is_empty() || rate == 0 {
        return samples.to_vec();
    }
    let step = f64::from(rate) / f64::from(RATE);
    let len = (samples.len() as f64 / step).floor() as usize;
    (0..len)
        .map(|i| {
            let from = (i as f64 * step) as usize;
            let to = (((i + 1) as f64 * step) as usize).clamp(from + 1, samples.len());
            samples[from..to].iter().sum::<f32>() / (to - from) as f32
        })
        .collect()
}

/// Whisper's language code for the page's `lang` (`de-CH` → `de`); `auto` when there is none.
pub fn whisper_language(lang: &str) -> String {
    let code = lang.split(['-', '_']).next().unwrap_or_default().to_ascii_lowercase();
    if (2..=3).contains(&code.len()) && code.chars().all(|c| c.is_ascii_lowercase()) { code } else { "auto".into() }
}

/// Whisper's segments as one text: without the markers it writes for silence or noise (`[BLANK_AUDIO]`,
/// `(music)`, `[ Silence ]`).
pub fn clean_text(segments: &[String]) -> String {
    let mut text = String::new();
    for segment in segments {
        let segment = segment.trim();
        let marker = (segment.starts_with('[') && segment.ends_with(']')) || (segment.starts_with('(') && segment.ends_with(')'));
        if segment.is_empty() || marker {
            continue;
        }
        if !text.is_empty() {
            text.push(' ');
        }
        text.push_str(segment);
    }
    text
}

/// Too quiet to be speech: Whisper would invent words for it.
pub fn is_silence(samples: &[f32]) -> bool {
    samples.iter().all(|s| s.abs() < 0.01)
}

#[cfg(windows)]
pub use native::Dictation;

#[cfg(windows)]
mod native {
    use std::io::{Read, Write};
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex, mpsc};
    use std::time::Instant;

    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use cpal::{FromSample, Sample, SampleFormat, SizedSample, Stream};
    use serde_json::{Value, json};
    use sha2::{Digest, Sha256};
    use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

    use super::{Command, MAX_SECONDS, MODEL, RATE, clean_text, is_silence, to_whisper_rate, whisper_language};
    use crate::diagnostics;

    type Sender = Arc<dyn Fn(Value) + Send + Sync>;

    enum Control {
        Stop,
        Abort,
    }

    /// The running dictation: one at a time, on its own thread.
    #[derive(Default)]
    pub struct Dictation {
        running: Option<(u64, mpsc::Sender<Control>)>,
    }

    impl Dictation {
        pub fn handle(&mut self, command: Command, send: impl Fn(Value) + Send + Sync + 'static) {
            match command {
                Command::Start { id, lang } => {
                    if let Some((_, control)) = self.running.take() {
                        let _ = control.send(Control::Abort);
                    }
                    let (control, commands) = mpsc::channel();
                    let send: Sender = Arc::new(send);
                    std::thread::spawn(move || run(id, &lang, &send, &commands));
                    self.running = Some((id, control));
                }
                Command::Stop { id } | Command::Abort { id } => {
                    if self.running.as_ref().is_some_and(|(running, _)| *running == id)
                        && let Some((_, control)) = self.running.take()
                    {
                        let _ = control.send(if matches!(command, Command::Stop { .. }) { Control::Stop } else { Control::Abort });
                    }
                }
            }
        }
    }

    /// A Web Speech error code and a text for the person.
    struct Failure(&'static str, String);

    fn run(id: u64, lang: &str, send: &Sender, commands: &mpsc::Receiver<Control>) {
        let event = |kind: &str, extra: Value| {
            let mut event = json!({ "id": id, "type": kind });
            if let (Value::Object(event), Value::Object(extra)) = (&mut event, extra) {
                event.extend(extra);
            }
            send(event);
        };
        match dictate(id, lang, &event, commands) {
            Ok(Some(text)) => event("final", json!({ "text": text })),
            Ok(None) => {}
            Err(Failure(error, message)) => {
                diagnostics::log(&format!("speech {id}: {error}: {message}"));
                event("error", json!({ "error": error, "message": message }));
            }
        }
        event("end", json!({}));
    }

    /// Records until stop, then transcribes. `None`: aborted, or nothing was said.
    fn dictate(id: u64, lang: &str, event: &dyn Fn(&str, Value), commands: &mpsc::Receiver<Control>) -> Result<Option<String>, Failure> {
        let recording = Recording::start(id)?;
        let model = model_path().ok_or_else(|| Failure("failed", "Orbit has no folder for the speech model.".into()))?;
        let mut stopped = false;
        if !model_ready(&model) {
            stopped = download(id, &model, event, commands)?;
        }
        if !stopped {
            match commands.recv() {
                Ok(Control::Stop) => {}
                Ok(Control::Abort) | Err(_) => {
                    diagnostics::log(&format!("speech {id}: aborted"));
                    return Ok(None);
                }
            }
        }
        let (samples, rate) = recording.finish();
        diagnostics::log(&format!("speech {id}: recording stopped, {:.1} s", samples.len() as f32 / rate as f32));
        let audio = to_whisper_rate(&samples, rate);
        if audio.len() < RATE as usize / 2 || is_silence(&audio) {
            diagnostics::log(&format!("speech {id}: nothing to transcribe (short or silent)"));
            return Ok(None);
        }
        event("progress", json!({ "phase": "transcribing" }));
        let started = Instant::now();
        let text = transcribe(&model, &audio, &whisper_language(lang))?;
        diagnostics::log(&format!("speech {id}: transcribed {:.1} s of audio in {:.1} s ({} characters)", audio.len() as f32 / RATE as f32, started.elapsed().as_secs_f32(), text.chars().count()));
        Ok((!text.is_empty()).then_some(text))
    }

    /// The default microphone, as mono `f32` samples at its own rate.
    struct Recording {
        _stream: Stream,
        samples: Arc<Mutex<Vec<f32>>>,
        rate: u32,
    }

    impl Recording {
        fn start(id: u64) -> Result<Self, Failure> {
            let no_mic = |detail: String| Failure("audio-capture", format!("The microphone could not start ({detail}). Connect a microphone, and check Windows Settings → Privacy & security → Microphone."));
            let device = cpal::default_host().default_input_device().ok_or_else(|| Failure("audio-capture", "No microphone found: connect one and try again.".into()))?;
            let supported = device.default_input_config().map_err(|e| no_mic(e.to_string()))?;
            let config = supported.config();
            let samples = Arc::new(Mutex::new(Vec::new()));
            let stream = match supported.sample_format() {
                SampleFormat::F32 => input::<f32>(&device, &config, &samples, id),
                SampleFormat::I16 => input::<i16>(&device, &config, &samples, id),
                SampleFormat::U16 => input::<u16>(&device, &config, &samples, id),
                SampleFormat::I32 => input::<i32>(&device, &config, &samples, id),
                other => return Err(no_mic(format!("sample format {other}"))),
            }
            .map_err(|e| no_mic(e.to_string()))?;
            stream.play().map_err(|e| no_mic(e.to_string()))?;
            let name = device.description().map(|d| d.to_string()).unwrap_or_default();
            diagnostics::log(&format!("speech {id}: recording from \"{name}\", {} Hz, {} channels", config.sample_rate, config.channels));
            Ok(Recording { _stream: stream, samples, rate: config.sample_rate })
        }

        fn finish(self) -> (Vec<f32>, u32) {
            drop(self._stream);
            let samples = std::mem::take(&mut *self.samples.lock().unwrap_or_else(|e| e.into_inner()));
            (samples, self.rate)
        }
    }

    fn input<T: SizedSample>(device: &cpal::Device, config: &cpal::StreamConfig, samples: &Arc<Mutex<Vec<f32>>>, id: u64) -> Result<Stream, cpal::Error>
    where
        f32: FromSample<T>,
    {
        let channels = usize::from(config.channels.max(1));
        let max = (config.sample_rate * MAX_SECONDS) as usize;
        let samples = samples.clone();
        device.build_input_stream::<T, _, _>(
            *config,
            move |data: &[T], _| {
                let Ok(mut samples) = samples.lock() else { return };
                for frame in data.chunks(channels) {
                    if samples.len() >= max {
                        return;
                    }
                    samples.push(frame.iter().map(|s| f32::from_sample(*s)).sum::<f32>() / frame.len() as f32);
                }
            },
            move |e| diagnostics::log(&format!("speech {id}: microphone error: {e}")),
            None,
        )
    }

    fn model_path() -> Option<PathBuf> {
        Some(dirs::data_local_dir()?.join("orbit-desktop").join("models").join(MODEL.file))
    }

    /// The model is there with its full size (a download is checked before it gets its name).
    fn model_ready(path: &Path) -> bool {
        std::fs::metadata(path).is_ok_and(|m| m.len() == MODEL.size)
    }

    /// Downloads the model next to its place, checks its SHA-256, then renames it. It keeps recording meanwhile;
    /// `Ok(true)`: the person stopped during the download (transcribe after it).
    fn download(id: u64, path: &Path, event: &dyn Fn(&str, Value), commands: &mpsc::Receiver<Control>) -> Result<bool, Failure> {
        let failed = |detail: String| Failure("network", format!("The speech model ({} MB, once) could not be downloaded: {detail}. Check the internet connection and try again.", MODEL.size / 1_000_000));
        diagnostics::log(&format!("speech {id}: downloading the model {} ({} bytes)", MODEL.file, MODEL.size));
        event("progress", json!({ "phase": "download", "percent": 0 }));
        let started = Instant::now();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| failed(e.to_string()))?;
        }
        let part = path.with_extension("part");
        let mut response = ureq::get(MODEL.url).call().map_err(|e| failed(e.to_string()))?;
        let mut reader = response.body_mut().as_reader();
        let mut file = std::fs::File::create(&part).map_err(|e| failed(e.to_string()))?;
        let mut hash = Sha256::new();
        let mut buffer = vec![0; 256 * 1024];
        let (mut done, mut shown, mut stopped) = (0u64, 0u64, false);
        loop {
            match commands.try_recv() {
                Ok(Control::Stop) => stopped = true,
                Ok(Control::Abort) | Err(mpsc::TryRecvError::Disconnected) => {
                    let _ = std::fs::remove_file(&part);
                    diagnostics::log(&format!("speech {id}: download cancelled"));
                    return Err(Failure("aborted", String::new()));
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
            let n = reader.read(&mut buffer).map_err(|e| failed(e.to_string()))?;
            if n == 0 {
                break;
            }
            hash.update(&buffer[..n]);
            file.write_all(&buffer[..n]).map_err(|e| failed(e.to_string()))?;
            done += n as u64;
            let percent = (done * 100 / MODEL.size).min(100);
            if percent != shown {
                shown = percent;
                event("progress", json!({ "phase": "download", "percent": percent }));
            }
        }
        file.flush().map_err(|e| failed(e.to_string()))?;
        drop(file);
        let digest = hash.finalize().iter().map(|b| format!("{b:02x}")).collect::<String>();
        if done != MODEL.size || digest != MODEL.sha256 {
            let _ = std::fs::remove_file(&part);
            return Err(Failure("network", format!("The downloaded speech model was damaged ({done} bytes, SHA-256 {digest}). Try again.")));
        }
        std::fs::rename(&part, path).map_err(|e| failed(e.to_string()))?;
        diagnostics::log(&format!("speech {id}: model downloaded and checked in {:.1} s", started.elapsed().as_secs_f32()));
        Ok(stopped)
    }

    /// The loaded model, kept for the next dictation.
    static CONTEXT: Mutex<Option<Arc<WhisperContext>>> = Mutex::new(None);

    fn transcribe(model: &Path, audio: &[f32], language: &str) -> Result<String, Failure> {
        let failed = |e: whisper_rs::WhisperError| Failure("failed", format!("The speech model failed: {e}"));
        let context = {
            let mut loaded = CONTEXT.lock().unwrap_or_else(|e| e.into_inner());
            match &*loaded {
                Some(context) => context.clone(),
                None => {
                    let context = Arc::new(WhisperContext::new_with_params(model, WhisperContextParameters::default()).map_err(failed)?);
                    *loaded = Some(context.clone());
                    context
                }
            }
        };
        let mut state = context.create_state().map_err(failed)?;
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_language(Some(language));
        params.set_n_threads(std::thread::available_parallelism().map_or(4, |n| n.get().min(8)) as i32);
        params.set_no_context(true);
        params.set_suppress_blank(true);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_special(false);
        params.set_print_timestamps(false);
        state.full(params, audio).map_err(failed)?;
        let segments = state.as_iter().map(|segment| segment.to_str_lossy().map(|s| s.into_owned()).unwrap_or_default()).collect::<Vec<_>>();
        Ok(clean_text(&segments))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn microphone_audio_becomes_16_khz() {
        let second = vec![0.5; 48_000];
        let audio = to_whisper_rate(&second, 48_000);
        assert_eq!(audio.len(), 16_000);
        assert!(audio.iter().all(|s| (s - 0.5).abs() < 1e-6));
        assert_eq!(to_whisper_rate(&vec![0.1; 44_100], 44_100).len(), 16_000);
        // Each output sample averages the input it covers: a 24 kHz tone at 48 kHz cancels out.
        let tone = (0..4_800).map(|i| if i % 2 == 0 { 1.0 } else { -1.0 }).collect::<Vec<f32>>();
        assert!(to_whisper_rate(&tone, 48_000).iter().all(|s| s.abs() < 0.34));
        assert_eq!(to_whisper_rate(&[0.2; 8_000], 8_000), vec![0.2; 16_000], "a slower input repeats its samples");
    }

    #[test]
    fn whisper_gets_the_pages_language() {
        assert_eq!(whisper_language("en-US"), "en");
        assert_eq!(whisper_language("de_CH"), "de");
        assert_eq!(whisper_language("HE"), "he");
        assert_eq!(whisper_language(""), "auto");
        assert_eq!(whisper_language("x1"), "auto");
    }

    #[test]
    fn silence_markers_are_left_out() {
        let segments = [" Hello there.", " [BLANK_AUDIO]", "(music)", " How are you? "].map(String::from);
        assert_eq!(clean_text(&segments), "Hello there. How are you?");
        assert_eq!(clean_text(&["[ Silence ]".into()]), "");
        assert!(is_silence(&[0.0, 0.005, -0.009]));
        assert!(!is_silence(&[0.0, 0.2]));
    }
}
