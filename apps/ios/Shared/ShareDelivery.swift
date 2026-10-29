import Foundation

public protocol ShareTransport {
    func upload(_ file: URL, name: String, path: String, query: [URLQueryItem]) async throws -> UUID
    func post(_ path: String, json: [String: Any]) async throws
}

public struct SharePhotoUpload {
    public let file: URL
    public let name: String
    public let width: Int
    public let height: Int
    public init(file: URL, name: String, width: Int, height: Int) {
        self.file = file; self.name = name; self.width = width; self.height = height
    }
}

/// One explicit user's send. Its immutable destination, acknowledged uploads and
/// nonce survive a chat retry. Task writes lack server idempotency, so a failed
/// attempt cannot be retried blindly after a potentially lost acknowledgement.
@MainActor
public final class ShareDelivery {
    public private(set) var uploaded = 0
    public private(set) var sent = false
    private let transport: any ShareTransport
    private let kind: ShareKind
    private let base: String
    private let target: String
    private let caption: String
    private let photos: [SharePhotoUpload]
    private let nonce = UUID().uuidString
    private var fileIDs: [UUID] = []
    private var attempted = false

    public init(transport: any ShareTransport, kind: ShareKind, workspaceID: UUID,
                destinationID: UUID, caption: String, photos: [SharePhotoUpload]) {
        self.transport = transport; self.kind = kind; self.caption = caption; self.photos = photos
        base = "/api/v1/workspaces/" + workspaceID.uuidString.lowercased()
        target = destinationID.uuidString.lowercased()
    }

    public func send(progress: (Int) -> Void = { _ in }) async throws {
        if sent { return }
        guard (1...10).contains(photos.count), caption.count <= 4000 else { throw ShareAPIError.invalidDestination }
        if attempted, kind == .task { throw ShareAPIError.unsafeRetry }
        attempted = true
        for photo in photos.dropFirst(uploaded) {
            try Task.checkCancellation()
            let path = kind == .chat ? base + "/chat/files" : base + "/tasks/\(target)/attachments"
            let size = kind == .chat ? [URLQueryItem(name: "width", value: String(photo.width)),
                                        URLQueryItem(name: "height", value: String(photo.height))] : []
            let id = try await transport.upload(photo.file, name: photo.name, path: path, query: size)
            fileIDs.append(id)
            uploaded += 1
            progress(uploaded)
        }
        if kind == .chat {
            try await transport.post(base + "/chat/conversations/\(target)/messages", json: [
                "body": caption, "file_ids": fileIDs.map { $0.uuidString.lowercased() }, "nonce": nonce,
            ])
        }
        sent = true
    }
}
