import Foundation
import OrbitSharing
import SwiftUI
import UniformTypeIdentifiers

@MainActor
final class PhotoShareModel: ObservableObject {
    @Published private(set) var accounts: [ShareAccount] = []
    @Published var accountID: UUID?
    @Published private(set) var workspaces: [ShareRecord] = []
    @Published var workspaceID: UUID?
    @Published var kind: ShareKind = .chat
    @Published var search = ""
    @Published var caption = ""
    @Published var destinationID: UUID?
    @Published private(set) var destinations: [ShareDestination] = []
    @Published private(set) var photos: [SharedPhoto] = []
    @Published private(set) var loadingPhotos = true
    @Published private(set) var loadingWorkspaces = false
    @Published private(set) var loadingDestinations = false
    @Published private(set) var sending = false
    @Published private(set) var started = false
    @Published private(set) var taskFailed = false
    @Published private(set) var uploaded = 0
    @Published private(set) var error: String?
    @Published private(set) var photoError: String?
    @Published private(set) var nextCursor: String?

    private let providers: [NSItemProvider]
    private let folder = FileManager.default.temporaryDirectory.appendingPathComponent("OrbitShare-" + UUID().uuidString)
    private var api: ShareAPI?
    private var userID: UUID?
    private var accountGeneration = 0
    private var destinationGeneration = 0
    private var loadedKey: String?
    private var stopped = false
    private var delivery: ShareDelivery?

    init(providers: [NSItemProvider]) {
        self.providers = providers.filter { $0.hasItemConformingToTypeIdentifier(UTType.image.identifier) }
        do {
            accounts = try ShareAccountStore.accounts()
            accountID = accounts.first?.id
        } catch { self.error = error.localizedDescription }
    }

    deinit { try? FileManager.default.removeItem(at: folder) }

    var destinationKey: String {
        "\(accountID?.uuidString ?? "")/\(workspaceID?.uuidString ?? "")/\(kind.rawValue)/\(kind == .task ? search : "")"
    }
    var visibleDestinations: [ShareDestination] {
        if kind == .task || search.isEmpty { return destinations }
        return destinations.filter { $0.title.localizedCaseInsensitiveContains(search) }
    }
    var canSend: Bool {
        !stopped && !sending && !taskFailed && !loadingPhotos && photoError == nil && !photos.isEmpty
            && !loadingWorkspaces && !loadingDestinations && loadedKey == destinationKey
            && destinationID.map { id in destinations.contains { $0.id == id } } == true
            && (kind != .chat || caption.count <= 4000)
    }

    func cancel() {
        stopped = true
        api?.cancel()
        try? FileManager.default.removeItem(at: folder)
    }

    func loadPhotos() async {
        do {
            guard (1...10).contains(providers.count) else { throw SharedPhoto.PhotoError.count }
            try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true,
                                                    attributes: [.protectionKey: FileProtectionType.complete])
            var loaded: [SharedPhoto] = []
            for (index, provider) in providers.enumerated() {
                try Task.checkCancellation()
                guard !stopped else { return }
                loaded.append(try await SharedPhoto.load(provider, folder: folder, index: index))
            }
            guard !stopped, !Task.isCancelled else { return }
            photos = loaded
        } catch {
            guard !stopped, !Task.isCancelled else { return }
            photoError = error.localizedDescription
        }
        loadingPhotos = false
    }

    func loadWorkspaces() async {
        guard !started else { return }
        accountGeneration += 1
        let token = accountGeneration
        api?.cancel()
        api = nil
        workspaces = []; workspaceID = nil; destinations = []; destinationID = nil
        userID = nil; loadedKey = nil
        guard let account = accounts.first(where: { $0.id == accountID }), !stopped else { return }
        loadingWorkspaces = true
        error = nil
        let client = ShareAPI(account: account)
        api = client
        do {
            let user: ShareUser = try await client.get("/api/v1/auth/me")
            let records: [ShareRecord] = try await client.get("/api/v1/workspaces")
            guard !stopped, !Task.isCancelled, accountGeneration == token else { return }
            userID = user.id
            workspaces = records
            workspaceID = records.first?.id
            loadingWorkspaces = false
        } catch {
            guard !stopped, !Task.isCancelled, accountGeneration == token else { return }
            self.error = error.localizedDescription
            loadingWorkspaces = false
        }
    }

    func loadDestinations(append: Bool = false) async {
        guard !started, let workspaceID, let api, let userID, !stopped else { return }
        destinationGeneration += 1
        let token = destinationGeneration
        let key = destinationKey
        let mode = kind
        var query = String(search.trimmingCharacters(in: .whitespacesAndNewlines).prefix(200))
        while query.utf8.count > 200 { query.removeLast() }
        let cursor = append ? nextCursor : nil
        if !append { destinations = []; destinationID = nil; nextCursor = nil; loadedKey = nil }
        loadingDestinations = true
        error = nil
        do {
            // SwiftUI cancels the old task when the server, workspace, kind or
            // search changes. A short debounce keeps task search inexpensive.
            if mode == .task, !query.isEmpty { try await Task.sleep(for: .milliseconds(250)) }
            let base = "/api/v1/workspaces/\(workspaceID.uuidString.lowercased())"
            var records: [ShareDestination]
            var following: String?
            if mode == .chat {
                let members: [ShareMember] = try await allPages(api, path: base + "/members")
                let conversations: [ShareConversation] = try await api.get(base + "/chat/conversations")
                records = conversations.filter { $0.canSend(as: userID) }.map {
                    ShareDestination(id: $0.id, title: $0.title(for: userID, members: members))
                }.sorted { $0.title.localizedCaseInsensitiveCompare($1.title) == .orderedAscending }
            } else {
                let projects: [ShareProject] = try await allPages(api, path: base + "/projects")
                var parameters = [URLQueryItem(name: "limit", value: "30"),
                                  URLQueryItem(name: "sort", value: "updated_at"), URLQueryItem(name: "order", value: "desc")]
                if !query.isEmpty { parameters.append(URLQueryItem(name: "search", value: query)) }
                if let cursor { parameters.append(URLQueryItem(name: "cursor", value: cursor)) }
                let page: SharePage<ShareTask> = try await api.get(base + "/tasks", query: parameters)
                records = page.items.map { task in
                    let project = projects.first { $0.id == task.projectId }?.key ?? "Task"
                    return ShareDestination(id: task.id, title: "\(project)-\(task.number) · \(task.title)")
                }
                following = page.nextCursor
            }
            guard !stopped, !Task.isCancelled, token == destinationGeneration, key == destinationKey else { return }
            if append {
                let existing = Set(destinations.map(\.id))
                records.removeAll { existing.contains($0.id) }
                destinations += records
            } else { destinations = records }
            nextCursor = following
            loadedKey = key
            loadingDestinations = false
        } catch {
            guard !stopped, !Task.isCancelled, token == destinationGeneration, key == destinationKey else { return }
            self.error = error.localizedDescription
            loadingDestinations = false
        }
    }

    func send() async -> Bool {
        guard canSend, let api, let workspaceID, let destinationID else { return false }
        started = true // Freeze server, destination and caption across retries.
        sending = true
        error = nil
        if delivery == nil {
            delivery = ShareDelivery(transport: api, kind: kind, workspaceID: workspaceID,
                                     destinationID: destinationID, caption: caption, photos: photos.map {
                SharePhotoUpload(file: $0.file, name: $0.name, width: $0.width, height: $0.height)
            })
        }
        do {
            try await delivery!.send { [weak self] in self?.uploaded = $0 }
            guard !stopped else { return false }
            sending = false
            return true
        } catch {
            guard !stopped else { return false }
            sending = false
            if kind == .task {
                // Task attachment writes have no idempotency key. Never blindly
                // retry a request whose acknowledgement may have been lost.
                taskFailed = true
                self.error = "\(uploaded) of \(photos.count) photos confirmed attached. \(error.localizedDescription) Check the task in Orbit before sharing again; the last photo may already be attached."
            } else {
                // Uploaded file IDs and the message nonce survive Retry, so it
                // cannot create a second message after a lost acknowledgement.
                self.error = error.localizedDescription
            }
            return false
        }
    }

    private func allPages<T: Decodable>(_ api: ShareAPI, path: String) async throws -> [T] {
        var items: [T] = []
        var cursor: String?
        var seen = Set<String>()
        repeat {
            try Task.checkCancellation()
            var query = [URLQueryItem(name: "limit", value: "100")]
            if let cursor { query.append(URLQueryItem(name: "cursor", value: cursor)) }
            let page: SharePage<T> = try await api.get(path, query: query)
            items += page.items
            cursor = page.nextCursor
            if let cursor, !seen.insert(cursor).inserted { throw ShareAPIError.invalidResponse }
        } while cursor != nil
        return items
    }
}
