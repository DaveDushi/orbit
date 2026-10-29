import ImageIO
import OrbitSharing
import SwiftUI
import UIKit
import UniformTypeIdentifiers

/// A real Photos share target. It sends in the extension; it does not use the
/// unsupported responder-chain trick to launch the containing application.
@MainActor
public final class ShareViewController: UIViewController {
    private var model: PhotoShareModel?

    public override func viewDidLoad() {
        super.viewDidLoad()
        preferredContentSize = CGSize(width: view.bounds.width, height: min(640, view.bounds.height))
        let providers = (extensionContext?.inputItems as? [NSExtensionItem] ?? []).flatMap { $0.attachments ?? [] }
        let model = PhotoShareModel(providers: providers)
        self.model = model
        let host = UIHostingController(rootView: PhotoShareView(model: model, cancel: { [weak self] in
            self?.model?.cancel()
            self?.extensionContext?.cancelRequest(withError: NSError(domain: NSCocoaErrorDomain, code: NSUserCancelledError))
        }, complete: { [weak self] in
            self?.extensionContext?.completeRequest(returningItems: nil)
        }))
        addChild(host)
        host.view.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(host.view)
        NSLayoutConstraint.activate([
            host.view.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            host.view.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            host.view.topAnchor.constraint(equalTo: view.topAnchor),
            host.view.bottomAnchor.constraint(equalTo: view.bottomAnchor),
        ])
        host.didMove(toParent: self)
    }

    public override func viewDidDisappear(_ animated: Bool) {
        super.viewDidDisappear(animated)
        model?.cancel()
    }
}

struct SharedPhoto: Identifiable {
    let id = UUID()
    let file: URL
    let name: String
    let preview: UIImage
    let width: Int
    let height: Int

    static func load(_ provider: NSItemProvider, folder: URL, index: Int) async throws -> SharedPhoto {
        guard let type = provider.registeredTypeIdentifiers.first(where: { UTType($0)?.conforms(to: .image) == true })
        else { throw PhotoError.unreadable }
        return try await withCheckedThrowingContinuation { continuation in
            provider.loadFileRepresentation(forTypeIdentifier: type) { source, error in
                do {
                    if let error { throw error }
                    guard let source else { throw PhotoError.unreadable }
                    // The provider's URL expires when this callback returns. Decode here,
                    // using ImageIO's bounded thumbnail instead of a full-resolution UIImage.
                    let photo = try autoreleasepool { () throws -> SharedPhoto in
                        guard let imageSource = CGImageSourceCreateWithURL(source as CFURL, nil),
                              let image = CGImageSourceCreateThumbnailAtIndex(imageSource, 0, [
                                kCGImageSourceCreateThumbnailFromImageAlways: true,
                                kCGImageSourceCreateThumbnailWithTransform: true,
                                kCGImageSourceThumbnailMaxPixelSize: 2048,
                                kCGImageSourceShouldCacheImmediately: true,
                              ] as CFDictionary),
                              let jpeg = UIImage(cgImage: image).jpegData(compressionQuality: 0.85),
                              let thumbnail = CGImageSourceCreateThumbnailAtIndex(imageSource, 0, [
                                kCGImageSourceCreateThumbnailFromImageAlways: true,
                                kCGImageSourceCreateThumbnailWithTransform: true,
                                kCGImageSourceThumbnailMaxPixelSize: 160,
                              ] as CFDictionary)
                        else { throw PhotoError.unreadable }
                        let name = "Photo-\(index + 1).jpg"
                        let file = folder.appendingPathComponent(UUID().uuidString + ".jpg")
                        try jpeg.write(to: file, options: [.atomic, .completeFileProtection])
                        return SharedPhoto(file: file, name: name, preview: UIImage(cgImage: thumbnail),
                                           width: image.width, height: image.height)
                    }
                    continuation.resume(returning: photo)
                } catch { continuation.resume(throwing: error) }
            }
        }
    }

    enum PhotoError: LocalizedError {
        case unreadable, count
        var errorDescription: String? {
            switch self {
            case .unreadable: "A selected photo couldn't be read. Download it from iCloud and try sharing again."
            case .count: "Share between 1 and 10 photos at a time."
            }
        }
    }
}
