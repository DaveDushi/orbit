import UIKit
import WebKit

/// The web view of one server and the rules for what it loads: the server's own pages stay in the app, anything
/// else opens in Safari, and files the page downloads go to the share sheet.
@MainActor
final class WebSession: NSObject, ObservableObject {
    let server: Server
    let webView: WKWebView

    /// Why the last page load failed; `nil` while the page is fine.
    @Published private(set) var loadError: String?

    private var downloads: [WKDownload: URL] = [:]

    init(server: Server) {
        self.server = server
        let configuration = WKWebViewConfiguration()
        configuration.websiteDataStore = WKWebsiteDataStore(forIdentifier: server.id)
        // Orbit reads `OrbitiOS/` to tell the app from Safari. `Mobile/…` is WebKit's own default, kept for sites
        // that look for it.
        configuration.applicationNameForUserAgent = "Mobile/15E148 OrbitiOS/\(Self.appVersion)"
        configuration.allowsInlineMediaPlayback = true
        webView = WKWebView(frame: .zero, configuration: configuration)
        super.init()

        webView.navigationDelegate = self
        webView.uiDelegate = self
        webView.allowsBackForwardNavigationGestures = true
        webView.allowsLinkPreview = false
        webView.isOpaque = false
        webView.backgroundColor = .black
        // Orbit pads for the notch and home indicator itself (`viewport-fit=cover` and `env(safe-area-inset-*)`).
        webView.scrollView.contentInsetAdjustmentBehavior = .never
        #if DEBUG
        webView.isInspectable = true
        #endif
        webView.load(URLRequest(url: server.url))
    }

    func retry() {
        loadError = nil
        if webView.url == nil {
            webView.load(URLRequest(url: server.url))
        } else {
            webView.reload()
        }
    }

    private func belongsToServer(_ url: URL) -> Bool {
        ["about", "blob", "data"].contains(url.scheme?.lowercased() ?? "") || ServerAddress.isSameOrigin(url, as: server.url)
    }

    private func fail(with error: Error) {
        let error = error as NSError
        // A load that was replaced by another, or that became a download, is not a failure.
        let replaced = error.domain == NSURLErrorDomain && error.code == NSURLErrorCancelled
        let becameDownload = error.domain == "WebKitErrorDomain" && error.code == 102
        if !replaced && !becameDownload {
            loadError = error.localizedDescription
        }
    }

    private func share(_ file: URL) {
        guard var top = webView.window?.rootViewController else { return }
        while let presented = top.presentedViewController {
            top = presented
        }
        let sheet = UIActivityViewController(activityItems: [file], applicationActivities: nil)
        sheet.popoverPresentationController?.sourceView = webView
        top.present(sheet, animated: true)
    }

    private static var appVersion: String {
        Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "0"
    }
}

extension WebSession: WKNavigationDelegate {
    func webView(
        _ webView: WKWebView,
        decidePolicyFor navigationAction: WKNavigationAction,
        decisionHandler: @escaping (WKNavigationActionPolicy) -> Void
    ) {
        if navigationAction.shouldPerformDownload {
            return decisionHandler(.download)
        }
        // Frames (embeds) load whatever they load; a new window is decided in `createWebViewWith`.
        guard let url = navigationAction.request.url, navigationAction.targetFrame?.isMainFrame == true,
              !belongsToServer(url)
        else { return decisionHandler(.allow) }
        UIApplication.shared.open(url)
        decisionHandler(.cancel)
    }

    func webView(
        _ webView: WKWebView,
        decidePolicyFor navigationResponse: WKNavigationResponse,
        decisionHandler: @escaping (WKNavigationResponsePolicy) -> Void
    ) {
        let disposition = (navigationResponse.response as? HTTPURLResponse)?
            .value(forHTTPHeaderField: "Content-Disposition") ?? ""
        let isAttachment = disposition.lowercased().hasPrefix("attachment")
        decisionHandler(navigationResponse.canShowMIMEType && !isAttachment ? .allow : .download)
    }

    func webView(_ webView: WKWebView, navigationAction: WKNavigationAction, didBecome download: WKDownload) {
        download.delegate = self
    }

    func webView(_ webView: WKWebView, navigationResponse: WKNavigationResponse, didBecome download: WKDownload) {
        download.delegate = self
    }

    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        loadError = nil
    }

    func webView(_ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: Error) {
        fail(with: error)
    }

    func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: Error) {
        fail(with: error)
    }

    func webViewWebContentProcessDidTerminate(_ webView: WKWebView) {
        webView.reload()
    }
}

extension WebSession: WKUIDelegate {
    /// `target="_blank"` and `window.open`: the server's pages load in this web view, other sites open in Safari.
    func webView(
        _ webView: WKWebView,
        createWebViewWith configuration: WKWebViewConfiguration,
        for navigationAction: WKNavigationAction,
        windowFeatures: WKWindowFeatures
    ) -> WKWebView? {
        if let url = navigationAction.request.url {
            if belongsToServer(url) {
                webView.load(navigationAction.request)
            } else {
                UIApplication.shared.open(url)
            }
        }
        return nil
    }
}

extension WebSession: WKDownloadDelegate {
    func download(
        _ download: WKDownload,
        decideDestinationUsing response: URLResponse,
        suggestedFilename: String,
        completionHandler: @escaping (URL?) -> Void
    ) {
        // A folder of its own keeps the server's file name without clashing with an earlier download.
        let folder = FileManager.default.temporaryDirectory.appending(path: UUID().uuidString, directoryHint: .isDirectory)
        do {
            try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        } catch {
            return completionHandler(nil)
        }
        let destination = folder.appending(path: suggestedFilename)
        downloads[download] = destination
        completionHandler(destination)
    }

    func downloadDidFinish(_ download: WKDownload) {
        if let file = downloads.removeValue(forKey: download) {
            share(file)
        }
    }

    func download(_ download: WKDownload, didFailWithError error: Error, resumeData: Data?) {
        downloads.removeValue(forKey: download)
    }
}
