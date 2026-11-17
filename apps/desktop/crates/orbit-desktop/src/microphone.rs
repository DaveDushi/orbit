//! Windows: the microphone for voice input (the chat composer's `webkitSpeechRecognition`).
//!
//! WebView2 keeps a microphone decision per origin in the profile. Without an answer from the app it asks with its
//! own prompt, and a "Block" there stays saved: the page then only gets `not-allowed`, and nothing in the app
//! could undo it. So the app allows the microphone for the server: it saves "allow" for the server's origin at
//! each start (this also replaces a saved "block"), and answers the requests of the server's own pages. Other
//! origins (frames of embedded apps, other sites) keep WebView2's own prompt.
//! Windows' own switch (Settings → Privacy & security → Microphone → "Let desktop apps access your microphone")
//! still decides above this. The speech service itself is chosen by the browser arguments (`signin::WEBVIEW2_ARGS`).
//! The runtime version, the saved permission and every permission request go to the diagnostics log.

use orbit_client::ServerUrl;
use webview2_com::Microsoft::Web::WebView2::Win32::{
    COREWEBVIEW2_PERMISSION_KIND, COREWEBVIEW2_PERMISSION_KIND_MICROPHONE, COREWEBVIEW2_PERMISSION_STATE_ALLOW, ICoreWebView2_13,
    ICoreWebView2Profile4,
};
use webview2_com::{CoTaskMemPWSTR, PermissionRequestedEventHandler, SetPermissionStateCompletedHandler};
use windows::core::{HSTRING, Interface, PWSTR};
use wry::{WebView, WebViewExtWindows};

use crate::{diagnostics, route};

pub fn allow(webview: &WebView, server: &ServerUrl) {
    diagnostics::log(&format!("WebView2 runtime {}", runtime_version(webview).unwrap_or_else(|e| format!("unknown ({e})"))));
    if let Err(e) = save_allow(webview, server) {
        diagnostics::log(&format!("microphone: cannot save allow for {}: {e}", server.origin()));
    }
    if let Err(e) = answer_requests(webview, server.clone()) {
        diagnostics::log(&format!("microphone: cannot answer permission requests: {e}"));
    }
}

fn runtime_version(webview: &WebView) -> windows::core::Result<String> {
    let mut version = PWSTR::null();
    unsafe { webview.environment().BrowserVersionString(&mut version)? };
    Ok(CoTaskMemPWSTR::from(version).to_string())
}

/// `ICoreWebView2Profile4::SetPermissionState` (WebView2 runtime 1.0.1938 and later).
fn save_allow(webview: &WebView, server: &ServerUrl) -> windows::core::Result<()> {
    unsafe {
        let profile = webview.webview().cast::<ICoreWebView2_13>()?.Profile()?.cast::<ICoreWebView2Profile4>()?;
        let origin = server.origin().to_owned();
        profile.SetPermissionState(
            COREWEBVIEW2_PERMISSION_KIND_MICROPHONE,
            &HSTRING::from(server.origin()),
            COREWEBVIEW2_PERMISSION_STATE_ALLOW,
            &SetPermissionStateCompletedHandler::create(Box::new(move |result| {
                diagnostics::log(&match result {
                    Ok(()) => format!("microphone: saved allow for {origin}"),
                    Err(e) => format!("microphone: allow for {origin} not saved: {e}"),
                });
                Ok(())
            })),
        )
    }
}

fn answer_requests(webview: &WebView, server: ServerUrl) -> windows::core::Result<()> {
    let mut token = 0i64;
    unsafe {
        webview.webview().add_PermissionRequested(
            &PermissionRequestedEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else { return Ok(()) };
                let mut kind = COREWEBVIEW2_PERMISSION_KIND::default();
                args.PermissionKind(&mut kind)?;
                let mut uri = PWSTR::null();
                args.Uri(&mut uri)?;
                let uri = CoTaskMemPWSTR::from(uri).to_string();
                let allow = kind == COREWEBVIEW2_PERMISSION_KIND_MICROPHONE && route::orbit_page(&server, &uri);
                if allow {
                    args.SetState(COREWEBVIEW2_PERMISSION_STATE_ALLOW)?;
                }
                // Every kind: a request that is not the microphone's, or a page that is not the server's, shows here.
                diagnostics::log(&format!("permission request: kind {} at {uri}: {}", kind.0, if allow { "allowed" } else { "left to WebView2" }));
                Ok(())
            })),
            &mut token,
        )
    }
}
