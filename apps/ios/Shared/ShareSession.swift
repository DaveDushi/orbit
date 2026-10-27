import Foundation
#if canImport(FoundationNetworking)
import FoundationNetworking
#endif
#if canImport(Security)
import Security
#endif

/// Only cookies for this server are handed to its share requests. No Safari or
/// other server's cookies are read, and credentials never enter web JavaScript.
public struct ShareCookie: Codable {
    public let name: String
    public let value: String
    public let domain: String
    public let path: String
    public let secure: Bool
    public let expires: Date?

    public init(_ cookie: HTTPCookie) {
        name = cookie.name
        value = cookie.value
        domain = cookie.domain
        path = cookie.path
        secure = cookie.isSecure
        expires = cookie.expiresDate
    }

    public func belongs(to url: URL, now: Date = Date()) -> Bool {
        guard let host = url.host?.lowercased(), !secure || url.scheme?.lowercased() == "https",
              expires.map({ $0 > now }) ?? true else { return false }
        let domain = domain.lowercased()
        if domain.hasPrefix(".") {
            let suffix = String(domain.dropFirst())
            guard host == suffix || host.hasSuffix("." + suffix) else { return false }
        } else if host != domain {
            return false
        }
        return true
    }

    public func matches(_ url: URL, now: Date = Date()) -> Bool {
        guard belongs(to: url, now: now) else { return false }
        let requestPath = url.path.isEmpty ? "/" : url.path
        return requestPath == path || (requestPath.hasPrefix(path)
            && (path.hasSuffix("/") || requestPath.dropFirst(path.count).hasPrefix("/")))
    }
}

public struct ShareAccount: Codable, Identifiable {
    public let id: UUID
    public let url: URL
    public let cookies: [ShareCookie]
    public var name: String { url.host ?? url.absoluteString }

    public init(id: UUID, url: URL, cookies: [ShareCookie]) {
        self.id = id
        self.url = url
        self.cookies = cookies
    }

    public func cookieHeader(for request: URL) -> String? {
        guard Self.sameOrigin(request, url) else { return nil }
        let matching = cookies.filter { $0.matches(request) }.sorted { $0.path.count > $1.path.count }
        return matching.isEmpty ? nil : matching.map { "\($0.name)=\($0.value)" }.joined(separator: "; ")
    }

    public static func sameOrigin(_ a: URL, _ b: URL) -> Bool {
        guard let scheme = a.scheme?.lowercased(), ["http", "https"].contains(scheme),
              scheme == b.scheme?.lowercased(), let host = a.host?.lowercased(), !host.isEmpty
        else { return false }
        let defaultPort = scheme == "https" ? 443 : 80
        return host == b.host?.lowercased() && (a.port ?? defaultPort) == (b.port ?? defaultPort)
    }
}

#if canImport(Security)
/// Both targets put the same, explicit keychain group FIRST in their signing
/// entitlements. Using the default group avoids baking an Apple team ID into
/// source. Items are device-only and readable only while the phone is unlocked.
public enum ShareAccountStore {
    private static var query: [String: Any] {
        [kSecClass as String: kSecClassGenericPassword,
         kSecAttrService as String: "dev.orbit.share-session"]
    }

    public static func accounts() throws -> [ShareAccount] {
        var request = query
        request[kSecReturnData as String] = true
        request[kSecMatchLimit as String] = kSecMatchLimitAll
        var result: CFTypeRef?
        let status = SecItemCopyMatching(request as CFDictionary, &result)
        if status == errSecItemNotFound { return [] }
        guard status == errSecSuccess else { throw KeychainError(status: status) }
        return try (result as? [Data] ?? []).map { try JSONDecoder().decode(ShareAccount.self, from: $0) }
            .sorted { $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending }
    }

    public static func save(_ account: ShareAccount) throws {
        var request = query
        request[kSecAttrAccount as String] = account.id.uuidString
        let data = try JSONEncoder().encode(account)
        let status = SecItemUpdate(request as CFDictionary, [kSecValueData as String: data] as CFDictionary)
        if status == errSecItemNotFound {
            request[kSecValueData as String] = data
            request[kSecAttrAccessible as String] = kSecAttrAccessibleWhenUnlockedThisDeviceOnly
            let added = SecItemAdd(request as CFDictionary, nil)
            guard added == errSecSuccess else { throw KeychainError(status: added) }
        } else if status != errSecSuccess {
            throw KeychainError(status: status)
        }
    }

    public static func remove(_ id: UUID) throws {
        var request = query
        request[kSecAttrAccount as String] = id.uuidString
        let status = SecItemDelete(request as CFDictionary)
        guard status == errSecSuccess || status == errSecItemNotFound else { throw KeychainError(status: status) }
    }

    public struct KeychainError: LocalizedError {
        let status: OSStatus
        public var errorDescription: String? {
            "Orbit couldn't access its shared sign-in (keychain error \(status)). Unlock the phone and open Orbit. If this continues, reinstall with the shared keychain signing entitlement."
        }
    }
}
#endif
