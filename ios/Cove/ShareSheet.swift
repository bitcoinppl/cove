import SwiftUI
import UIKit
import UniformTypeIdentifiers

enum ShareSheet {
    /// shows share sheet for the given file URL
    @MainActor
    static func present(for url: URL) {
        guard let windowScene = UIApplication.shared.connectedScenes
            .compactMap({ $0 as? UIWindowScene })
            .first,
            let rootViewController = windowScene.windows
            .first(where: { $0.isKeyWindow })?.rootViewController
        else {
            return
        }

        let activityViewController = UIActivityViewController(
            activityItems: [url],
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

        rootViewController.present(activityViewController, animated: true)
    }

    /// presents share sheet with arbitrary data by writing to a temporary file
    /// - Parameters:
    ///   - data: the data to share
    ///   - filename: the filename to use for the temporary file
    ///   - utType: the uniform type identifier for the file (defaults to .plainText)
    ///   - completion: called after the share sheet dismisses with success/failure result
    @MainActor
    static func present(
        data: String,
        filename: String,
        utType _: UTType = .plainText,
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

        let activityViewController = UIActivityViewController(
            activityItems: [fileURL],
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
            // attempt to clean up temp file
            try? FileManager.default.removeItem(at: fileURL)

            if let error {
                Log.error("Share sheet error: \(error.localizedDescription)")
                completion(false)
            } else {
                completion(completed)
            }
        }

        rootViewController.present(activityViewController, animated: true)
    }
}
