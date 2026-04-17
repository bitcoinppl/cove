import LinkPresentation
import SwiftUI
import UIKit

@MainActor
private class ShareableFile: NSObject, UIActivityItemSource {
    let url: URL
    let iconImage: UIImage?

    init(url: URL, iconImage: UIImage? = nil) {
        self.url = url
        self.iconImage = iconImage
        super.init()
    }

    func activityViewControllerPlaceholderItem(_: UIActivityViewController) -> Any {
        url
    }

    func activityViewController(_: UIActivityViewController, itemForActivityType _: UIActivity.ActivityType?) -> Any? {
        url
    }

    func activityViewControllerLinkMetadata(_: UIActivityViewController) -> LPLinkMetadata? {
        let metadata = LPLinkMetadata()
        metadata.title = url.lastPathComponent

        if let iconImage {
            metadata.iconProvider = NSItemProvider(object: iconImage)
        }

        return metadata
    }
}

enum ShareSheet {
    /// Shows share sheet for the given file URL
    @MainActor
    static func present(for url: URL) {
        present(for: url) { _ in }
    }

    /// Shows share sheet for the given file URL with a completion handler
    @MainActor
    static func present(for url: URL, completion: @escaping (Bool) -> Void) {
        guard let windowScene = UIApplication.shared.connectedScenes
            .compactMap({ $0 as? UIWindowScene })
            .first,
            let rootViewController = windowScene.windows
            .first(where: { $0.isKeyWindow })?.rootViewController
        else {
            completion(false)
            return
        }

        // load Cove icon
        let iconImage = UIImage(named: "icon")

        let activityViewController = UIActivityViewController(
            activityItems: [ShareableFile(url: url, iconImage: iconImage)],
            applicationActivities: nil
        )

        // configure for iPad
        if let popover = activityViewController.popoverPresentationController {
            popover.sourceView = rootViewController.view
            popover.sourceRect = CGRect(
                x: rootViewController.view.bounds.midX,
                y: rootViewController.view.bounds.midY,
                width: 0,
                height: 0
            )
            popover.permittedArrowDirections = []
        }

        activityViewController.completionWithItemsHandler = { _, completed, _, error in
            do {
                try FileManager.default.removeItem(at: url)
            } catch let removeError as NSError where removeError.domain == NSCocoaErrorDomain && removeError.code == NSFileNoSuchFileError {
                // already cleaned up
            } catch {
                Log.error("Failed to remove temporary backup file: \(error)")
            }

            if let error {
                Log.error("Share sheet error: \(error.localizedDescription)")
                completion(false)
            } else {
                completion(completed)
            }
        }

        var presenter = rootViewController
        while let presented = presenter.presentedViewController {
            presenter = presented
        }
        presenter.present(activityViewController, animated: true)
    }

    /// Like `present(data:filename:completion:)` but defers by 400ms so that a
    /// transient presenter (Menu, confirmationDialog) can finish its dismissal
    /// animation before the share sheet appears. Centralises the magic delay and
    /// failure-logging so callers don't repeat them.
    @MainActor
    static func presentFromMenu(data: String, filename: String) {
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.4) {
            present(data: data, filename: filename) { success in
                if !success { Log.warn("Share sheet cancelled or failed: \(filename)") }
            }
        }
    }

    /// Binary-data variant of `presentFromMenu`.
    @MainActor
    static func presentFromMenu(data: Data, filename: String) {
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.4) {
            present(data: data, filename: filename) { success in
                if !success { Log.warn("Share sheet cancelled or failed: \(filename)") }
            }
        }
    }

    /// Presents share sheet with arbitrary data by writing to a temporary file
    /// - Parameters:
    ///   - data: the data to share
    ///   - filename: the filename to use for the temporary file
    ///   - completion: called after the share sheet dismisses with success/failure result
    @MainActor
    static func present(
        data: String,
        filename: String,
        completion: @escaping (Bool) -> Void
    ) {
        guard let bytes = data.data(using: .utf8) else {
            Log.error("Failed to encode share-sheet payload as UTF-8")
            completion(false)
            return
        }
        present(data: bytes, filename: filename, completion: completion)
    }

    /// Presents share sheet for binary data by writing to a temporary file
    /// - Parameters:
    ///   - data: the raw bytes to share
    ///   - filename: the filename to use for the temporary file
    ///   - completion: called after the share sheet dismisses with success/failure result
    @MainActor
    static func present(
        data: Data,
        filename: String,
        completion: @escaping (Bool) -> Void
    ) {
        guard let windowScene = UIApplication.shared.connectedScenes
            .compactMap({ $0 as? UIWindowScene })
            .first,
            let rootViewController = windowScene.windows
            .first(where: { $0.isKeyWindow })?.rootViewController
        else {
            completion(false)
            return
        }

        // create temp file
        let tempDir = FileManager.default.temporaryDirectory
        let fileURL = tempDir.appendingPathComponent(filename)

        do {
            try data.write(to: fileURL, options: .atomic)
        } catch {
            Log.error("Failed to write temp file for share sheet: \(error.localizedDescription)")
            completion(false)
            return
        }

        // load Cove icon
        let iconImage = UIImage(named: "icon")

        let activityViewController = UIActivityViewController(
            activityItems: [ShareableFile(url: fileURL, iconImage: iconImage)],
            applicationActivities: nil
        )

        // configure for iPad
        if let popover = activityViewController.popoverPresentationController {
            popover.sourceView = rootViewController.view
            popover.sourceRect = CGRect(
                x: rootViewController.view.bounds.midX,
                y: rootViewController.view.bounds.midY,
                width: 0,
                height: 0
            )
            popover.permittedArrowDirections = []
        }

        // set completion handler
        activityViewController.completionWithItemsHandler = { _, completed, _, error in
            do {
                try FileManager.default.removeItem(at: fileURL)
            } catch let removeError as NSError where removeError.domain == NSCocoaErrorDomain && removeError.code == NSFileNoSuchFileError {
                // already cleaned up
            } catch {
                Log.error("Failed to remove temporary file: \(error)")
            }

            if let error {
                Log.error("Share sheet error: \(error.localizedDescription)")
                completion(false)
            } else {
                completion(completed)
            }
        }

        var presenter = rootViewController
        while let presented = presenter.presentedViewController {
            presenter = presented
        }
        presenter.present(activityViewController, animated: true)
    }
}
