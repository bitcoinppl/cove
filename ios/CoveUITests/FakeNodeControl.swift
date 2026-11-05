import XCTest

/// Where the test host's services listen, see rust/xtask/src/ui_test_ports.rs
///
/// Several UI test runs on one Mac each take their own block of ports, and xtask names this run's
/// base in `COVE_UI_TEST_PORT_BASE`. Without it the tests use 39100, the first block a run takes,
/// which is where a lone `just fake-node-run` listens
enum UITestHostPorts {
    static let base = ProcessInfo.processInfo.environment["COVE_UI_TEST_PORT_BASE"].flatMap(Int.init) ?? 39100

    /// the simulator shares the host's loopback, so no forwarding is needed
    static var fakeNodeControl: URL {
        URL(string: "http://127.0.0.1:\(base)")!
    }

    static var esploraUrl: String {
        "http://127.0.0.1:\(base + 1)"
    }

    static var signerControl: URL {
        URL(string: "http://127.0.0.1:\(base + 10)")!
    }

    static var tapSignerBridge: String {
        "127.0.0.1:\(base + 11)"
    }
}

/// Synchronous client for the cove-fake-node control API, see rust/crates/cove-fake-node/README.md
struct FakeNodeControl {
    struct FundedOutput: Decodable {
        let txid: String
        let vout: UInt32
        let sats: UInt64

        var outpoint: String {
            "\(txid):\(vout)"
        }
    }

    struct Broadcast: Decodable {
        struct Output: Decodable {
            let address: String?
            let sats: UInt64
        }

        let txid: String
        let inputs: [String]
        let outputs: [Output]
    }

    struct Outspend: Decodable {
        let spent: Bool
        let spendingTxid: String?

        enum CodingKeys: String, CodingKey {
            case spent
            case spendingTxid = "spending_txid"
        }
    }

    private struct Funded: Decodable {
        let outputs: [FundedOutput]
    }

    private let baseUrl = UITestHostPorts.fakeNodeControl

    func requireHealthy() throws {
        do {
            _ = try request("GET", "health")
        } catch {
            throw ControlRequestError(
                description: "cove-fake-node is not reachable at \(baseUrl), run this test through `just ios-ui-background`: \(error)"
            )
        }
    }

    func reset() throws {
        _ = try request("POST", "reset")
    }

    func fund(address: String, sats: [UInt64]) throws -> [FundedOutput] {
        let body: [String: Any] = ["address": address, "sats": sats, "confirm": true]
        return try decode(Funded.self, request("POST", "fund", body: body)).outputs
    }

    func fundWallet(mnemonic: String, sats: [UInt64]) throws -> [FundedOutput] {
        let outputs = sats.enumerated().map { index, sats in ["index": index, "sats": sats] as [String: Any] }
        let body: [String: Any] = ["mnemonic": mnemonic, "outputs": outputs, "confirm": true]

        return try decode(Funded.self, request("POST", "fund-wallet", body: body)).outputs
    }

    func mine() throws {
        _ = try request("POST", "mine", body: ["blocks": 1])
    }

    func broadcasts() throws -> [Broadcast] {
        try decode([Broadcast].self, request("GET", "broadcasts"))
    }

    /// Polls until the app has broadcast `count` transactions
    func waitForBroadcasts(count: Int) throws -> [Broadcast] {
        let deadline = Date().addingTimeInterval(15)
        var sent = try broadcasts()
        while sent.count < count, Date() < deadline {
            Thread.sleep(forTimeInterval: 0.5)
            sent = try broadcasts()
        }

        XCTAssertEqual(sent.count, count, "unexpected broadcasts: \(sent)")
        return sent
    }

    func outspend(_ output: FundedOutput) throws -> Outspend {
        try decode(Outspend.self, request("GET", "outspend/\(output.txid)/\(output.vout)"))
    }

    private func decode<T: Decodable>(_ type: T.Type, _ data: Data) throws -> T {
        try JSONDecoder().decode(type, from: data)
    }

    private func request(_ method: String, _ path: String, body: [String: Any]? = nil) throws -> Data {
        try controlRequest(baseUrl, method, path, body: body)
    }
}

/// An error a control API returned, or that reaching it raised
struct ControlRequestError: Error, CustomStringConvertible {
    let description: String
}

/// Sends a JSON request to a host control API and waits for the response
///
/// The simulator shares the host's loopback, where xtask runs the fake node and
/// the hardware signer host
func controlRequest(_ baseUrl: URL, _ method: String, _ path: String, body: [String: Any]? = nil) throws -> Data {
    var request = URLRequest(url: baseUrl.appendingPathComponent(path), timeoutInterval: 60)
    request.httpMethod = method

    if let body {
        request.httpBody = try JSONSerialization.data(withJSONObject: body)
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
    }

    let response = ControlResponse()
    let done = DispatchSemaphore(value: 0)
    URLSession.shared.dataTask(with: request) { data, urlResponse, error in
        response.data = data
        response.status = (urlResponse as? HTTPURLResponse)?.statusCode
        response.error = error
        done.signal()
    }.resume()
    done.wait()

    if let error = response.error { throw error }

    let data = response.data ?? Data()
    guard response.status == 200 else {
        let message = (try? JSONDecoder().decode(ControlErrorBody.self, from: data))?.error ?? String(decoding: data, as: UTF8.self)
        throw ControlRequestError(description: "\(method) /\(path) returned \(response.status ?? 0): \(message)")
    }

    return data
}

private struct ControlErrorBody: Decodable {
    let error: String
}

private final class ControlResponse: @unchecked Sendable {
    var data: Data?
    var status: Int?
    var error: Error?
}
