import Foundation
import OrbitSharing
import UIKit
import WebKit

/// Keeps the native extension's credential snapshot in step with this server's
/// WebKit cookies, including HttpOnly Orbit and gateway cookies. No script bridge.
@MainActor
final class ShareSessionSync: NSObject, WKHTTPCookieStoreObserver {
    private let server: Server
    private let cookies: WKHTTPCookieStore
    private let report: (String?) -> Void
    private var generation = 0
    private var foreground: NSObjectProtocol?

    init(server: Server, cookies: WKHTTPCookieStore, report: @escaping (String?) -> Void) {
        self.server = server
        self.cookies = cookies
        self.report = report
        super.init()
        cookies.add(self)
        foreground = NotificationCenter.default.addObserver(
            forName: UIApplication.didBecomeActiveNotification, object: nil, queue: .main
        ) { [weak self] _ in
            Task { @MainActor in self?.refresh() }
        }
        refresh()
    }

    deinit {
        if let foreground { NotificationCenter.default.removeObserver(foreground) }
        // WKHTTPCookieStore holds its observers weakly.
    }

    func cookiesDidChange(in cookieStore: WKHTTPCookieStore) { refresh() }

    func refresh() {
        generation += 1
        let token = generation
        cookies.getAllCookies { [weak self] cookies in
            guard let self, token == self.generation else { return }
            guard ServerStore.shared.servers.contains(where: { $0.id == self.server.id }) else {
                try? ShareAccountStore.remove(self.server.id)
                return
            }
            let endpoint = self.server.url.appending(path: "api/v1/auth/me")
            let matching = cookies.map(ShareCookie.init).filter { $0.belongs(to: self.server.url) }
            do {
                if matching.contains(where: { ["__Host-orbit_session", "orbit_session_dev"].contains($0.name) && $0.matches(endpoint) }) {
                    try ShareAccountStore.save(ShareAccount(id: self.server.id, url: self.server.url, cookies: matching))
                } else {
                    // Signing out must remove the extension's access too.
                    try ShareAccountStore.remove(self.server.id)
                }
                self.report(nil)
            } catch {
                self.report(error.localizedDescription)
            }
        }
    }
}
