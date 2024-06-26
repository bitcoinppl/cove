//
//  CoveApp.swift
//  Cove
//
//  Created by Praveen Perera  on 6/17/24.
//

import SwiftUI

struct NavigateKey: EnvironmentKey {
    static let defaultValue: (Route) -> Void = { _ in }
}

extension EnvironmentValues {
    var navigate: (Route) -> Void {
        get { self[NavigateKey.self] }
        set { self[NavigateKey.self] = newValue }
    }
}

@main
struct CoveApp: App {
    @State var model: MainViewModel

    public init() {
        model = MainViewModel()
    }

    var tintColor: Color {
        switch model.router.routes.last {
        case .newWallet(.hotWallet(.select)):
            Color.blue
        case .newWallet:
            Color.white
        default:
            Color.blue
        }
    }

    var body: some Scene {
        WindowGroup {
            NavigationStack(path: $model.router.routes) {
                NewWalletView(route: NewWalletRoute.select)
                    .navigationDestination(for: Route.self, destination: { route in
                        switch route {
                        case .cove:
                            CoveView(model: model)
                                .onAppear {
                                    print("in main view, router is: \(model.router.routes)")
                                }
                        case let .newWallet(route: route):
                            NewWalletView(route: route)
                        }
                    })
                    .onChange(of: model.router.routes) { _, new in
                        model.dispatch(event: Event.routeChanged(routes: new))
                    }
            }
            .tint(tintColor)
        }
        .environment(model)
        .environment(\.navigate) { route in
            model.pushRoute(route)
        }
    }
}

#if canImport(HotSwiftUI)
@_exported import HotSwiftUI
#elseif canImport(Inject)
@_exported import Inject
#else
// This code can be found in the Swift package:
// https://github.com/johnno1962/HotSwiftUI

#if DEBUG
import Combine

private var loadInjectionOnce: () = {
        guard objc_getClass("InjectionClient") == nil else {
            return
        }
        #if os(macOS) || targetEnvironment(macCatalyst)
        let bundleName = "macOSInjection.bundle"
        #elseif os(tvOS)
        let bundleName = "tvOSInjection.bundle"
        #elseif os(visionOS)
        let bundleName = "xrOSInjection.bundle"
        #elseif targetEnvironment(simulator)
        let bundleName = "iOSInjection.bundle"
        #else
        let bundleName = "maciOSInjection.bundle"
        #endif
        let bundlePath = "/Applications/InjectionIII.app/Contents/Resources/"+bundleName
        guard let bundle = Bundle(path: bundlePath), bundle.load() else {
            return print("""
                ⚠️ Could not load injection bundle from \(bundlePath). \
                Have you downloaded the InjectionIII.app from either \
                https://github.com/johnno1962/InjectionIII/releases \
                or the Mac App Store?
                """)
        }
}()

public let injectionObserver = InjectionObserver()

public class InjectionObserver: ObservableObject {
    @Published var injectionNumber = 0
    var cancellable: AnyCancellable? = nil
    let publisher = PassthroughSubject<Void, Never>()
    init() {
        cancellable = NotificationCenter.default.publisher(for:
            Notification.Name("INJECTION_BUNDLE_NOTIFICATION"))
            .sink { [weak self] change in
            self?.injectionNumber += 1
            self?.publisher.send()
        }
    }
}

extension SwiftUI.View {
    public func eraseToAnyView() -> some SwiftUI.View {
        _ = loadInjectionOnce
        return AnyView(self)
    }
    public func enableInjection() -> some SwiftUI.View {
        return eraseToAnyView()
    }
    public func loadInjection() -> some SwiftUI.View {
        return eraseToAnyView()
    }
    public func onInjection(bumpState: @escaping () -> ()) -> some SwiftUI.View {
        return self
            .onReceive(injectionObserver.publisher, perform: bumpState)
            .eraseToAnyView()
    }
}

@available(iOS 13.0, *)
@propertyWrapper
public struct ObserveInjection: DynamicProperty {
    @ObservedObject private var iO = injectionObserver
    public init() {}
    public private(set) var wrappedValue: Int {
        get {0} set {}
    }
}
#else
extension SwiftUI.View {
    @inline(__always)
    public func eraseToAnyView() -> some SwiftUI.View { return self }
    @inline(__always)
    public func enableInjection() -> some SwiftUI.View { return self }
    @inline(__always)
    public func loadInjection() -> some SwiftUI.View { return self }
    @inline(__always)
    public func onInjection(bumpState: @escaping () -> ()) -> some SwiftUI.View {
        return self
    }
}

@available(iOS 13.0, *)
@propertyWrapper
public struct ObserveInjection {
    public init() {}
    public private(set) var wrappedValue: Int {
        get {0} set {}
    }
}
#endif
#endif
