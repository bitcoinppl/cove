import CryptoKit
import Foundation
import Network
import SwiftUI

enum TorStatus {
    case disabled
    case bootstrapping
    case ready
    case error
}

enum OrbotStatus {
    case checking
    case detected
    case notDetected
}

struct TorUiState: Equatable {
    var enabled = false
    var mode: TorMode = .builtIn
    var status: TorStatus = .disabled
    var progressPercent = 0
    var currentStep = "Disabled"
    var latestLogLine = "Tor is off"
    var logLines: [String] = ["Tor is off"]
    var externalHost = "127.0.0.1"
    var externalPort = "9050"
    var externalValidationError: String?
    var orbotStatus: OrbotStatus = .checking
    var orbotVersion: String?
}

struct TorBootstrapSnapshot: Equatable {
    var percent: Int
    var step: String
    var isReady: Bool
    var hasError: Bool
    var lastLine: String
}

struct TorApiSnapshot: Equatable {
    var isTor: Bool
    var ip: String?
    var raw: String
}

enum TorStatusDot: Equatable {
    case green
    case yellow
    case red
    case gray

    var color: Color {
        switch self {
        case .green: Color(red: 0.20, green: 0.78, blue: 0.35)
        case .yellow: Color(red: 1.00, green: 0.76, blue: 0.03)
        case .red: Color(red: 1.00, green: 0.23, blue: 0.19)
        case .gray: Color(red: 0.62, green: 0.62, blue: 0.62)
        }
    }
}

struct TorQuickStatus: Equatable {
    var enabled = false
    var overall: TorStatusDot = .gray
    var torConnection: TorStatusDot = .gray
    var nodeReachable: TorStatusDot = .gray
    var nodeSynced: TorStatusDot = .gray
    var torMessage = "Tor disabled"
    var nodeMessage = "Node status unavailable"
    var syncMessage = "Sync status unavailable"
    var logs: [String] = []
}

enum TorSupportError: LocalizedError {
    case invalidEndpoint
    case timeout
    case invalidResponse
    case notHTTP

    var errorDescription: String? {
        switch self {
        case .invalidEndpoint:
            "Invalid SOCKS endpoint"
        case .timeout:
            "Connection timed out"
        case .invalidResponse:
            "Invalid Tor API response"
        case .notHTTP:
            "Tor API did not return an HTTP response"
        }
    }
}

extension TorMode {
    static let uiCases: [TorMode] = [.builtIn, .orbot, .external]

    var title: String {
        switch self {
        case .builtIn: "Built-in"
        case .orbot: "Orbot (External App)"
        case .external: "Custom SOCKS5 Proxy"
        }
    }

    var shortTitle: String {
        switch self {
        case .builtIn: "Built-in"
        case .orbot: "Orbot"
        case .external: "Custom SOCKS5 Proxy"
        }
    }

    var persistedValue: String {
        switch self {
        case .builtIn: "BUILT_IN"
        case .orbot: "ORBOT"
        case .external: "EXTERNAL"
        }
    }

    static func fromConfig(_ value: String?) -> TorMode {
        switch value {
        case "Orbot", "ORBOT":
            .orbot
        case "External", "EXTERNAL":
            .external
        default:
            .builtIn
        }
    }
}

func torTimestamp() -> String {
    let formatter = DateFormatter()
    formatter.dateFormat = "HH:mm:ss"
    return formatter.string(from: Date())
}

func redactedEndpointForLog(_ endpoint: String) -> String {
    "id=\(shortLogId(endpoint)), onion=\(isOnionNodeUrl(endpoint))"
}

func redactedProxyForLog(host: String, port: Int) -> String {
    "id=\(shortLogId("\(host):\(port)")), port=\(port)"
}

func redactedNodeForLog(_ node: Node) -> String {
    "apiType=\(node.apiType), \(redactedEndpointForLog(node.url))"
}

private func shortLogId(_ value: String) -> String {
    let digest = SHA256.hash(data: Data(value.utf8))
    return digest.prefix(4).map { String(format: "%02x", $0) }.joined()
}

func validateTorExternalConfig(host: String, port: String) -> String? {
    if host.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
        return "Host is required"
    }

    guard let parsed = Int(port) else {
        return "Port must be a number"
    }

    guard (1 ... 65535).contains(parsed) else {
        return "Port must be between 1 and 65535"
    }

    return nil
}

func parseEndpointHostPort(_ endpoint: String) -> (String, Int)? {
    let parts = endpoint.split(separator: ":", maxSplits: 1).map(String.init)
    guard parts.count == 2, let port = Int(parts[1]), (1 ... 65535).contains(port) else {
        return nil
    }

    let host = parts[0].trimmingCharacters(in: .whitespacesAndNewlines)
    guard !host.isEmpty else { return nil }
    return (host, port)
}

func isOnionNodeUrl(_ url: String) -> Bool {
    func host(from value: String) -> String? {
        URLComponents(string: value)?.host
    }

    let rawHost = host(from: url) ?? host(from: "tcp://\(url)")
    return rawHost?.lowercased().hasSuffix(".onion") == true
}

func switchToFirstClearnetPresetNode(_ nodeSelector: NodeSelector) async throws -> Node {
    let fallbackNode = nodeSelector.nodeList().compactMap { selection -> Node? in
        guard case let .preset(node) = selection else { return nil }
        return isOnionNodeUrl(node.url) ? nil : node
    }.first

    guard let fallbackNode else {
        throw TorSupportError.invalidEndpoint
    }

    try await nodeSelector.checkAndSaveNode(node: fallbackNode)
    return fallbackNode
}

private func isRustTorLog(_ line: String) -> Bool {
    let trimmed = line.trimmingCharacters(in: .whitespacesAndNewlines)
    return trimmed.hasPrefix("[INFO ")
        || trimmed.hasPrefix("[WARN ")
        || trimmed.hasPrefix("[ERROR ")
        || trimmed.hasPrefix("[DEBUG ")
}

private func firstRegexInt(_ pattern: String, in string: String, group: Int = 1) -> Int? {
    guard let regex = try? NSRegularExpression(pattern: pattern) else { return nil }
    let range = NSRange(string.startIndex..., in: string)
    guard let match = regex.firstMatch(in: string, range: range),
          match.numberOfRanges > group,
          let valueRange = Range(match.range(at: group), in: string)
    else {
        return nil
    }

    return Int(string[valueRange])
}

private func firstRegexInts(_ pattern: String, in string: String) -> [Int]? {
    guard let regex = try? NSRegularExpression(pattern: pattern) else { return nil }
    let range = NSRange(string.startIndex..., in: string)
    guard let match = regex.firstMatch(in: string, range: range), match.numberOfRanges > 1 else {
        return nil
    }

    return (1 ..< match.numberOfRanges).compactMap { index in
        guard let valueRange = Range(match.range(at: index), in: string) else { return nil }
        return Int(string[valueRange])
    }
}

private func artiStatus(_ line: String) -> (percent: Int, message: String)? {
    let pattern = #"arti_client::status]\s*(100|[0-9]{1,2})%:\s*(.+)$"#
    guard let regex = try? NSRegularExpression(pattern: pattern) else { return nil }
    let range = NSRange(line.startIndex..., in: line)
    guard let match = regex.firstMatch(in: line, range: range),
          match.numberOfRanges == 3,
          let percentRange = Range(match.range(at: 1), in: line),
          let messageRange = Range(match.range(at: 2), in: line),
          let percent = Int(line[percentRange])
    else {
        return nil
    }

    return (min(max(percent, 0), 100), String(line[messageRange]))
}

func deriveBuiltInBootstrapSnapshot(_ logLines: [String]) -> TorBootstrapSnapshot {
    guard !logLines.isEmpty else {
        return TorBootstrapSnapshot(
            percent: 0,
            step: "Waiting for Tor runtime",
            isReady: false,
            hasError: false,
            lastLine: "No Tor logs yet"
        )
    }

    let rustLogs = logLines.filter(isRustTorLog)
    guard !rustLogs.isEmpty else {
        return TorBootstrapSnapshot(
            percent: 0,
            step: "Waiting for Tor runtime",
            isReady: false,
            hasError: false,
            lastLine: logLines.last ?? "No Tor logs yet"
        )
    }

    let restartMarkers = [
        "built-in tor endpoint requested without cache; launching proxy",
        "built-in tor launch initiated",
        "starting built-in tor runtime thread",
    ]
    let restartIndex = rustLogs.indices.last { index in
        let line = rustLogs[index].lowercased()
        return restartMarkers.contains { line.contains($0) }
    }
    let scopedLogs = restartIndex.map { Array(rustLogs[$0...]) } ?? rustLogs

    var percent = 0
    var step = "Starting Tor"
    var ready = false
    var hasError = false
    var initialMissingMicrodescriptors: Int?

    for line in scopedLogs {
        let lowered = line.lowercased()
        if let status = artiStatus(line) {
            percent = status.percent
            step = "\(status.percent)%: \(status.message)"
            if status.percent >= 100 {
                ready = true
            }
            continue
        }

        if let found = firstRegexInt(#"\b(100|[0-9]{1,2})%\b"#, in: line), found > percent {
            percent = found
        }

        if lowered.contains("built-in tor launch initiated")
            || lowered.contains("starting built-in tor runtime thread")
        {
            percent = max(percent, 3)
            step = "Launching runtime"
        } else if lowered.contains("built-in tor runtime created")
            || lowered.contains("launching arti socks proxy task")
        {
            percent = max(percent, 8)
            step = "Starting SOCKS proxy"
        } else if lowered.contains("listening on"),
                  lowered.contains("127.0.0.1:") || lowered.contains("[::1]:")
        {
            percent = max(percent, 15)
            step = "SOCKS listener ready"
        } else if lowered.contains("looking for a consensus") {
            percent = max(percent, 22)
            step = "Looking for consensus"
        } else if lowered.contains("downloading certificates for consensus") {
            percent = max(percent, 35)
            step = "Downloading consensus certificates"
        } else if lowered.contains("downloading microdescriptors") {
            step = "Downloading microdescriptors"
            let missingFraction = firstRegexInts(#"missing\s+(\d+)\s*/\s*(\d+)"#, in: lowered)
            let missing = missingFraction?.first ?? firstRegexInt(#"missing\s+(\d+)"#, in: lowered)

            if let missing {
                let baseline: Int
                if let total = missingFraction?.dropFirst().first, total > 0 {
                    if initialMissingMicrodescriptors == nil || total > initialMissingMicrodescriptors! {
                        initialMissingMicrodescriptors = total
                    }
                    baseline = max(initialMissingMicrodescriptors ?? total, 1)
                } else {
                    if initialMissingMicrodescriptors == nil || missing > initialMissingMicrodescriptors! {
                        initialMissingMicrodescriptors = missing
                    }
                    baseline = max(initialMissingMicrodescriptors ?? missing, 1)
                }

                let completedRatio = Double(max(baseline - missing, 0)) / Double(baseline)
                let dynamicPercent = min(max(45 + Int(completedRatio * 46.0), 45), 91)
                percent = max(percent, dynamicPercent)
            } else {
                percent = max(percent, 45)
            }
        } else if lowered.contains("marked consensus usable") {
            percent = max(percent, 93)
            step = "Building circuits"
        } else if lowered.contains("enough information to build circuits") {
            percent = max(percent, 96)
            step = "Building circuits"
        } else if lowered.contains("directory is complete") {
            percent = 100
            step = "Tor ready"
            ready = true
        } else if lowered.contains("sufficiently bootstrapped; proxy now functional") {
            percent = max(percent, 97)
            step = "Circuits available, finishing directory"
        }

        let benignReloadXdgWarning = lowered.contains("arti::reload_cfg")
            && (lowered.contains("xdg project directories")
                || lowered.contains("unable to determine home directory")
                || lowered.contains("cache_dir"))
        let fatalSignals = [
            "built-in tor bootstrap failed",
            "built-in tor proxy exited",
            "failed to initialize built-in tor runtime",
            "failed to create built-in tor runtime",
            "built-in tor socks listener not ready",
            "can't find path for port_info_file",
            "operation not supported because arti feature disabled",
        ]

        if !benignReloadXdgWarning, fatalSignals.contains(where: { lowered.contains($0) }) {
            hasError = true
        }
    }

    if ready {
        hasError = false
    }

    if !ready, percent >= 100 {
        percent = 99
    }

    return TorBootstrapSnapshot(
        percent: min(max(percent, 0), 100),
        step: step,
        isReady: ready,
        hasError: hasError,
        lastLine: scopedLogs.last ?? rustLogs.last ?? logLines.last ?? "No Tor logs yet"
    )
}

func testSocksEndpoint(host: String, port: Int, timeout: TimeInterval = 3) async -> Result<Void, Error> {
    guard (1 ... 65535).contains(port),
          let endpointPort = NWEndpoint.Port(rawValue: UInt16(port))
    else {
        return .failure(TorSupportError.invalidEndpoint)
    }

    return await withCheckedContinuation { continuation in
        let queue = DispatchQueue(label: "org.bitcoinppl.cove.tor.socks-test")
        let connection = NWConnection(host: NWEndpoint.Host(host), port: endpointPort, using: .tcp)
        var finished = false

        func finish(_ result: Result<Void, Error>) {
            guard !finished else { return }
            finished = true
            connection.cancel()
            continuation.resume(returning: result)
        }

        connection.stateUpdateHandler = { state in
            switch state {
            case .ready:
                finish(.success(()))
            case let .failed(error):
                finish(.failure(error))
            case let .waiting(error):
                if case .posix(.ECONNREFUSED) = error {
                    finish(.failure(error))
                }
            default:
                break
            }
        }

        queue.asyncAfter(deadline: .now() + timeout) {
            finish(.failure(TorSupportError.timeout))
        }
        connection.start(queue: queue)
    }
}

func testTorApiThroughSocks(host: String, port: Int, timeout: TimeInterval = 15) async -> Result<TorApiSnapshot, Error> {
    guard (1 ... 65535).contains(port) else {
        return .failure(TorSupportError.invalidEndpoint)
    }

    let httpsResult = await testTorApiThroughUrlSessionSocks(host: host, port: port, timeout: timeout)
    if httpsResult.isSuccess() {
        return httpsResult
    }

    return httpsResult
}

private func testTorApiThroughUrlSessionSocks(
    host: String,
    port: Int,
    timeout: TimeInterval
) async -> Result<TorApiSnapshot, Error> {
    do {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.timeoutIntervalForRequest = timeout
        configuration.timeoutIntervalForResource = timeout
        configuration.connectionProxyDictionary = [
            "SOCKSEnable": NSNumber(value: true),
            "SOCKSProxy": host,
            "SOCKSPort": NSNumber(value: port),
        ]

        let session = URLSession(configuration: configuration)
        defer { session.invalidateAndCancel() }

        guard let url = URL(string: "https://check.torproject.org/api/ip") else {
            return .failure(TorSupportError.invalidEndpoint)
        }

        let (data, response) = try await session.data(from: url)
        guard let http = response as? HTTPURLResponse else {
            return .failure(TorSupportError.notHTTP)
        }
        guard (200 ..< 300).contains(http.statusCode) else {
            return .failure(TorSupportError.invalidResponse)
        }

        return parseTorApiJson(data)
    } catch {
        return .failure(error)
    }
}

private func parseTorApiJson(_ data: Data) -> Result<TorApiSnapshot, Error> {
    let raw = String(data: data, encoding: .utf8) ?? ""
    let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
    let isTor = (json?["IsTor"] as? Bool)
        ?? (json?["is_tor"] as? Bool)
        ?? (raw.range(
            of: #""istor"\s*:\s*true"#,
            options: [.caseInsensitive, .regularExpression]
        ) != nil)
    let ip = json?["IP"] as? String

    return .success(TorApiSnapshot(isTor: isTor, ip: ip, raw: raw))
}
