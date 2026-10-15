import SwiftUI

struct ServerPickerView: View {
    @EnvironmentObject private var store: ServerStore
    @State private var address = ""
    @State private var checking = false
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
                        if checking {
                            ProgressView()
                        } else {
                            Text("Connect")
                        }
                    }
                    .disabled(checking || address.trimmingCharacters(in: .whitespaces).isEmpty)
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
    }

    private func connect() {
        guard !checking else { return }
        guard let origin = ServerAddress.origin(from: address) else {
            error = "Enter an https address, like orbit.example.com."
            return
        }
        checking = true
        error = nil
        Task {
            defer { checking = false }
            do {
                let reached = try await ServerAddress.verify(origin)
                store.open(reached)
            } catch let failure {
                error = failure.localizedDescription
            }
        }
    }
}
