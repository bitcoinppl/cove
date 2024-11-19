//
//  Routes+Ext.swift
//  Cove
//
//  Created by Praveen Perera on 6/17/24.
//

import Foundation

extension Route: Equatable, Hashable {
    public static func == (lhs: Route, rhs: Route) -> Bool {
        isRouteEqual(route: lhs, routeToCheck: rhs)
    }

    public func hash(into hasher: inout Hasher) {
        let hashed = hashRoute(route: self)
        hasher.combine(hashed)
    }
}

extension HotWalletRoute {
    func intoRoute() -> Route {
        RouteFactory().hotWallet(route: self)
    }
}
