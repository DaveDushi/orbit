import Foundation
#if canImport(FoundationNetworking)
import FoundationNetworking
#endif
import XCTest

final class ShareTests: XCTestCase {
    private let origin = URL(string: "https://orbit.example.com")!

    private func cookie(domain: String = "orbit.example.com", path: String = "/", secure: Bool = true,
                        expires: Date? = nil) -> ShareCookie {
        var properties: [HTTPCookiePropertyKey: Any] = [.name: "session", .value: "test-only", .domain: domain, .path: path]
        if secure { properties[.secure] = "TRUE" }
        if let expires { properties[.expires] = expires }
        return ShareCookie(HTTPCookie(properties: properties)!)
    }

    func testCookieBoundaries() {
        let endpoint = origin.appendingPathComponent("api/v1/workspaces")
        XCTAssertTrue(cookie().matches(endpoint))
        XCTAssertFalse(cookie(domain: "exe.dev").matches(endpoint))
        XCTAssertFalse(cookie().matches(URL(string: "https://other.orbit.example.com/api")!))
        XCTAssertFalse(cookie().matches(URL(string: "https://orbit.example.com.attacker.test/api")!))
        XCTAssertTrue(cookie(domain: ".example.com").matches(endpoint))
        XCTAssertFalse(cookie(domain: ".otherexample.com").matches(endpoint))
        XCTAssertFalse(cookie().matches(URL(string: "http://orbit.example.com/api")!))
        XCTAssertFalse(cookie(expires: Date(timeIntervalSince1970: 1)).matches(endpoint))
        XCTAssertTrue(cookie(path: "/api").matches(endpoint))
        XCTAssertFalse(cookie(path: "/api/v1/other").matches(endpoint))
        XCTAssertFalse(cookie(path: "/ap").matches(endpoint))
    }

    func testAccountNeverSendsCookiesToAnotherOrigin() {
        let account = ShareAccount(id: UUID(), url: origin, cookies: [cookie()])
        XCTAssertEqual(account.cookieHeader(for: origin.appendingPathComponent("api/v1/workspaces")), "session=test-only")
        XCTAssertNil(account.cookieHeader(for: URL(string: "https://exe.dev/auth")!))
        XCTAssertNil(account.cookieHeader(for: URL(string: "https://orbit.example.com:8443/api")!))
        XCTAssertNil(account.cookieHeader(for: URL(string: "http://orbit.example.com/api")!))
    }

    func testWireShapesAndWritableDestinations() throws {
        let id = UUID(), other = UUID(), conversation = UUID(), task = UUID(), project = UUID()
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        let data = Data("""
            [{"id":"\(conversation)","kind":"dm","name":"","member_ids":["\(id)","\(other)"],"self_dm":false,"archived":false}]
            """.utf8)
        let records = try decoder.decode([ShareConversation].self, from: data)
        let members = try decoder.decode([ShareMember].self, from: Data("""
            [{"user_id":"\(other)","display_name":"Recipient"}]
            """.utf8))
        XCTAssertTrue(records[0].canSend(as: id))
        XCTAssertFalse(records[0].canSend(as: UUID()))
        XCTAssertEqual(records[0].title(for: id, members: members), "Recipient")
        let page = try decoder.decode(SharePage<ShareTask>.self, from: Data("""
            {"items":[{"id":"\(task)","project_id":"\(project)","number":42,"title":"Real task"}],"next_cursor":"next"}
            """.utf8))
        XCTAssertEqual(page.items.first?.id, task)
        XCTAssertEqual(page.nextCursor, "next")
        let archived = try decoder.decode([ShareConversation].self, from: Data(String(decoding: data, as: UTF8.self)
            .replacingOccurrences(of: "\"archived\":false", with: "\"archived\":true").utf8))
        XCTAssertFalse(archived[0].canSend(as: id))
    }

    @MainActor
    func testChatUploadsThenPostsAndRetryReusesNonce() async throws {
        let transport = FakeShareTransport()
        transport.failPosts = 1
        let workspace = UUID(), conversation = UUID()
        let delivery = ShareDelivery(transport: transport, kind: .chat, workspaceID: workspace,
                                     destinationID: conversation, caption: "Photo caption", photos: photos(2))
        do { try await delivery.send(); XCTFail("Expected lost acknowledgement") } catch {}
        XCTAssertEqual(delivery.uploaded, 2)
        XCTAssertFalse(delivery.sent)
        try await delivery.send()
        try await delivery.send() // Completing twice must not send again either.
        XCTAssertTrue(delivery.sent)
        XCTAssertEqual(transport.uploads.count, 2)
        XCTAssertEqual(transport.posts.count, 2)
        XCTAssertEqual(transport.posts[0].json["nonce"] as? String, transport.posts[1].json["nonce"] as? String)
        XCTAssertEqual(transport.posts[1].json["body"] as? String, "Photo caption")
        XCTAssertEqual(transport.posts[1].json["file_ids"] as? [String], transport.fileIDs.map { $0.uuidString.lowercased() })
        XCTAssertTrue(transport.uploads.allSatisfy { $0.path == "/api/v1/workspaces/\(workspace.uuidString.lowercased())/chat/files" })
        XCTAssertEqual(transport.posts[1].path, "/api/v1/workspaces/\(workspace.uuidString.lowercased())/chat/conversations/\(conversation.uuidString.lowercased())/messages")
    }

    @MainActor
    func testChatPartialUploadRetriesOnlyUnacknowledgedPhoto() async throws {
        let transport = FakeShareTransport()
        transport.failUploadAt = 2
        let delivery = ShareDelivery(transport: transport, kind: .chat, workspaceID: UUID(), destinationID: UUID(),
                                     caption: "", photos: photos(3))
        do { try await delivery.send(); XCTFail("Expected upload failure") } catch {}
        XCTAssertEqual(delivery.uploaded, 1)
        XCTAssertTrue(transport.posts.isEmpty)
        try await delivery.send()
        XCTAssertEqual(transport.uploads.map(\.name), ["Photo-1.jpg", "Photo-2.jpg", "Photo-2.jpg", "Photo-3.jpg"])
        XCTAssertEqual(transport.posts.count, 1)
        XCTAssertEqual((transport.posts[0].json["file_ids"] as? [String])?.count, 3)
    }

    @MainActor
    func testTaskUploadsDirectlyWithoutChatMessage() async throws {
        let transport = FakeShareTransport()
        let workspace = UUID(), task = UUID()
        let delivery = ShareDelivery(transport: transport, kind: .task, workspaceID: workspace, destinationID: task,
                                     caption: "", photos: photos(2))
        try await delivery.send()
        XCTAssertTrue(delivery.sent)
        XCTAssertTrue(transport.posts.isEmpty)
        XCTAssertEqual(transport.uploads.count, 2)
        XCTAssertTrue(transport.uploads.allSatisfy {
            $0.path == "/api/v1/workspaces/\(workspace.uuidString.lowercased())/tasks/\(task.uuidString.lowercased())/attachments"
                && $0.query.isEmpty
        })
    }

    @MainActor
    func testTaskFailureCannotBlindlyRetryOrClaimSuccess() async throws {
        let transport = FakeShareTransport()
        transport.failUploadAt = 2
        let delivery = ShareDelivery(transport: transport, kind: .task, workspaceID: UUID(), destinationID: UUID(),
                                     caption: "", photos: photos(3))
        do { try await delivery.send(); XCTFail("Expected failure") } catch {}
        XCTAssertEqual(delivery.uploaded, 1)
        XCTAssertFalse(delivery.sent)
        do { try await delivery.send(); XCTFail("Unsafe task retry should be blocked") } catch {}
        XCTAssertEqual(transport.uploads.count, 2)
        XCTAssertTrue(transport.posts.isEmpty)
    }

    @MainActor
    func testInvalidPhotoCountsNeverUpload() async throws {
        for count in [0, 11] {
            let transport = FakeShareTransport()
            let delivery = ShareDelivery(transport: transport, kind: .chat, workspaceID: UUID(), destinationID: UUID(),
                                         caption: "", photos: photos(count))
            do { try await delivery.send(); XCTFail("Expected count rejection") } catch {}
            XCTAssertTrue(transport.uploads.isEmpty)
            XCTAssertTrue(transport.posts.isEmpty)
        }
    }

    func testNativeHTTPRequestsHaveOriginAndScopedCookies() async throws {
        let account = ShareAccount(id: UUID(), url: origin, cookies: [cookie(), cookie(domain: "other.example.com")])
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [ShareTestURLProtocol.self]
        let client = ShareAPI(account: account, configuration: config)
        let records: [ShareRecord] = try await client.get("/api/v1/workspaces")
        XCTAssertEqual(records.first?.name, "Native fixture")
        let request = ShareTestURLProtocol.lastRequest!
        XCTAssertEqual(request.value(forHTTPHeaderField: "Cookie"), "session=test-only")
        XCTAssertEqual(request.value(forHTTPHeaderField: "Origin"), origin.absoluteString)
        XCTAssertEqual(request.value(forHTTPHeaderField: "Accept"), "application/json")
        XCTAssertNil(config.httpCookieStorage)
        XCTAssertNil(config.urlCache)
        do {
            let _: [ShareRecord] = try await client.get("https://other.example.com/api/v1/workspaces")
            XCTFail("Cross-origin requests must be rejected")
        } catch {}
        do {
            let _: [ShareRecord] = try await client.get("/api/v1/gateway")
            XCTFail("Gateway redirects must not verify")
        } catch ShareAPIError.signIn {} catch { XCTFail("Wrong gateway classification: \(error)") }
        do {
            let _: [ShareRecord] = try await client.get("/api/v1/html")
            XCTFail("HTML is not an API response")
        } catch {}
    }

    func testMultipartAndRedirectPolicyOverRealHTTP() async throws {
        let process = Process()
        let output = Pipe()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/env")
        process.arguments = ["python3", "Tests/http_fixture.py"]
        process.standardOutput = output
        try process.run()
        defer { if process.isRunning { process.terminate() }; process.waitUntilExit() }
        let port = String(decoding: output.fileHandleForReading.availableData, as: UTF8.self)
            .trimmingCharacters(in: .whitespacesAndNewlines)
        let server = try XCTUnwrap(URL(string: "http://127.0.0.1:\(port)"))
        let account = ShareAccount(id: UUID(), url: server, cookies: [cookie(domain: "127.0.0.1", secure: false)])
        let client = ShareAPI(account: account)
        let file = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString + ".jpg")
        defer { try? FileManager.default.removeItem(at: file) }
        try Data("fixture-photo-bytes".utf8).write(to: file)
        _ = try await client.upload(file, name: "Photo\"\\\r\ninjected.jpg", path: "/api/v1/chat")
        _ = try await client.upload(file, name: "Photo.jpg", path: "/api/v1/task")
        try await client.post("/api/v1/messages", json: ["body": "Caption", "nonce": UUID().uuidString, "file_ids": []])
        struct State: Decodable { let redirects: Int }
        do {
            let _: State = try await client.get("/api/v1/gateway")
            XCTFail("A gated GET must not follow the redirect")
        } catch ShareAPIError.signIn {} catch { XCTFail("Wrong gateway error: \(error)") }
        do {
            _ = try await client.upload(file, name: "Photo.jpg", path: "/api/v1/gateway")
            XCTFail("A photo upload must not follow the redirect")
        } catch ShareAPIError.signIn {} catch { XCTFail("Wrong gateway error: \(error)") }
        let state: State = try await client.get("/api/v1/state")
        XCTAssertEqual(state.redirects, 0)
    }

    private func photos(_ count: Int) -> [SharePhotoUpload] {
        (0..<count).map { SharePhotoUpload(file: URL(fileURLWithPath: "/tmp/fixture-\($0).jpg"), name: "Photo-\($0 + 1).jpg", width: 100, height: 80) }
    }
}

private final class FakeShareTransport: ShareTransport {
    struct Upload { let name: String; let path: String; let query: [URLQueryItem] }
    struct Post { let path: String; let json: [String: Any] }
    var uploads: [Upload] = []
    var posts: [Post] = []
    var fileIDs: [UUID] = []
    var failUploadAt: Int?
    var failPosts = 0
    func upload(_ file: URL, name: String, path: String, query: [URLQueryItem]) async throws -> UUID {
        uploads.append(Upload(name: name, path: path, query: query))
        if uploads.count == failUploadAt { failUploadAt = nil; throw URLError(.networkConnectionLost) }
        let id = UUID(); fileIDs.append(id); return id
    }
    func post(_ path: String, json: [String: Any]) async throws {
        posts.append(Post(path: path, json: json))
        if failPosts > 0 { failPosts -= 1; throw URLError(.networkConnectionLost) }
    }
}

private final class ShareTestURLProtocol: URLProtocol {
    static var lastRequest: URLRequest?
    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() {
        Self.lastRequest = request
        let path = request.url!.path
        let status = path.hasSuffix("gateway") ? 307 : 200
        let type = path.hasSuffix("html") ? "text/html" : "application/json"
        let response = HTTPURLResponse(url: request.url!, statusCode: status, httpVersion: "HTTP/1.1",
                                       headerFields: ["Content-Type": type, "Location": "https://exe.dev/auth"])!
        client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
        let body = "[{\"id\":\"\(UUID())\",\"name\":\"Native fixture\"}]"
        client?.urlProtocol(self, didLoad: Data(body.utf8))
        client?.urlProtocolDidFinishLoading(self)
    }
    override func stopLoading() {}
}

let shareTests = testCase([
    ("testCookieBoundaries", ShareTests.testCookieBoundaries),
    ("testAccountNeverSendsCookiesToAnotherOrigin", ShareTests.testAccountNeverSendsCookiesToAnotherOrigin),
    ("testWireShapesAndWritableDestinations", ShareTests.testWireShapesAndWritableDestinations),
    ("testChatUploadsThenPostsAndRetryReusesNonce", asyncTest(ShareTests.testChatUploadsThenPostsAndRetryReusesNonce)),
    ("testChatPartialUploadRetriesOnlyUnacknowledgedPhoto", asyncTest(ShareTests.testChatPartialUploadRetriesOnlyUnacknowledgedPhoto)),
    ("testTaskUploadsDirectlyWithoutChatMessage", asyncTest(ShareTests.testTaskUploadsDirectlyWithoutChatMessage)),
    ("testTaskFailureCannotBlindlyRetryOrClaimSuccess", asyncTest(ShareTests.testTaskFailureCannotBlindlyRetryOrClaimSuccess)),
    ("testInvalidPhotoCountsNeverUpload", asyncTest(ShareTests.testInvalidPhotoCountsNeverUpload)),
    ("testNativeHTTPRequestsHaveOriginAndScopedCookies", asyncTest(ShareTests.testNativeHTTPRequestsHaveOriginAndScopedCookies)),
    ("testMultipartAndRedirectPolicyOverRealHTTP", asyncTest(ShareTests.testMultipartAndRedirectPolicyOverRealHTTP)),
])
