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
            try data.write(to: fileURL, atomically: true, encoding: .utf8)
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
