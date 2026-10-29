import SwiftUI

struct ServerPickerView: View {
    @EnvironmentObject private var store: ServerStore
    @State private var address = ""
    @State private var connection: Server?
    @State private var error: String?

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    TextField("orbit.example.com", text: $address)
                        .keyboardType(.URL)
                        .textContentType(.URL)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .submitLabel(.go)
                        .onSubmit(connect)
                    Button(action: connect) {
                        if connection != nil {
                            ProgressView()
                        } else {
                            Text("Connect")
                        }
                    }
                    .disabled(connection != nil || address.trimmingCharacters(in: .whitespaces).isEmpty)
                } header: {
                    Text("Server address")
                } footer: {
                    if let error {
                        Text(error).foregroundStyle(.red)
                    }
                }

                if !store.servers.isEmpty {
                    Section("Recent") {
                        ForEach(store.servers) { server in
                            Button(server.name) { store.open(server.url) }
                        }
                        .onDelete { offsets in
                            for server in offsets.map({ store.servers[$0] }) {
                                store.remove(server)
                            }
                        }
                    }
                }
            }
            .navigationTitle("Connect to Orbit")
        }
        .alert("Couldn't forget server", isPresented: Binding(
            get: { store.removalError != nil }, set: { if !$0 { store.removalError = nil } }
        )) {
            Button("OK", role: .cancel) { store.removalError = nil }
        } message: {
            Text(store.removalError ?? "")
        }
        .sheet(item: $connection) { server in
            ServerConnectionView(server: server, isNew: !store.servers.contains { $0.id == server.id }) { verified in
                connection = nil
                store.open(verified)
            }
        }
    }

    private func connect() {
        guard connection == nil else { return }
        guard let origin = ServerAddress.origin(from: address) else {
            error = "Enter an https address, like orbit.example.com."
            return
        }
        error = nil
        connection = store.candidate(for: origin)
    }
}
