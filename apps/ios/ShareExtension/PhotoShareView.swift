import OrbitSharing
import SwiftUI

struct PhotoShareView: View {
    @ObservedObject var model: PhotoShareModel
    let cancel: () -> Void
    let complete: () -> Void

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    if model.loadingPhotos {
                        ProgressView("Preparing photos…")
                    } else {
                        ScrollView(.horizontal) {
                            HStack {
                                ForEach(model.photos) { photo in
                                    Image(uiImage: photo.preview)
                                        .resizable().scaledToFill().frame(width: 72, height: 72)
                                        .clipShape(RoundedRectangle(cornerRadius: 8))
                                        .accessibilityLabel(photo.name)
                                }
                            }
                        }
                        Text("\(model.photos.count) photo\(model.photos.count == 1 ? "" : "s")")
                            .font(.caption).foregroundStyle(.secondary)
                    }
                    if let error = model.photoError { Text(error).foregroundStyle(.red) }
                }

                if model.accounts.isEmpty {
                    Section {
                        Text("Open Orbit and sign in to a server first, then return to Photos and share again.")
                    }
                } else {
                    Section {
                        Picker("Server", selection: $model.accountID) {
                            ForEach(model.accounts) { account in
                                Text(account.name).tag(Optional(account.id))
                            }
                        }
                        if model.loadingWorkspaces {
                            ProgressView("Loading workspaces…")
                        } else if model.workspaces.isEmpty, model.error == nil {
                            Text("No workspaces are available for this account.").foregroundStyle(.secondary)
                        } else if !model.workspaces.isEmpty {
                            Picker("Workspace", selection: $model.workspaceID) {
                                ForEach(model.workspaces) { workspace in
                                    Text(workspace.name).tag(Optional(workspace.id))
                                }
                            }
                        }
                        Picker("Destination", selection: $model.kind) {
                            ForEach(ShareKind.allCases) { kind in Text(kind.title).tag(kind) }
                        }
                        .pickerStyle(.segmented)
                    }
                    .disabled(model.started)

                    if model.workspaceID != nil {
                        Section(model.kind == .chat ? "Send to" : "Attach to") {
                            TextField(model.kind == .chat ? "Find a chat or DM" : "Find a task", text: $model.search)
                                .textInputAutocapitalization(.never).autocorrectionDisabled()
                                .disabled(model.started)
                            ForEach(model.visibleDestinations) { destination in
                                Button {
                                    model.destinationID = destination.id
                                } label: {
                                    HStack {
                                        Label(destination.title, systemImage: model.kind.symbol)
                                            .foregroundStyle(.primary)
                                            .multilineTextAlignment(.leading)
                                        Spacer()
                                        if model.destinationID == destination.id {
                                            Image(systemName: "checkmark").foregroundStyle(.tint)
                                        }
                                    }
                                }
                                .disabled(model.started)
                                .accessibilityAddTraits(model.destinationID == destination.id ? .isSelected : [])
                            }
                            if model.loadingDestinations {
                                ProgressView("Loading destinations…")
                            } else if model.visibleDestinations.isEmpty, model.error == nil {
                                Text(model.kind == .chat ? "No matching chats you can send to." : "No matching tasks.")
                                    .foregroundStyle(.secondary)
                            }
                            if model.nextCursor != nil {
                                Button("Load more tasks") { Task { await model.loadDestinations(append: true) } }
                                    .disabled(model.started || model.loadingDestinations)
                            }
                        }
                        if model.kind == .chat {
                            Section("Message (optional)") {
                                TextField("Add a message…", text: $model.caption, axis: .vertical)
                                    .lineLimit(2...5).disabled(model.started)
                                if model.caption.count > 4000 {
                                    Text("Messages can have up to 4,000 characters.").foregroundStyle(.red)
                                }
                            }
                        }
                    }
                }

                if model.sending {
                    Section {
                        ProgressView(value: Double(model.uploaded), total: Double(max(model.photos.count, 1)))
                        Text("\(model.uploaded) of \(model.photos.count) photos uploaded. Sending…")
                            .font(.caption).foregroundStyle(.secondary)
                    }
                }
                if let error = model.error {
                    Section {
                        Text(error).foregroundStyle(.red)
                        if !model.started, !model.accounts.isEmpty {
                            Button("Retry connection") {
                                Task { await model.loadWorkspaces() }
                            }
                            .disabled(model.loadingWorkspaces)
                        }
                    }
                }
            }
            .navigationTitle("Share to Orbit")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel", action: cancel).disabled(model.sending)
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button(model.started ? "Retry" : model.kind == .chat ? "Send" : "Attach") {
                        Task { if await model.send() { complete() } }
                    }
                    .disabled(!model.canSend)
                }
            }
        }
        .tint(Color(red: 242 / 255, green: 69 / 255, blue: 143 / 255))
        .task { await model.loadPhotos() }
        .task(id: model.accountID) { await model.loadWorkspaces() }
        .task(id: model.destinationKey) { await model.loadDestinations() }
    }
}
