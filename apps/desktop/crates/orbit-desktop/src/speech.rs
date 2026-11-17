//! Windows: voice input with a local Whisper model. No cloud service, no API key and no Windows setting.
//!
//! WebView2's `webkitSpeechRecognition` does not work in the app (0.1.4: `network`, 0.1.6: `not-allowed` without
//! opening the microphone), and Windows' own dictation (0.1.7/0.1.8) needs the "Online speech recognition" privacy
//! switch. So the bridge script gives the page a `SpeechRecognition` that runs here: start records the default
//! microphone (cpal, WASAPI) and loads the model; while it records, whisper.cpp transcribes parts of about 5 s on a
//! second thread, so after stop only the last part is left (0.5-1 s).
//!
//! The model (Whisper base, multilingual, 5-bit: [`MODEL`]) is downloaded once, at the first use, from a pinned URL and
//! checked against its SHA-256; it records meanwhile. The page gets the microphone's `level` (10 times a second),
//! `progress` events (download, transcribing), each part's text (`final`), an `error` (a Web Speech error code and a
//! text for the person) and `end`. Every step and its time go to the diagnostics log, never the words.

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

/// While it records, the audio is transcribed in parts of about this length, so after stop only the last part is left.
pub const PART_SECONDS: u32 = 5;

/// Where the next part of `pending` (mono audio at `rate`, not transcribed yet) ends: in the middle of the quietest
/// 100 ms of its last 2 seconds, so that a word is rarely cut. `None` while less than [`PART_SECONDS`] waits.
pub fn part_end(pending: &[f32], rate: u32) -> Option<usize> {
    let window = (rate / 10).max(1) as usize;
    if rate == 0 || pending.len() < (rate * PART_SECONDS) as usize {
        return None;
    }
    let from = pending.len() - (2 * rate) as usize;
    let loudness = |start: usize| pending[start..start + window].iter().map(|s| s.abs()).sum::<f32>();
    let quietest = (from..=pending.len() - window).step_by(window / 2).min_by(|a, b| loudness(*a).total_cmp(&loudness(*b)))?;
    Some(quietest + window / 2)
}

/// whisper.cpp refuses input shorter than 1 s: a short last part gets silence after it.
pub fn padded(audio: &[f32]) -> Vec<f32> {
    let mut audio = audio.to_vec();
    let min = RATE as usize * 11 / 10;
    if audio.len() < min {
        audio.resize(min, 0.0);
    }
    audio
}

/// The encoder's context for `samples` of 16 kHz audio: Whisper encodes 30 s as 1500 positions (one per 20 ms), and
/// always the whole window unless told otherwise. A 5 s part needs about a sixth of it, which makes it that much faster.
/// A margin keeps the end of the part, and a floor keeps short parts accurate.
pub fn audio_ctx(samples: usize) -> i32 {
    (samples.div_ceil(320) + 128).clamp(384, 1500) as i32
}

#[cfg(windows)]
pub use native::Dictation;

#[cfg(windows)]
mod native {
    use std::io::{Read, Write};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex, mpsc};
    use std::time::{Duration, Instant};

    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use cpal::{FromSample, Sample, SampleFormat, SizedSample, Stream};
    use serde_json::{Value, json};
    use sha2::{Digest, Sha256};
    use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

    use super::{Command, MAX_SECONDS, MODEL, RATE, audio_ctx, clean_text, is_silence, padded, part_end, to_whisper_rate, whisper_language};
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
                    let events = Events { id, send: Arc::new(send), aborted: Arc::default() };
                    std::thread::spawn(move || run(&lang, &events, &commands));
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

    /// The page's events of one dictation; after an abort they are no longer sent.
    #[derive(Clone)]
    struct Events {
        id: u64,
        send: Sender,
        aborted: Arc<AtomicBool>,
    }

    impl Events {
        fn send(&self, kind: &str, extra: Value) {
            if self.aborted.load(Ordering::Relaxed) {
                return;
            }
            let mut event = json!({ "id": self.id, "type": kind });
            if let (Value::Object(event), Value::Object(extra)) = (&mut event, extra) {
                event.extend(extra);
            }
            (self.send)(event);
        }
    }

    /// A Web Speech error code and a text for the person.
    struct Failure(&'static str, String);

    fn run(lang: &str, events: &Events, commands: &mpsc::Receiver<Control>) {
        let id = events.id;
        if let Err(Failure(error, message)) = dictate(lang, events, commands) {
            diagnostics::log(&format!("speech {id}: {error}: {message}"));
            events.send("error", json!({ "error": error, "message": message }));
        }
        events.send("end", json!({}));
    }

    /// Records until stop. Meanwhile it sends the microphone's level, and a second thread transcribes the audio in
    /// parts of about [`super::PART_SECONDS`] (each part's text goes to the page as soon as it is ready), so after stop only
    /// the last part is left.
    fn dictate(lang: &str, events: &Events, commands: &mpsc::Receiver<Control>) -> Result<(), Failure> {
        let id = events.id;
        let recording = Recording::start(id)?;
        let model = model_path().ok_or_else(|| Failure("failed", "Orbit has no folder for the speech model.".into()))?;
        let mut stopped = false;
        if !model_ready(&model) {
            stopped = download(id, &model, events, commands)?;
        }
        let (parts, transcriber) = Transcriber::start(model, whisper_language(lang), events.clone());
        let rate = recording.rate;
        let (mut done, mut heard) = (0, 0);
        while !stopped {
            match commands.recv_timeout(Duration::from_millis(100)) {
                Ok(Control::Stop) => stopped = true,
                Ok(Control::Abort) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                    events.aborted.store(true, Ordering::Relaxed);
                    diagnostics::log(&format!("speech {id}: aborted"));
                    return Ok(());
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    let samples = recording.samples.lock().unwrap_or_else(|e| e.into_inner());
                    let level = samples[heard..].iter().fold(0f32, |peak, s| peak.max(s.abs()));
                    heard = samples.len();
                    let end = (!transcriber.busy.load(Ordering::Relaxed)).then(|| part_end(&samples[done..], rate)).flatten();
                    let part = end.map(|end| to_whisper_rate(&samples[done..done + end], rate));
                    drop(samples);
                    events.send("level", json!({ "level": (level.min(1.0) * 100.0).round() / 100.0 }));
                    if let (Some(end), Some(part)) = (end, part) {
                        done += end;
                        transcriber.busy.store(true, Ordering::Relaxed);
                        let _ = parts.send(part);
                    }
                }
            }
        }
        let stop = Instant::now();
        let (samples, rate) = recording.finish();
        events.send("progress", json!({ "phase": "transcribing" }));
        let last = to_whisper_rate(&samples[done.min(samples.len())..], rate);
        diagnostics::log(&format!("speech {id}: recording stopped, {:.1} s, {:.1} s left to transcribe", samples.len() as f32 / rate as f32, last.len() as f32 / RATE as f32));
        // A click on the mic and at once again: Whisper would invent words for half a second of noise.
        if !is_silence(&last) && samples.len() >= rate as usize / 2 {
            let _ = parts.send(last);
        }
        drop(parts);
        let text = transcriber.thread.join().unwrap_or_else(|_| Err(Failure("failed", "The speech model stopped.".into())))?;
        diagnostics::log(&format!("speech {id}: stop to text {:.2} s ({} characters in all)", stop.elapsed().as_secs_f32(), text.chars().count()));
        Ok(())
    }

    /// Transcribes the parts it gets in order, on its own thread: it loads the model first (once per app run), while
    /// the person speaks. Ends when the parts' sender is dropped, with the whole text.
    struct Transcriber {
        busy: Arc<AtomicBool>,
        thread: std::thread::JoinHandle<Result<String, Failure>>,
    }

    impl Transcriber {
        fn start(model: PathBuf, language: String, events: Events) -> (mpsc::Sender<Vec<f32>>, Self) {
            let (parts, received) = mpsc::channel::<Vec<f32>>();
            let busy = Arc::new(AtomicBool::new(false));
            let idle = busy.clone();
            let thread = std::thread::spawn(move || {
                let context = load(&model, events.id)?;
                let mut text = String::new();
                for part in received {
                    if events.aborted.load(Ordering::Relaxed) {
                        break;
                    }
                    if !is_silence(&part) {
                        let started = Instant::now();
                        let said = transcribe(&context, &padded(&part), &language, &text)?;
                        diagnostics::log(&format!("speech {}: transcribed {:.1} s of audio in {:.2} s", events.id, part.len() as f32 / RATE as f32, started.elapsed().as_secs_f32()));
                        if !said.is_empty() {
                            events.send("final", json!({ "text": said }));
                            text = if text.is_empty() { said } else { format!("{text} {said}") };
                        }
                    }
                    idle.store(false, Ordering::Relaxed);
                }
                Ok(text)
            });
            (parts, Transcriber { busy, thread })
        }
    }

    /// The default microphone, as mono `f32` samples at its own rate.
    struct Recording {
        stream: Stream,
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
            Ok(Recording { stream, samples, rate: config.sample_rate })
        }

        fn finish(self) -> (Vec<f32>, u32) {
            drop(self.stream);
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
    fn download(id: u64, path: &Path, events: &Events, commands: &mpsc::Receiver<Control>) -> Result<bool, Failure> {
        let failed = |detail: String| Failure("network", format!("The speech model ({} MB, once) could not be downloaded: {detail}. Check the internet connection and try again.", MODEL.size / 1_000_000));
        diagnostics::log(&format!("speech {id}: downloading the model {} ({} bytes)", MODEL.file, MODEL.size));
        events.send("progress", json!({ "phase": "download", "percent": 0 }));
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
                    events.aborted.store(true, Ordering::Relaxed);
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
                events.send("progress", json!({ "phase": "download", "percent": percent }));
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

    fn load(model: &Path, id: u64) -> Result<Arc<WhisperContext>, Failure> {
        let mut loaded = CONTEXT.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(context) = &*loaded {
            return Ok(context.clone());
        }
        let started = Instant::now();
        let context = Arc::new(WhisperContext::new_with_params(model, WhisperContextParameters::default()).map_err(failed)?);
        diagnostics::log(&format!("speech {id}: model loaded in {:.2} s", started.elapsed().as_secs_f32()));
        *loaded = Some(context.clone());
        Ok(context)
    }

    fn failed(e: whisper_rs::WhisperError) -> Failure {
        Failure("failed", format!("The speech model failed: {e}"))
    }

    /// One part, greedy and without timestamps. `before` is the text so far: its end is the prompt, so a part goes on
    /// with the sentence (no capital letter after a cut). The encoder's context fits the part ([`audio_ctx`]): with the
    /// whole 30 s window and timestamps, whisper.cpp kept decoding the empty rest of the window when the speech went on
    /// to the end (a 10 s clip took 15-20 s on the build VM in 0.1.9). A token limit stops a part that repeats itself.
    fn transcribe(context: &WhisperContext, audio: &[f32], language: &str, before: &str) -> Result<String, Failure> {
        let mut state = context.create_state().map_err(failed)?;
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_language(Some(language));
        params.set_n_threads(std::thread::available_parallelism().map_or(4, |n| n.get().min(8)) as i32);
        params.set_audio_ctx(audio_ctx(audio.len()));
        params.set_no_timestamps(true);
        params.set_single_segment(true);
        params.set_temperature_inc(0.0);
        params.set_max_tokens((audio.len() / RATE as usize * 8 + 24) as i32);
        params.set_no_context(true);
        let prompt = before.char_indices().rev().nth(200).map_or(before, |(at, _)| &before[at..]);
        if !prompt.is_empty() {
            params.set_initial_prompt(prompt);
        }
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
    fn a_part_ends_in_the_quietest_moment_of_its_last_two_seconds() {
        let rate = 16_000;
        let loud = |seconds: f32| vec![0.5; (seconds * rate as f32) as usize];
        assert_eq!(part_end(&loud(4.9), rate), None, "less than 5 s waits");
        // 5.5 s of speech with a pause from 4.2 to 4.4 s: the cut is in the pause.
        let mut audio = loud(4.2);
        audio.extend(vec![0.0; 3_200]);
        audio.extend(loud(1.1));
        let end = part_end(&audio, rate).unwrap();
        assert!((67_200..=70_400).contains(&end), "{end}");
        // A pause before the last 2 s is not used.
        let mut audio = vec![0.0; 16_000];
        audio.extend(loud(5.0));
        assert!(part_end(&audio, rate).unwrap() >= audio.len() - 32_000);
    }

    #[test]
    fn short_parts_are_padded_and_the_encoder_fits_the_part() {
        assert_eq!(padded(&[0.3; 8_000]).len(), 17_600);
        assert_eq!(padded(&[0.3; 8_000])[..8_000], [0.3; 8_000]);
        assert_eq!(padded(&[0.3; 80_000]).len(), 80_000);
        assert_eq!(audio_ctx(16_000), 384, "a floor for short parts");
        assert_eq!(audio_ctx(7 * 16_000), 478);
        assert_eq!(audio_ctx(10 * 16_000), 628);
        assert_eq!(audio_ctx(60 * 16_000), 1500, "never more than Whisper's 30 s window");
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
