//! Windows: the microphone for voice input (the chat composer's `webkitSpeechRecognition`).
//!
//! WebView2 keeps a microphone decision per origin in the profile. Without an answer from the app it asks with its
//! own prompt, and a "Block" there stays saved: the page then only gets `not-allowed`, and nothing in the app
//! could undo it. So the app allows the microphone for the server: it saves "allow" for the server's origin at
//! each start (this also replaces a saved "block"), and answers the requests of the server's own pages. Other
//! origins (frames of embedded apps, other sites) keep WebView2's own prompt.
//! Windows' own switch (Settings → Privacy & security → Microphone → "Let desktop apps access your microphone")
//! still decides above this. The speech service itself is chosen by the browser arguments (`signin::WEBVIEW2_ARGS`).

use orbit_client::ServerUrl;
use webview2_com::Microsoft::Web::WebView2::Win32::{
    COREWEBVIEW2_PERMISSION_KIND, COREWEBVIEW2_PERMISSION_KIND_MICROPHONE, COREWEBVIEW2_PERMISSION_STATE_ALLOW, ICoreWebView2_13,
    ICoreWebView2Profile4,
};
use webview2_com::{CoTaskMemPWSTR, PermissionRequestedEventHandler, SetPermissionStateCompletedHandler};
use windows::core::{HSTRING, Interface, PWSTR};
use wry::{WebView, WebViewExtWindows};

use crate::route;

pub fn allow(webview: &WebView, server: &ServerUrl) {
    if let Err(e) = save_allow(webview, server) {
        eprintln!("orbit-desktop: cannot save the microphone permission: {e}");
    }
    if let Err(e) = answer_requests(webview, server.clone()) {
        eprintln!("orbit-desktop: cannot answer microphone requests: {e}");
    }
}

/// `ICoreWebView2Profile4::SetPermissionState` (WebView2 runtime 1.0.1938 and later).
fn save_allow(webview: &WebView, server: &ServerUrl) -> windows::core::Result<()> {
    unsafe {
        let profile = webview.webview().cast::<ICoreWebView2_13>()?.Profile()?.cast::<ICoreWebView2Profile4>()?;
        profile.SetPermissionState(
            COREWEBVIEW2_PERMISSION_KIND_MICROPHONE,
            &HSTRING::from(server.origin()),
            COREWEBVIEW2_PERMISSION_STATE_ALLOW,
            &SetPermissionStateCompletedHandler::create(Box::new(|result| {
                if let Err(e) = result {
                    eprintln!("orbit-desktop: the microphone permission was not saved: {e}");
                }
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
                if kind != COREWEBVIEW2_PERMISSION_KIND_MICROPHONE {
                    return Ok(());
                }
                let mut uri = PWSTR::null();
                args.Uri(&mut uri)?;
                if route::orbit_page(&server, &CoTaskMemPWSTR::from(uri).to_string()) {
                    args.SetState(COREWEBVIEW2_PERMISSION_STATE_ALLOW)?;
                }
                Ok(())
            })),
            &mut token,
        )
    }
}
