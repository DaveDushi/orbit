import SwiftUI
import WebKit

struct ServerView: View {
    @EnvironmentObject private var store: ServerStore
    @StateObject private var session: WebSession

    init(server: Server) {
        _session = StateObject(wrappedValue: WebSession(server: server))
    }

    var body: some View {
        // Only the container safe area is ignored: the keyboard still shrinks the page, as
        // `interactive-widget=resizes-content` asks.
        WebView(webView: session.webView)
            .ignoresSafeArea(.container)
            .safeAreaInset(edge: .top, spacing: 0) {
                if let error = session.sharingError {
                    Text(error).font(.caption).padding(8)
                        .frame(maxWidth: .infinity)
                        .background(Color(.secondarySystemBackground))
                }
            }
            .overlay {
                if let message = session.loadError {
                    ContentUnavailableView {
                        Label("Can't reach \(session.server.name)", systemImage: "wifi.exclamationmark")
                    } description: {
                        Text(message)
                    } actions: {
                        Button("Retry", action: session.retry)
                            .buttonStyle(.borderedProminent)
                        Button("Change server", action: store.leave)
                    }
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .background(Color(.systemBackground))
                }
            }
    }
}

struct WebView: UIViewRepresentable {
    let webView: WKWebView

    func makeUIView(context: Context) -> WKWebView { webView }

    func updateUIView(_ webView: WKWebView, context: Context) {}
}
