//! Windows: voice input with Windows' own speech recognition (`Windows.Media.SpeechRecognition`, dictation).
//!
//! WebView2's `webkitSpeechRecognition` refuses with `not-allowed` in the app and never even asks for the
//! microphone (0.1.6 on a Windows 11 PC: no permission request in the diagnostics log, no microphone use in
//! Windows' "Recent activity"). So the bridge script gives the page a `SpeechRecognition` that runs here: the page
//! sends start, stop and abort; the app sends back the words (`partial`, `final`), an `error` with a Web Speech
//! error code and a text for the person, and `end`.
//!
//! Dictation needs Windows' "Online speech recognition" (Settings → Privacy & security → Speech): without it Windows
//! refuses with SPERR_SPEECH_PRIVACY_POLICY_NOT_ACCEPTED, and the page says where to turn it on. Every step goes to
//! the diagnostics log, never the words.

// The page's commands are read everywhere; only Windows runs them.
#![cfg_attr(not(windows), allow(dead_code))]

/// A command from the page's `SpeechRecognition` (`assets/bridge.js`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Start { id: u64, lang: String },
    Stop { id: u64 },
    Abort { id: u64 },
}

/// SPERR_SPEECH_PRIVACY_POLICY_NOT_ACCEPTED: "Online speech recognition" is off.
const PRIVACY_NOT_ACCEPTED: u32 = 0x8004_5509;
/// E_ACCESSDENIED: Windows refuses the microphone to the app.
const ACCESS_DENIED: u32 = 0x8007_0005;

/// A Windows error as a Web Speech error code and a text for the person.
pub fn error_for(code: u32) -> (&'static str, String) {
    match code {
        PRIVACY_NOT_ACCEPTED => (
            "service-not-allowed",
            "Windows speech recognition is off: turn on Windows Settings → Privacy & security → Speech → “Online speech recognition”, then try again.".into(),
        ),
        ACCESS_DENIED => (
            "not-allowed",
            "Windows refused the microphone: turn on Windows Settings → Privacy & security → Microphone → “Microphone access” and “Let apps access your microphone”, then try again.".into(),
        ),
        code => ("failed", format!("Windows speech recognition failed (0x{code:08X}).")),
    }
}

/// A `SpeechRecognitionResultStatus` that is not a normal end (stop, cancel, silence), as an error for the page.
pub fn status_error(status: i32, lang: &str) -> Option<(&'static str, String)> {
    Some(match status {
        // Success, UserCanceled, TimeoutExceeded (silence), PauseLimitExceeded
        0 | 5 | 7 | 8 => return None,
        1 => ("language-not-supported", format!("Windows has no dictation for {lang}: add the language with speech recognition in Windows Settings → Time & language → Language & region.")),
        9 => ("network", "Windows' speech service could not be reached.".into()),
        10 => ("audio-capture", "Windows found no microphone it may use: connect one, and check Windows Settings → Privacy & security → Microphone.".into()),
        4 => ("failed", "The sound was too poor to recognize. Try again closer to the microphone.".into()),
        status => ("failed", format!("Windows speech recognition stopped (status {status}).")),
    })
}

/// The dictation language for the page's `lang` (`de-CH`): the same tag, else the same language in another region,
/// else Windows' own speech language.
pub fn pick_language(wanted: &str, supported: &[String], system: &str) -> String {
    let base = |tag: &str| tag.split('-').next().unwrap_or_default().to_ascii_lowercase();
    supported
        .iter()
        .find(|tag| tag.eq_ignore_ascii_case(wanted))
        .or_else(|| supported.iter().find(|tag| !wanted.is_empty() && base(tag) == base(wanted)))
        .map_or_else(|| system.to_owned(), Clone::clone)
}

#[cfg(windows)]
pub use native::Dictation;

#[cfg(windows)]
mod native {
    use std::sync::{Arc, mpsc};
    use std::time::Duration;

    use serde_json::{Value, json};
    use windows::Foundation::TypedEventHandler;
    use windows::Globalization::Language;
    use windows::Media::SpeechRecognition::{
        ISpeechRecognitionConstraint, SpeechContinuousRecognitionCompletedEventArgs, SpeechContinuousRecognitionResultGeneratedEventArgs,
        SpeechContinuousRecognitionSession, SpeechRecognitionConfidence, SpeechRecognitionHypothesisGeneratedEventArgs,
        SpeechRecognitionResultStatus, SpeechRecognitionScenario, SpeechRecognitionTopicConstraint, SpeechRecognizer,
    };
    use windows::core::{HSTRING, Interface};

    use super::{Command, error_for, pick_language, status_error};
    use crate::diagnostics;

    type Sender = Arc<dyn Fn(Value) + Send + Sync>;

    enum Control {
        Stop,
        Abort,
        /// Windows ended the session (a stop, silence or an error).
        Ended,
    }

    /// The running dictation: one at a time, on its own thread (WinRT's waits must not block the window).
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
                    let ended = control.clone();
                    let send: Sender = Arc::new(send);
                    std::thread::spawn(move || run(id, &lang, send, ended, commands));
                    self.running = Some((id, control));
                }
                Command::Stop { id } => {
                    if let Some((running, control)) = &self.running
                        && *running == id
                    {
                        let _ = control.send(Control::Stop);
                    }
                }
                Command::Abort { id } => {
                    if self.running.as_ref().is_some_and(|(running, _)| *running == id)
                        && let Some((_, control)) = self.running.take()
                    {
                        let _ = control.send(Control::Abort);
                    }
                }
            }
        }
    }

    struct Failure(&'static str, String);

    impl From<windows::core::Error> for Failure {
        fn from(e: windows::core::Error) -> Self {
            let code = e.code().0 as u32;
            let (error, message) = error_for(code);
            diagnostics::log(&format!("speech: Windows error 0x{code:08X}: {}", e.message()));
            Failure(error, message)
        }
    }

    fn run(id: u64, lang: &str, send: Sender, ended: mpsc::Sender<Control>, commands: mpsc::Receiver<Control>) {
        let (recognizer, session) = match start(id, lang, &send, ended) {
            Ok(started) => started,
            Err(Failure(error, message)) => {
                diagnostics::log(&format!("speech {id}: not started: {error}"));
                send(json!({ "id": id, "type": "error", "error": error, "message": message }));
                send(json!({ "id": id, "type": "end" }));
                return;
            }
        };
        match commands.recv() {
            Ok(Control::Stop) => {
                diagnostics::log(&format!("speech {id}: stop"));
                if let Err(e) = session.StopAsync().and_then(|a| a.join()) {
                    diagnostics::log(&format!("speech {id}: stop failed: {e}"));
                }
                // The last words and `Completed` follow the stop; do not wait forever for them.
                if !matches!(commands.recv_timeout(Duration::from_secs(5)), Ok(Control::Ended)) {
                    send(json!({ "id": id, "type": "end" }));
                }
            }
            Ok(Control::Abort) | Err(_) => {
                diagnostics::log(&format!("speech {id}: abort"));
                let _ = session.CancelAsync().and_then(|a| a.join());
            }
            Ok(Control::Ended) => {}
        }
        let _ = recognizer.Close();
    }

    fn start(id: u64, lang: &str, send: &Sender, ended: mpsc::Sender<Control>) -> Result<(SpeechRecognizer, SpeechContinuousRecognitionSession), Failure> {
        let supported = SpeechRecognizer::SupportedTopicLanguages()?;
        let mut tags = Vec::new();
        for i in 0..supported.Size()? {
            tags.push(supported.GetAt(i)?.LanguageTag()?.to_string());
        }
        let system = SpeechRecognizer::SystemSpeechLanguage()?.LanguageTag()?.to_string();
        let tag = pick_language(lang, &tags, &system);
        diagnostics::log(&format!("speech {id}: start, page language {lang}, dictation in {tag} (Windows has: {})", tags.join(" ")));
        if tags.is_empty() {
            return Err(Failure("language-not-supported", "Windows has no dictation language: add a language with speech recognition in Windows Settings → Time & language → Language & region.".into()));
        }

        let recognizer = SpeechRecognizer::Create(&Language::CreateLanguage(&HSTRING::from(&tag))?)?;
        let topic = SpeechRecognitionTopicConstraint::Create(SpeechRecognitionScenario::Dictation, &HSTRING::from("dictation"))?;
        recognizer.Constraints()?.Append(&topic.cast::<ISpeechRecognitionConstraint>()?)?;
        let compiled = recognizer.CompileConstraintsAsync()?.join()?.Status()?;
        diagnostics::log(&format!("speech {id}: dictation compiled, status {}", compiled.0));
        if let Some((error, message)) = status_error(compiled.0, &tag) {
            return Err(Failure(error, message));
        }

        let partial = send.clone();
        recognizer.HypothesisGenerated(&TypedEventHandler::<SpeechRecognizer, SpeechRecognitionHypothesisGeneratedEventArgs>::new(move |_, args| {
            let text = args.ok()?.Hypothesis()?.Text()?.to_string();
            partial(json!({ "id": id, "type": "partial", "text": text }));
            Ok(())
        }))?;
        let session = recognizer.ContinuousRecognitionSession()?;
        let words = send.clone();
        session.ResultGenerated(&TypedEventHandler::<SpeechContinuousRecognitionSession, SpeechContinuousRecognitionResultGeneratedEventArgs>::new(
            move |_, args| {
                let result = args.ok()?.Result()?;
                if result.Status()? == SpeechRecognitionResultStatus::Success && result.Confidence()? != SpeechRecognitionConfidence::Rejected {
                    words(json!({ "id": id, "type": "final", "text": result.Text()?.to_string() }));
                }
                Ok(())
            },
        ))?;
        let done = send.clone();
        let tag_done = tag.clone();
        session.Completed(&TypedEventHandler::<SpeechContinuousRecognitionSession, SpeechContinuousRecognitionCompletedEventArgs>::new(
            move |_, args| {
                let status = args.ok()?.Status()?.0;
                diagnostics::log(&format!("speech {id}: ended, status {status}"));
                if let Some((error, message)) = status_error(status, &tag_done) {
                    done(json!({ "id": id, "type": "error", "error": error, "message": message }));
                }
                done(json!({ "id": id, "type": "end" }));
                let _ = ended.send(Control::Ended);
                Ok(())
            },
        ))?;
        session.StartAsync()?.join()?;
        diagnostics::log(&format!("speech {id}: listening"));
        Ok((recognizer, session))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dictation_language_follows_the_page_then_its_language_then_windows() {
        let supported = ["en-US".to_owned(), "de-DE".to_owned(), "fr-FR".to_owned()];
        assert_eq!(pick_language("en-us", &supported, "en-US"), "en-US");
        assert_eq!(pick_language("de-CH", &supported, "en-US"), "de-DE");
        assert_eq!(pick_language("he-IL", &supported, "en-US"), "en-US");
        assert_eq!(pick_language("", &supported, "fr-FR"), "fr-FR");
    }

    #[test]
    fn windows_errors_tell_the_person_what_to_turn_on() {
        let (error, message) = error_for(0x8004_5509);
        assert_eq!(error, "service-not-allowed");
        assert!(message.contains("Online speech recognition"));
        assert_eq!(error_for(0x8007_0005).0, "not-allowed");
        assert_eq!(error_for(0x8000_4005).1, "Windows speech recognition failed (0x80004005).");
    }

    #[test]
    fn a_stop_cancel_or_silence_is_a_normal_end() {
        for normal in [0, 5, 7, 8] {
            assert_eq!(status_error(normal, "en-US"), None);
        }
        assert_eq!(status_error(1, "de-CH").unwrap().0, "language-not-supported");
        assert_eq!(status_error(9, "en-US").unwrap().0, "network");
        assert_eq!(status_error(10, "en-US").unwrap().0, "audio-capture");
    }
}
