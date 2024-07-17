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
    @State var id = UUID()

    public init() {
        // initialize keychain
        _ = Keychain(keychain: KeychainAccessor())

        model = MainViewModel()
    }

    var tintColor: Color {
        switch model.currentRoute {
        case .newWallet(.hotWallet(.create)):
            Color.white
        case .newWallet(.hotWallet(.verifyWords)):
            Color.white
        case .selectedWallet:
            Color.white
        default:
            Color.blue
        }
    }

    var body: some Scene {
        WindowGroup {
            ZStack {
                NavigationStack(path: $model.router.routes) {
                    RouteView(model: model)
                        .navigationDestination(for: Route.self, destination: { route in
                            RouteView(model: model, route: route)
                        })
                        .toolbar {
                            ToolbarItem(placement: .navigationBarLeading) {
                                Button(action: {
                                    withAnimation {
                                        model.toggleSidebar()
                                    }
                                }) {
                                    Image(systemName: "line.horizontal.3")
                                }
                                .frame(minWidth: 50, minHeight: 50)
                            }
                        }
                }
                .tint(tintColor)

                SidebarView(isShowing: $model.isSidebarVisible, currentRoute: model.currentRoute)
            }
            .id(id)
            .tint(tintColor)
            .environment(\.navigate) { route in
                model.pushRoute(route)
            }
            .environment(model)
            .preferredColorScheme(model.colorScheme)
            .onChange(of: model.router.routes) { old, new in
                if !old.isEmpty && new.isEmpty {
                    id = UUID()
                }

                model.dispatch(action: AppAction.updateRoute(routes: new))
            }
            .onChange(of: model.selectedNetwork) {
                id = UUID()
            }
            .gesture(
                model.router.routes.isEmpty ?
                    DragGesture()
                    .onChanged { gesture in
                        if gesture.startLocation.x < 25, gesture.translation.width > 100 {
                            withAnimation(.spring()) {
                                model.isSidebarVisible = true
                            }
                        }
                    }
                    .onEnded { gesture in
                        if gesture.startLocation.x < 20, gesture.translation.width > 50 {
                            withAnimation(.spring()) {
                                model.isSidebarVisible = true
                            }
                        }
                    } : nil
            )
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
            let bundlePath = "/Applications/InjectionIII.app/Contents/Resources/" + bundleName
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
                    .sink { [weak self] _ in
                        self?.injectionNumber += 1
                        self?.publisher.send()
                    }
            }
        }

        public extension SwiftUI.View {
            func eraseToAnyView() -> some SwiftUI.View {
                _ = loadInjectionOnce
                return AnyView(self)
            }

            func enableInjection() -> some SwiftUI.View {
                return eraseToAnyView()
            }

            func loadInjection() -> some SwiftUI.View {
                return eraseToAnyView()
            }

            func onInjection(bumpState: @escaping () -> Void) -> some SwiftUI.View {
                return onReceive(injectionObserver.publisher, perform: bumpState)
                    .eraseToAnyView()
            }
        }

        @available(iOS 13.0, *)
        @propertyWrapper
        public struct ObserveInjection: DynamicProperty {
            @ObservedObject private var iO = injectionObserver
            public init() {}
            public private(set) var wrappedValue: Int {
                get { 0 } set {}
            }
        }
    #else
        public extension SwiftUI.View {
            @inline(__always)
            func eraseToAnyView() -> some SwiftUI.View { return self }
            @inline(__always)
            func enableInjection() -> some SwiftUI.View { return self }
            @inline(__always)
            func loadInjection() -> some SwiftUI.View { return self }
            @inline(__always)
            func onInjection(bumpState _: @escaping () -> Void) -> some SwiftUI.View {
                return self
            }
        }

        @available(iOS 13.0, *)
        @propertyWrapper
        public struct ObserveInjection {
            public init() {}
            public private(set) var wrappedValue: Int {
                get { 0 } set {}
            }
        }
    #endif
#endif
