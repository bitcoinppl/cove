//
//  CoveAppDelegate.swift
//  Cove
//
//  Created by Praveen Perera on 11/18/25.
//

import UIKit

final class CoveAppDelegate: NSObject, UIApplicationDelegate {
    func application(
        _: UIApplication,
        configurationForConnecting connectingSceneSession: UISceneSession,
        options _: UIScene.ConnectionOptions
    ) -> UISceneConfiguration {
        let configuration = UISceneConfiguration(
            name: nil,
            sessionRole: connectingSceneSession.role
        )
        configuration.delegateClass = CovePopupSceneDelegate.self
        return configuration
    }
}
