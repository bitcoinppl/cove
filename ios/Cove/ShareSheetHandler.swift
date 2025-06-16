import UIKit
import SwiftUI

struct ShareSheetHandler {
    
    /// shows share sheet for the given file URL
    static func presentShareSheet(for url: URL) {
        guard let windowScene = UIApplication.shared.connectedScenes
            .compactMap({ $0 as? UIWindowScene })
            .first,
              let rootViewController = windowScene.windows
            .first(where: { $0.isKeyWindow })?.rootViewController else {
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
    
    /// checks if the file should be handled with share sheet instead of direct opening
    static func shouldUseShareSheet(for url: URL) -> Bool {
        // check if file is from iCloud Downloads or similar external sources
        let urlString = url.absoluteString.lowercased()
        return urlString.contains("downloads") || 
               urlString.contains("icloud") ||
               urlString.contains("mobile/documents")
    }
}