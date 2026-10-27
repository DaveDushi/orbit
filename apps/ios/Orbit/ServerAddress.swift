import Foundation

/// Address normalization and the response contract shared by connection checks.
enum ServerAddress {
    static let statusPath = "/api/v1/setup/status"

    enum Verification: Equatable {
        case verified(URL)
        case authenticationRequired
        case notOrbit
        case unavailable(Int)
    }

    /// Orbit is served from the root. Plain HTTP is accepted only for local hosts.
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

    /// Email links carry one-time tokens in their path/query. Unlike a server
    /// address, preserve the whole URL and require an explicit HTTPS link.
    static func signInLink(from input: String) -> URL? {
        let text = input.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.contains(where: { $0.isWhitespace }),
              let components = URLComponents(string: text),
              components.scheme?.lowercased() == "https",
              let host = components.host, !host.isEmpty,
              components.user == nil, components.password == nil
        else { return nil }
        return components.url
    }

    static func isSameOrigin(_ url: URL, as origin: URL) -> Bool {
        guard let a = URLComponents(url: url, resolvingAgainstBaseURL: false),
              let b = URLComponents(url: origin, resolvingAgainstBaseURL: false),
              let scheme = a.scheme?.lowercased(), scheme == b.scheme
        else { return false }
        return a.host?.lowercased() == b.host
            && (a.port ?? defaultPort(scheme)) == (b.port ?? defaultPort(scheme))
    }

    /// A gateway's login page is not evidence that the address isn't Orbit. It
    /// also isn't evidence that it is: only the setup JSON can verify a server.
    static func verification(
        status: Int,
        url: URL,
        contentType: String?,
        body: Data,
        requestedURL: URL
    ) -> Verification {
        if status == 200, url.path == statusPath,
           (try? JSONDecoder().decode(SetupStatus.self, from: body)) != nil,
           let reached = origin(from: url.absoluteString) {
            return .verified(reached)
        }
        if status == 401 || status == 403 {
            return .authenticationRequired
        }
        if status >= 500 {
            return .unavailable(status)
        }
        if (200..<400).contains(status),
           contentType?.lowercased().hasPrefix("text/html") == true
            || !isSameOrigin(url, as: requestedURL) || url.path != requestedURL.path {
            return .authenticationRequired
        }
        return .notOrbit
    }

    private struct SetupStatus: Decodable {
        let complete: Bool
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
