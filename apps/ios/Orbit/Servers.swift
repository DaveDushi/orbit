import Foundation
import OrbitSharing
import WebKit

/// An Orbit server the app has connected to. `id` also names its website data store, so each server keeps its own
/// cookies and storage.
struct Server: Codable, Identifiable, Hashable {
    let id: UUID
    let url: URL

    var name: String { URLComponents(url: url, resolvingAgainstBaseURL: false)?.host ?? url.absoluteString }
}

@MainActor
final class ServerStore: ObservableObject {
    static let shared = ServerStore()

    /// Most recently opened first.
    @Published private(set) var servers: [Server] = []
    @Published private(set) var current: Server?
    @Published var removalError: String?

    private let defaults = UserDefaults.standard

    private init() {
        if let data = defaults.data(forKey: "servers"), let saved = try? JSONDecoder().decode([Server].self, from: data) {
            servers = saved
        }
        let currentID = defaults.string(forKey: "currentServer").flatMap(UUID.init(uuidString:))
        current = servers.first { $0.id == currentID }
    }

    /// Reuse the server's data store during verification and gateway sign-in.
    func candidate(for url: URL) -> Server {
        servers.first { $0.url == url } ?? Server(id: UUID(), url: url)
    }

    func open(_ url: URL) {
        open(candidate(for: url))
    }

    func open(_ server: Server) {
        servers.removeAll { $0.id == server.id || $0.url == server.url }
        servers.insert(server, at: 0)
        current = server
        save()
    }

    func leave() {
        current = nil
        save()
    }

    /// Forgets a server and deletes its cookies and storage. The data store can only go once no web view uses it,
    /// which holds here because the picker is showing.
    func remove(_ server: Server) {
        // Don't claim the server was forgotten while its extension credential
        // still exists. Leave it available to retry if the keychain is locked.
        do { try ShareAccountStore.remove(server.id) }
        catch { removalError = error.localizedDescription; return }
        servers.removeAll { $0.id == server.id }
        if current?.id == server.id {
            current = nil
        }
        save()
        WKWebsiteDataStore.remove(forIdentifier: server.id) { _ in }
    }

    private func save() {
        defaults.set(try? JSONEncoder().encode(servers), forKey: "servers")
        defaults.set(current?.id.uuidString, forKey: "currentServer")
    }
}
