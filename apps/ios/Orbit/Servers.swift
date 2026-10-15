import Foundation
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

    private let defaults = UserDefaults.standard

    private init() {
        if let data = defaults.data(forKey: "servers"), let saved = try? JSONDecoder().decode([Server].self, from: data) {
            servers = saved
        }
        let currentID = defaults.string(forKey: "currentServer").flatMap(UUID.init(uuidString:))
        current = servers.first { $0.id == currentID }
    }

    func open(_ url: URL) {
        let server = servers.first { $0.url == url } ?? Server(id: UUID(), url: url)
        servers.removeAll { $0.id == server.id }
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

enum ServerAddress {
    /// The origin a typed address names, or `nil`. `https` is assumed; plain `http` is allowed only for local hosts,
    /// which is also all App Transport Security allows. Any path is dropped: Orbit is served from the root.
    static func origin(from input: String) -> URL? {
        var text = input.trimmingCharacters(in: .whitespacesAndNewlines)
        if !text.contains("://") {
            text = "https://" + text
        }
        guard let components = URLComponents(string: text),
              let scheme = components.scheme?.lowercased(),
              let host = components.host?.lowercased(), !host.isEmpty,
              scheme == "https" || (scheme == "http" && isLocal(host))
        else { return nil }
        var origin = URLComponents()
        origin.scheme = scheme
        origin.host = host
        origin.port = components.port == defaultPort(scheme) ? nil : components.port
        return origin.url
    }

    static func isSameOrigin(_ url: URL, as origin: URL) -> Bool {
        guard let a = URLComponents(url: url, resolvingAgainstBaseURL: false),
              let b = URLComponents(url: origin, resolvingAgainstBaseURL: false),
              let scheme = a.scheme?.lowercased(), scheme == b.scheme
        else { return false }
        return a.host?.lowercased() == b.host
            && (a.port ?? defaultPort(scheme)) == (b.port ?? defaultPort(scheme))
    }

    /// The origin of the Orbit server an address leads to, after redirects (`example.com` to `www.example.com`).
    /// The web view opens other origins in Safari, so the app must save the one that answered. Throws unless it
    /// answers like Orbit.
    static func verify(_ origin: URL) async throws -> URL {
        var request = URLRequest(url: origin.appending(path: "api/v1/setup/status"), timeoutInterval: 15)
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        let (data, response) = try await URLSession.shared.data(for: request)
        guard let response = response as? HTTPURLResponse, response.statusCode == 200,
              (try? JSONDecoder().decode(SetupStatus.self, from: data)) != nil,
              let reached = response.url.flatMap({ Self.origin(from: $0.absoluteString) })
        else { throw NotOrbit() }
        return reached
    }

    private struct SetupStatus: Decodable {
        let complete: Bool
    }

    private struct NotOrbit: LocalizedError {
        var errorDescription: String? { "This address does not answer like an Orbit server." }
    }

    private static func isLocal(_ host: String) -> Bool {
        host == "localhost" || host.hasSuffix(".local") || !host.contains(".")
            || host.allSatisfy { $0.isNumber || $0 == "." }
    }

    private static func defaultPort(_ scheme: String) -> Int? {
        switch scheme {
        case "https": 443
        case "http": 80
        default: nil
        }
    }
}
