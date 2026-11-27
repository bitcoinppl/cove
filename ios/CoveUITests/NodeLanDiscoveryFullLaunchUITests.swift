import Darwin
import XCTest

/// Node settings search the local network only when asked, list only the servers for the selected
/// network, and save a picked server through the node check, one choice at a time
///
/// A UI test can't put an Electrum server on the simulator's network, so the app's debug-only
/// `COVE_UI_TEST_LAN_DISCOVERY` seam answers each search the screen starts with the loopback
/// servers the test scripts, which the simulator shares with this machine. Rust checks those
/// servers as it checks swept ones, so the fake node's Electrum port is listed and a server on
/// another network isn't; the Rust tests cover the sweep itself. Needs the fake node, which
/// `xtask ios-ui` (`just ios-ui-background`) starts for every run
final class NodeLanDiscoveryFullLaunchUITests: XCTestCase {
    private static let fakeServerHost = "fake-electrs.local"
    private static let slowServerHost = "slow-electrs.local"
    private static let mainnetServerHost = "mainnet-electrs.local"
    private static let signetDefaultNode = "mutinynet"

    private var app: XCUIApplication!

    override func setUp() {
        super.setUp()
        continueAfterFailure = false

        app = XCUIApplication()
        app.launchEnvironment["COVE_UI_TEST_ESPLORA_URL"] = UITestHostPorts.esploraUrl
    }

    /// Opening node settings doesn't search, and a server the search finds is checked and saved as
    /// the selected custom node when picked
    func testPickingAServerFoundOnTheNetworkSavesItAsTheSelectedNode() {
        let electrumUrl = "tcp://127.0.0.1:\(UITestHostPorts.electrumPort)"
        script([.servers([(Self.fakeServerHost, UITestHostPorts.electrumPort)])])
        openNodeSettings()

        // a search, even one made on its own, would turn the button into Search Again
        let find = app.buttons["Find Nodes on This Network"]
        XCTAssertTrue(find.waitForExistence(timeout: 10), "node settings never showed the search")
        XCTAssertFalse(foundServer(Self.fakeServerHost).exists, "opening node settings must not search the network")

        find.tap()
        let found = foundServer(Self.fakeServerHost)
        XCTAssertTrue(found.waitForExistence(timeout: 10), "the found server was never listed")
        XCTAssertTrue(app.buttons["Search Again"].exists)

        found.tap()
        let connected = app.staticText(containing: "Connected to node successfully")
        XCTAssertTrue(connected.waitForExistence(timeout: 30), "the found server was never saved")
        waitUntil(found, matches: NSPredicate(format: "selected == true"), timeout: 10, "the saved server isn't shown as selected")

        // a picked server is saved as a custom Electrum node, which the form shows from then on.
        // A tap while the popup is up only closes it, so wait for it to go first
        XCTAssertTrue(connected.waitForNonExistence(timeout: 10), "the success popup never closed")
        app.goBack()
        openNodeRow()
        XCTAssertTrue(nodeRow("Custom Electrum").waitForExistence(timeout: 10))
        XCTAssertTrue(nodeRow("Custom Electrum").isSelected, "the saved server isn't the selected node")
        XCTAssertEqual(app.textFields["Enter URL"].value as? String, electrumUrl)
    }

    /// Every server is asked which chain it serves, so one on another network is never offered
    func testAServerOnAnotherNetworkIsNotListed() throws {
        let mainnet = try ElectrumTestServer(genesisHash: ElectrumTestServer.mainnetGenesis)
        defer { mainnet.close() }

        script([.servers([(Self.mainnetServerHost, mainnet.port), (Self.fakeServerHost, UITestHostPorts.electrumPort)])])
        openNodeSettings()
        app.tapButton(named: "Find Nodes on This Network")

        XCTAssertTrue(app.buttons["Search Again"].waitForExistence(timeout: 15), "the search never finished")
        XCTAssertTrue(foundServer(Self.fakeServerHost).exists, "the signet server wasn't listed")
        XCTAssertFalse(foundServer(Self.mainnetServerHost).exists, "a mainnet server was listed on signet")
        XCTAssertGreaterThan(mainnet.requests, 0, "the mainnet server was never asked")
    }

    /// Stopping a search keeps the servers it already found
    func testStopKeepsTheServersFoundSoFar() {
        script([.servers([(Self.fakeServerHost, UITestHostPorts.electrumPort)], keepSearching: true)])
        openNodeSettings()
        app.tapButton(named: "Find Nodes on This Network")

        let found = foundServer(Self.fakeServerHost)
        XCTAssertTrue(found.waitForExistence(timeout: 10), "the found server was never listed")
        XCTAssertTrue(app.staticTexts["Searching this network…"].exists, "a running search isn't shown as searching")

        app.tapButton(named: "Stop")
        XCTAssertTrue(app.buttons["Search Again"].waitForExistence(timeout: 5), "Stop didn't stop the search")
        XCTAssertTrue(found.exists, "stopping dropped the server found")
        XCTAssertFalse(app.staticTexts["Searching this network…"].exists, "a stopped search is still shown as searching")
        XCTAssertFalse(app.staticTexts["No Electrum servers found on this network"].exists, "a stopped search says it found nothing")
    }

    /// Only one choice is checked at a time: a preset tapped while a found server is still being
    /// checked, after its loading popup is closed, is ignored until that check ends
    func testAPresetTappedWhileAFoundServerIsCheckedIsIgnoredUntilTheCheckEnds() throws {
        // proves it serves signet, then never answers the node check, which holds it open
        let slow = try ElectrumTestServer(genesisHash: ElectrumTestServer.signetGenesis)
        defer { slow.close() }

        script([.servers([(Self.slowServerHost, slow.port)])])
        openNodeSettings()
        app.tapButton(named: "Find Nodes on This Network")

        let found = foundServer(Self.slowServerHost)
        XCTAssertTrue(found.waitForExistence(timeout: 10), "the found server was never listed")
        found.tap()

        // closing the loading popup stops only the wait for it, never the check itself
        closePopup()
        let preset = nodeRow(Self.signetDefaultNode)
        preset.tap()
        Thread.sleep(forTimeInterval: 1)
        XCTAssertFalse(preset.isSelected, "a preset can't replace a node still being checked")

        XCTAssertTrue(app.staticText(containing: "Failed to connect to node").waitForExistence(timeout: 60), "the slow server's check never ended")
        closePopup()

        preset.tap()
        waitUntil(preset, matches: NSPredicate(format: "selected == true"), timeout: 30, "the preset couldn't be picked once the check ended")
    }

    /// Each way a search can end says what to do next, and Search Again searches again
    func testEachSearchOutcomeSaysWhatToDoNext() {
        script([
            .error("noLocalNetwork"),
            .error("localNetworkBlocked"),
            .servers([]),
            .servers([(Self.fakeServerHost, UITestHostPorts.electrumPort)]),
        ])
        openNodeSettings()

        app.tapButton(named: "Find Nodes on This Network")
        XCTAssertTrue(app.staticTexts["Connect to Wi-Fi to find nodes on your network"].waitForExistence(timeout: 10))

        app.tapButton(named: "Search Again")
        XCTAssertTrue(app.staticTexts.containing(NSPredicate(format: "label BEGINSWITH 'A VPN is blocking this network'")).firstMatch.waitForExistence(timeout: 10))

        app.tapButton(named: "Search Again")
        XCTAssertTrue(app.staticTexts["No Electrum servers found on this network"].waitForExistence(timeout: 10))

        // iOS never says whether local network access was denied, so an empty search points to
        // Settings, which only the person can change it in
        let openSettings = app.buttons["Open Settings"]
        XCTAssertTrue(openSettings.waitForExistence(timeout: 5), "an empty search didn't point to Settings")
        openSettings.tap()
        let settings = XCUIApplication(bundleIdentifier: "com.apple.Preferences")
        XCTAssertTrue(settings.wait(for: .runningForeground, timeout: 10), "Open Settings didn't open Settings")
        app.activate()
        XCTAssertTrue(app.buttons["Search Again"].waitForExistence(timeout: 10))

        app.tapButton(named: "Search Again")
        XCTAssertTrue(foundServer(Self.fakeServerHost).waitForExistence(timeout: 10), "Search Again didn't search again")
        XCTAssertFalse(openSettings.exists, "the Settings help stayed once a search found a server")
    }

    /// Leaving node settings while a found server is checked doesn't save the custom form over it
    ///
    /// Leaving saves what was typed into the custom form, so this edits the form, picks a server
    /// that never answers the check, closes the loading popup and goes back. Once that check has failed, the
    /// saved node must still be the one from before
    func testLeavingWhileAFoundServerIsCheckedDoesNotSaveTheForm() throws {
        // proves it serves signet, then never answers the node check, which holds it open
        let slow = try ElectrumTestServer(genesisHash: ElectrumTestServer.signetGenesis)
        defer { slow.close() }

        script([.servers([(Self.slowServerHost, slow.port)])])
        openNodeSettings()

        // the saved node is the fake Esplora custom node, so a new name is an unsaved edit
        let name = app.textFields["Node Name (optional)"]
        XCTAssertTrue(name.waitForExistence(timeout: 10), "the custom form isn't showing the saved node")
        name.tap()
        name.typeTextOnceFocused("Edited")

        app.tapButton(named: "Find Nodes on This Network")
        let found = foundServer(Self.slowServerHost)
        XCTAssertTrue(found.waitForExistence(timeout: 10), "the found server was never listed")
        found.tap()

        // closing the loading popup leaves the back button free while the check runs on
        closePopup()
        app.goBack()
        XCTAssertTrue(app.navigationBars["Settings"].waitForExistence(timeout: 10), "going back didn't leave node settings")

        let failed = app.staticText(containing: "Failed to connect to node")
        XCTAssertTrue(failed.waitForExistence(timeout: 60), "the slow server's check never ended")
        XCTAssertTrue(failed.waitForNonExistence(timeout: 15), "the failure popup never closed")

        openNodeRow()
        XCTAssertTrue(name.waitForExistence(timeout: 10), "the saved node is no longer the custom Esplora node")
        XCTAssertFalse((name.value as? String ?? "").contains("Edited"), "the form was saved over the server being checked")
    }

    // MARK: - Steps

    private enum Search {
        case error(String)
        case servers([(host: String, port: Int)], keepSearching: Bool = false)
    }

    /// Sets the search results the app's seam answers each search with, in order
    private func script(_ searches: [Search]) {
        let entries: [[String: Any]] = searches.map { search in
            switch search {
            case let .error(error):
                ["error": error]
            case let .servers(servers, keepSearching):
                [
                    "servers": servers.map { ["host": $0.host, "port": $0.port] },
                    "keepSearching": keepSearching,
                ]
            }
        }

        let json = try! JSONSerialization.data(withJSONObject: ["searches": entries])
        app.launchEnvironment["COVE_UI_TEST_LAN_DISCOVERY"] = String(decoding: json, as: UTF8.self)
    }

    private func openNodeSettings() {
        app.launch(expecting: app.staticTexts["Welcome to Cove"])
        app.importTestMnemonic()

        app.tapButton(named: "app.sidebar.open")
        app.tapButton(named: "Settings")
        XCTAssertTrue(app.navigationBars["Settings"].waitForExistence(timeout: 10))
        openNodeRow()
    }

    private func openNodeRow() {
        app.tapStaticText("Node", until: nodeRow(Self.signetDefaultNode))
    }

    private func foundServer(_ host: String) -> XCUIElement {
        app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", host)).firstMatch
    }

    private func nodeRow(_ name: String) -> XCUIElement {
        app.buttons[name].firstMatch
    }

    /// Taps outside the open popup, which closes it
    private func closePopup() {
        app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.95)).tap()
        Thread.sleep(forTimeInterval: 1)
    }
}

/// An Electrum server on this machine's loopback, which the simulator shares, that proves it
/// serves the chain whose genesis block is `genesisHash` and never answers anything else, so a
/// node check of it runs until it times out
private final class ElectrumTestServer: @unchecked Sendable {
    static let signetGenesis = "00000008819873e925422c1ff0f99f7cc9bbb232af63a077a480a3633bee1ef6"
    static let mainnetGenesis = "000000000019d6689c085ae165831e934ff763ae46a2a6c172b3f1b60a8ce26f"

    let port: Int
    private let socket: Int32
    private let genesisHash: String
    private let lock = NSLock()
    private var requestCount = 0

    /// How many requests the server has been sent
    var requests: Int {
        lock.withLock { requestCount }
    }

    init(genesisHash: String) throws {
        self.genesisHash = genesisHash

        let fd = Darwin.socket(AF_INET, SOCK_STREAM, 0)
        guard fd >= 0 else { throw POSIXError(.init(rawValue: errno) ?? .EIO) }

        var address = sockaddr_in()
        address.sin_len = UInt8(MemoryLayout<sockaddr_in>.size)
        address.sin_family = sa_family_t(AF_INET)
        address.sin_addr.s_addr = inet_addr("127.0.0.1")
        address.sin_port = 0

        let bound = withUnsafePointer(to: &address) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                bind(fd, $0, socklen_t(MemoryLayout<sockaddr_in>.size))
            }
        }

        var assigned = sockaddr_in()
        var length = socklen_t(MemoryLayout<sockaddr_in>.size)
        let named = withUnsafeMutablePointer(to: &assigned) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { getsockname(fd, $0, &length) }
        }

        guard bound == 0, listen(fd, 8) == 0, named == 0 else {
            let error = POSIXError(.init(rawValue: errno) ?? .EIO)
            Darwin.close(fd)
            throw error
        }

        socket = fd
        port = Int(UInt16(bigEndian: assigned.sin_port))
        Thread.detachNewThread { [self] in acceptConnections() }
    }

    func close() {
        shutdown(socket, SHUT_RDWR)
        Darwin.close(socket)
    }

    private func acceptConnections() {
        while true {
            let connection = accept(socket, nil, nil)
            guard connection >= 0 else { return }
            Thread.detachNewThread { [self] in answer(connection) }
        }
    }

    private func answer(_ connection: Int32) {
        defer { Darwin.close(connection) }

        var pending = Data()
        var buffer = [UInt8](repeating: 0, count: 4096)

        while true {
            let count = read(connection, &buffer, buffer.count)
            guard count > 0 else { return }
            pending.append(contentsOf: buffer[..<count])

            while let newline = pending.firstIndex(of: UInt8(ascii: "\n")) {
                let line = pending[..<newline]
                pending = Data(pending[pending.index(after: newline)...])

                guard let response = response(to: line) else { continue }
                _ = response.withUnsafeBytes { write(connection, $0.baseAddress, response.count) }
            }
        }
    }

    /// The reply to one request line, or nil for the requests this server never answers
    private func response(to line: Data) -> Data? {
        guard let request = try? JSONSerialization.jsonObject(with: line) as? [String: Any],
              let method = request["method"] as? String
        else { return nil }

        lock.withLock { requestCount += 1 }

        let result: Any
        switch method {
        case "server.version": result = ["Electrum test server 1.0", "1.4"]
        case "server.features": result = ["genesis_hash": genesisHash, "server_version": "Electrum test server 1.0"]
        default: return nil
        }

        let reply: [String: Any] = ["jsonrpc": "2.0", "id": request["id"] ?? NSNull(), "result": result]
        guard var data = try? JSONSerialization.data(withJSONObject: reply) else { return nil }
        data.append(UInt8(ascii: "\n"))
        return data
    }
}
