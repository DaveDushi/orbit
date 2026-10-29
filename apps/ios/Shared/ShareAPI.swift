import Foundation
#if canImport(FoundationNetworking)
import FoundationNetworking
#endif

public struct ShareRecord: Decodable, Identifiable {
    public let id: UUID
    public let name: String
}

public struct ShareUser: Decodable { public let id: UUID }
public struct ShareMember: Decodable {
    public let userId: UUID
    public let displayName: String
}
public struct ShareProject: Decodable {
    public let id: UUID
    public let key: String
}
public struct ShareTask: Decodable, Identifiable {
    public let id: UUID
    public let projectId: UUID
    public let number: Int
    public let title: String
}
public struct SharePage<Item: Decodable>: Decodable {
    public let items: [Item]
    public let nextCursor: String?
}
public struct ShareConversation: Decodable, Identifiable {
    public let id: UUID
    public let kind: String
    public let name: String
    public let memberIds: [UUID]
    public let selfDm: Bool
    public let archived: Bool

    public func canSend(as user: UUID) -> Bool { !archived && memberIds.contains(user) }

    public func title(for user: UUID, members: [ShareMember]) -> String {
        if kind != "dm" { return "#" + name }
        if selfDm { return "Notes to self" }
        let names = memberIds.filter { $0 != user }.map { id in
            members.first { $0.userId == id }?.displayName ?? "Former member"
        }
        return names.isEmpty ? "Direct message" : names.joined(separator: ", ")
    }
}

public enum ShareKind: String, CaseIterable, Identifiable {
    case chat, task
    public var id: String { rawValue }
    public var title: String { self == .chat ? "Chat / DM" : "Task" }
    public var symbol: String { self == .chat ? "bubble.left.and.bubble.right" : "checkmark.circle" }
}

public struct ShareDestination: Identifiable {
    public let id: UUID
    public let title: String
    public init(id: UUID, title: String) { self.id = id; self.title = title }
}

public enum ShareAPIError: LocalizedError {
    case signIn, invalidResponse, invalidDestination, unsafeRetry, server(Int)
    public var errorDescription: String? {
        switch self {
        case .signIn: "Your sign-in has expired or the server requires authentication. Open Orbit, sign in, then share again."
        case .invalidResponse: "The server didn't return an Orbit response. Open Orbit and check the connection."
        case .invalidDestination: "This share destination or selection isn't valid."
        case .unsafeRetry: "Check the task in Orbit before sharing again; the last upload may already have completed."
        case .server(let status): "Orbit couldn't complete the request (HTTP \(status)). Check your access and the server's upload limits."
        }
    }
}

/// A private, cookie-less URLSession. Credentials are added only to the selected
/// server's requests. Redirects are NEVER followed: an expired gateway must not
/// receive a photo upload at its login endpoint (or forward it to another host).
public final class ShareAPI: ShareTransport {
    private let account: ShareAccount
    private let session: URLSession
    private let redirects = RejectShareRedirects()
    private let decoder: JSONDecoder = {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        return decoder
    }()

    public init(account: ShareAccount, configuration: URLSessionConfiguration = .ephemeral) {
        self.account = account
        configuration.httpCookieStorage = nil
        configuration.httpShouldSetCookies = false
        configuration.urlCache = nil
        configuration.requestCachePolicy = .reloadIgnoringLocalCacheData
        configuration.timeoutIntervalForRequest = 30
        configuration.timeoutIntervalForResource = 120
        session = URLSession(configuration: configuration, delegate: redirects, delegateQueue: nil)
    }

    deinit { session.invalidateAndCancel() }
    public func cancel() { session.invalidateAndCancel() }

    public func get<T: Decodable>(_ path: String, query: [URLQueryItem] = []) async throws -> T {
        var request = try request(path)
        var components = URLComponents(url: request.url!, resolvingAgainstBaseURL: false)!
        if !query.isEmpty { components.queryItems = query }
        request.url = components.url
        return try decoder.decode(T.self, from: await perform(request))
    }

    public func post(_ path: String, json: [String: Any]) async throws {
        var request = try request(path)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.httpBody = try JSONSerialization.data(withJSONObject: json)
        let data = try await perform(request)
        struct Receipt: Decodable { let result: ShareUser }
        // A status code alone isn't proof that the chat message was accepted.
        _ = try decoder.decode(Receipt.self, from: data)
    }

    /// Multipart is written to disk, not assembled with all the photos in memory.
    /// Only one photo is uploaded at a time; the extension has a small memory limit.
    public func upload(_ file: URL, name: String, path: String, query: [URLQueryItem] = []) async throws -> UUID {
        var request = try request(path)
        if !query.isEmpty {
            var components = URLComponents(url: request.url!, resolvingAgainstBaseURL: false)!
            components.queryItems = query
            request.url = components.url
        }
        request.httpMethod = "POST"
        let boundary = "Orbit-" + UUID().uuidString
        request.setValue("multipart/form-data; boundary=\(boundary)", forHTTPHeaderField: "Content-Type")
        let body = file.deletingLastPathComponent().appendingPathComponent(UUID().uuidString + ".multipart")
        defer { try? FileManager.default.removeItem(at: body) }
        let safeName = name.replacingOccurrences(of: "\"", with: "_")
            .replacingOccurrences(of: "\r", with: "_").replacingOccurrences(of: "\n", with: "_")
            .replacingOccurrences(of: "\\", with: "_")
        var attributes: [FileAttributeKey: Any] = [.posixPermissions: 0o600]
        #if os(iOS)
        attributes[.protectionKey] = FileProtectionType.complete
        #endif
        guard FileManager.default.createFile(atPath: body.path, contents: nil, attributes: attributes)
        else { throw CocoaError(.fileWriteUnknown) }
        let output = try FileHandle(forWritingTo: body)
        let input = try FileHandle(forReadingFrom: file)
        do {
            try output.write(contentsOf: Data("--\(boundary)\r\nContent-Disposition: form-data; name=\"file\"; filename=\"\(safeName)\"\r\nContent-Type: image/jpeg\r\n\r\n".utf8))
            while let data = try input.read(upToCount: 64 * 1024), !data.isEmpty {
                try Task.checkCancellation()
                try output.write(contentsOf: data)
            }
            try output.write(contentsOf: Data("\r\n--\(boundary)--\r\n".utf8))
            try output.close()
            try input.close()
        } catch {
            try? output.close(); try? input.close()
            throw error
        }
        let (data, response) = try await session.upload(for: request, fromFile: body)
        try validate(response)
        struct Uploaded: Decodable { let id: UUID }
        // One request uploads one file: accept only a single file record.
        if let record = try? decoder.decode(Uploaded.self, from: data) { return record.id }
        let records = try decoder.decode([Uploaded].self, from: data)
        guard records.count == 1, let record = records.first else { throw ShareAPIError.invalidResponse }
        return record.id
    }

    private func request(_ path: String) throws -> URLRequest {
        guard path.hasPrefix("/api/v1/"), let url = URL(string: path, relativeTo: account.url)?.absoluteURL,
              ShareAccount.sameOrigin(url, account.url), url.user == nil, url.password == nil
        else { throw ShareAPIError.invalidDestination }
        var request = URLRequest(url: url)
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        request.setValue(account.url.absoluteString, forHTTPHeaderField: "Origin")
        request.setValue(account.cookieHeader(for: url), forHTTPHeaderField: "Cookie")
        return request
    }

    private func perform(_ request: URLRequest) async throws -> Data {
        let (data, response) = try await session.data(for: request)
        try validate(response)
        return data
    }

    private func validate(_ response: URLResponse) throws {
        guard let response = response as? HTTPURLResponse, let url = response.url,
              ShareAccount.sameOrigin(url, account.url) else { throw ShareAPIError.invalidResponse }
        if [301, 302, 303, 307, 308, 401, 403].contains(response.statusCode) { throw ShareAPIError.signIn }
        guard (200..<300).contains(response.statusCode) else { throw ShareAPIError.server(response.statusCode) }
        guard response.mimeType == "application/json" else { throw ShareAPIError.invalidResponse }
    }
}

private final class RejectShareRedirects: NSObject, URLSessionTaskDelegate {
    func urlSession(_ session: URLSession, task: URLSessionTask, willPerformHTTPRedirection response: HTTPURLResponse,
                    newRequest request: URLRequest, completionHandler: @escaping (URLRequest?) -> Void) {
        completionHandler(nil)
    }
}
