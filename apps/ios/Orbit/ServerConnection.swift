import SwiftUI
import WebKit

/// Verification and gateway login use WebKit, not URLSession: the gate's cookies
/// must live in the very same per-server data store the Orbit page will use.
@MainActor
final class ServerConnection: NSObject, ObservableObject {
    let server: Server
    let webView: WKWebView
    @Published private(set) var checking = true
    @Published private(set) var needsSignIn = false
    @Published private(set) var pageHost = ""
    @Published private(set) var error: String?

    private let isNew: Bool
    private let onConnected: (Server) -> Void
    private var response: HTTPURLResponse?
    private var generation = 0
    private var stopped = false
    private var connected = false

    private var probeURL: URL { server.url.appending(path: "api/v1/setup/status") }

    init(server: Server, isNew: Bool, onConnected: @escaping (Server) -> Void) {
        self.server = server
        self.isNew = isNew
        self.onConnected = onConnected
        let configuration = WKWebViewConfiguration()
        configuration.websiteDataStore = WKWebsiteDataStore(forIdentifier: server.id)
        webView = WKWebView(frame: .zero, configuration: configuration)
        super.init()
        webView.navigationDelegate = self
        webView.uiDelegate = self
        webView.allowsBackForwardNavigationGestures = true
        #if DEBUG
        webView.isInspectable = true
        #endif
        retry()
    }

    deinit {
        // Cancelled/failed attempts must not leave orphaned persistent stores.
        // Run after this model and its web view have been released.
        if isNew, !connected {
            let id = server.id
            Task { @MainActor in
                WKWebsiteDataStore.remove(forIdentifier: id) { _ in }
            }
        }
    }

    func retry() {
        guard !stopped else { return }
        error = nil
        checking = true
        var request = URLRequest(url: probeURL, cachePolicy: .reloadIgnoringLocalCacheData, timeoutInterval: 15)
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        webView.load(request)
    }

    func openSignInLink(_ input: String) {
        guard !stopped, let url = ServerAddress.signInLink(from: input) else { return }
        generation += 1
        webView.stopLoading()
        error = nil
        checking = true
        // Do not normalize to an origin or send to UIApplication.open: the
        // one-time token and the current WebKit session are both required.
        webView.load(URLRequest(url: url, cachePolicy: .reloadIgnoringLocalCacheData, timeoutInterval: 15))
    }

    func cancel() {
        stopped = true
        generation += 1
        webView.stopLoading()
        webView.navigationDelegate = nil
        webView.uiDelegate = nil
    }

    private func accept(status: Int, url: URL, contentType: String?, body: String, openSignInPage: Bool = false) {
        guard !stopped else { return }
        checking = false
        switch ServerAddress.verification(
            status: status, url: url, contentType: contentType, body: Data(body.utf8), requestedURL: probeURL
        ) {
        case .verified(let origin):
            connected = true
            cancel()
            onConnected(Server(id: server.id, url: origin))
        case .authenticationRequired:
            needsSignIn = true
            error = nil
            if openSignInPage && contentType?.lowercased().hasPrefix("text/html") != true {
                webView.load(URLRequest(url: server.url))
            }
        case .notOrbit:
            error = "This address did not return Orbit's setup status. Check the address and try again."
        case .unavailable(let status):
            error = "The server returned HTTP \(status). Try again when it is available."
        }
    }

    /// A gateway may return to the server's home page rather than to the probe.
    /// Check in an isolated JS world with WebKit's cookies; never export them to
    /// URLSession or mistake the authentication provider for the Orbit origin.
    private func checkFromServerPage(generation token: Int) {
        checking = true
        webView.callAsyncJavaScript("""
            const response = await fetch(path, {
                credentials: 'same-origin', cache: 'no-store',
                headers: {Accept: 'application/json'}, signal: AbortSignal.timeout(15000)
            });
            return {status: response.status, url: response.url,
                    contentType: response.headers.get('Content-Type'), body: await response.text()};
            """, arguments: ["path": ServerAddress.statusPath], in: nil, in: .defaultClient
        ) { [weak self] result in
            guard let self, !self.stopped, self.generation == token else { return }
            self.checking = false
            // A still-gated fetch can fail CORS when redirected to the provider.
            // Keep its sign-in page visible; Check again retries a top-level load.
            guard case .success(let value) = result, let value = value as? [String: Any],
                  let status = value["status"] as? Int,
                  let text = value["url"] as? String, let url = URL(string: text),
                  let body = value["body"] as? String
            else { return }
            self.accept(status: status, url: url, contentType: value["contentType"] as? String, body: body)
        }
    }
}

extension ServerConnection: WKNavigationDelegate {
    func webView(_ webView: WKWebView, didStartProvisionalNavigation navigation: WKNavigation!) {
        generation += 1
        response = nil
        checking = true
        error = nil
    }

    func webView(
        _ webView: WKWebView, decidePolicyFor action: WKNavigationAction,
        decisionHandler: @escaping (WKNavigationActionPolicy) -> Void
    ) {
        guard let url = action.request.url else { return decisionHandler(.cancel) }
        if url.scheme == "about" || ServerAddress.origin(from: url.absoluteString) != nil {
            // Cross-origin HTTPS navigation is allowed only in this explicit,
            // cancellable sign-in sheet. The normal Orbit view keeps its rules.
            return decisionHandler(.allow)
        }
        if ["mailto", "tel"].contains(url.scheme?.lowercased() ?? "") {
            UIApplication.shared.open(url)
        }
        decisionHandler(.cancel)
    }

    func webView(
        _ webView: WKWebView, decidePolicyFor navigationResponse: WKNavigationResponse,
        decisionHandler: @escaping (WKNavigationResponsePolicy) -> Void
    ) {
        if navigationResponse.isForMainFrame {
            response = navigationResponse.response as? HTTPURLResponse
            pageHost = navigationResponse.response.url?.host ?? ""
        }
        decisionHandler(navigationResponse.canShowMIMEType ? .allow : .cancel)
    }

    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        guard !stopped, let response, let url = response.url else { return }
        let token = generation
        if url.path == ServerAddress.statusPath {
            webView.evaluateJavaScript("document.body.innerText", in: nil, in: .defaultClient) { [weak self] result in
                guard let self, !self.stopped, self.generation == token else { return }
                guard case .success(let value) = result, let body = value as? String else {
                    self.checking = false
                    self.error = "Couldn't read the server's response. Try again."
                    return
                }
                self.accept(status: response.statusCode, url: url, contentType: response.mimeType,
                            body: body, openSignInPage: true)
            }
        } else {
            checking = false
            needsSignIn = true
            if ServerAddress.isSameOrigin(url, as: server.url) {
                checkFromServerPage(generation: token)
            }
        }
    }

    func webView(_ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: Error) {
        fail(error)
    }

    func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: Error) {
        fail(error)
    }

    private func fail(_ failure: Error) {
        guard !stopped, (failure as NSError).code != NSURLErrorCancelled else { return }
        checking = false
        error = failure.localizedDescription
    }
}

extension ServerConnection: WKUIDelegate {
    func webView(
        _ webView: WKWebView, createWebViewWith configuration: WKWebViewConfiguration,
        for navigationAction: WKNavigationAction, windowFeatures: WKWindowFeatures
    ) -> WKWebView? {
        // Keep SSO popups in this data store too, rather than sending them to Safari.
        if navigationAction.targetFrame == nil {
            webView.load(navigationAction.request)
        }
        return nil
    }
}

struct ServerConnectionView: View {
    @Environment(\.dismiss) private var dismiss
    @StateObject private var connection: ServerConnection
    @State private var showingSignInLink = false
    @State private var signInLink = ""

    init(server: Server, isNew: Bool, onConnected: @escaping (Server) -> Void) {
        _connection = StateObject(wrappedValue: ServerConnection(server: server, isNew: isNew, onConnected: onConnected))
    }

    var body: some View {
        NavigationStack {
            VStack(spacing: 0) {
                if connection.needsSignIn {
                    VStack(spacing: 4) {
                        Text("Sign in to access \(connection.server.name).")
                        Text("Page: \(connection.pageHost)").font(.caption).foregroundStyle(.secondary)
                        Button("Use email sign-in link") { showingSignInLink = true }
                            .buttonStyle(.bordered)
                    }
                    .padding()
                }
                WebView(webView: connection.webView)
                    .overlay {
                        if let error = connection.error {
                            ContentUnavailableView {
                                Label("Can't connect", systemImage: "wifi.exclamationmark")
                            } description: {
                                Text(error)
                            } actions: {
                                Button("Retry", action: connection.retry)
                            }
                            .background(Color(.systemBackground))
                        } else if !connection.needsSignIn {
                            ProgressView("Checking server…")
                                .frame(maxWidth: .infinity, maxHeight: .infinity)
                                .background(Color(.systemBackground))
                        }
                    }
            }
            .navigationTitle("Connect to server")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { connection.cancel(); dismiss() }
                }
                ToolbarItem(placement: .confirmationAction) {
                    if connection.checking {
                        ProgressView()
                    } else {
                        Button("Check again", action: connection.retry)
                    }
                }
            }
        }
        .alert("Open email sign-in link", isPresented: $showingSignInLink) {
            TextField("https://…", text: $signInLink)
                .keyboardType(.URL)
                .textContentType(.URL)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .privacySensitive()
            Button("Open link") {
                let link = signInLink
                signInLink = ""
                connection.openSignInLink(link)
            }
            .disabled(ServerAddress.signInLink(from: signInLink) == nil)
            Button("Cancel", role: .cancel) { signInLink = "" }
        } message: {
            Text("Paste the full HTTPS link from your sign-in email. It will open here in the same sign-in session, not in Safari.")
        }
        .onDisappear {
            signInLink = ""
            connection.cancel()
        }
    }
}
