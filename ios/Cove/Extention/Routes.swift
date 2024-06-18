//
//  Router.swift
//  Cove
//
//  Created by Praveen Perera on 6/17/24.
//

import Foundation

extension HotWalletRoute {
    func intoRoute() -> Route {
        RouteFactory().hotWallet(route: self)
    }
}
