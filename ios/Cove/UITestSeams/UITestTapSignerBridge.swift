//
//  UITestTapSignerBridge.swift
//  Cove
//
//  Debug-only stand-in for TAPSIGNER NFC sessions in UI tests
//

#if DEBUG
    import CoveCore
    import Darwin
    import Foundation

    private let logger = Log(id: "UITestTapSignerBridge")

    /// A TCP bridge to a TAPSIGNER card emulator that replaces NFC in UI tests
    ///
    /// When `COVE_UI_TEST_TAPSIGNER_BRIDGE` is `host:port`, TAPSIGNER commands
    /// and NFC tag reads skip the NFC session and talk to the bridge that
    /// `xtask ui-test-host` serves. Each APDU goes out as a frame with a 4-byte
    /// big-endian length and the card's response comes back the same way, so
    /// Rust drives the same command sequence it sends a real card. The bridge
    /// closing the connection looks like the card leaving the NFC field
    struct UITestTapSignerBridge {
        static let environmentKey = "COVE_UI_TEST_TAPSIGNER_BRIDGE"

        // `{"cmd": "nfc"}` in a CBOR APDU, which returns the URL a card's NDEF record holds
        private static let nfcUrlApdu = Data([
            0x00, 0xCB, 0x00, 0x00, 0x09, 0xA1, 0x63, 0x63, 0x6D, 0x64, 0x63, 0x6E, 0x66, 0x63,
        ])

        let host: String
        let port: UInt16

        static var configured: UITestTapSignerBridge? {
            guard let value = ProcessInfo.processInfo.environment[environmentKey] else {
                return nil
            }

            guard let separator = value.lastIndex(of: ":"),
                  let port = UInt16(value[value.index(after: separator)...])
            else {
                logger.error("\(environmentKey) must be host:port, got \(value)")
                return nil
            }

            return UITestTapSignerBridge(host: String(value[..<separator]), port: port)
        }

        func transport(onMessage: @escaping @Sendable (String) -> Void) -> UITestTapCardTransport {
            UITestTapCardTransport(bridge: self, onMessage: onMessage)
        }

        /// The URL a TAPSIGNER's NDEF record holds, as an NFC tag read returns it
        func readNdefUrl() async throws -> String {
            let transport = transport(onMessage: { _ in })
            defer { transport.close() }

            let response = try await transport.transmitApdu(commandApdu: Self.nfcUrlApdu)
            guard let url = Self.cborUrl(in: response) else {
                throw TransportError.UnknownError("The card emulator returned no NFC URL")
            }

            return url
        }

        /// Reads `{"url": text}` from a card response, ignoring the trailing status word
        private static func cborUrl(in response: Data) -> String? {
            let bytes = [UInt8](response)
            let prefix: [UInt8] = [0xA1, 0x63, 0x75, 0x72, 0x6C]
            guard bytes.count > prefix.count, Array(bytes[..<prefix.count]) == prefix else {
                return nil
            }

            // a CBOR text string header: major type 3 with an inline, 1-byte or 2-byte length
            var index = prefix.count
            let header = bytes[index]
            index += 1

            let length: Int
            switch header {
            case 0x60 ... 0x77:
                length = Int(header - 0x60)
            case 0x78 where bytes.count > index:
                length = Int(bytes[index])
                index += 1
            case 0x79 where bytes.count > index + 1:
                length = Int(bytes[index]) << 8 | Int(bytes[index + 1])
                index += 2
            default:
                return nil
            }

            guard bytes.count >= index + length else { return nil }
            return String(bytes: bytes[index ..< index + length], encoding: .utf8)
        }
    }

    /// Sends a TAPSIGNER session's APDUs over one bridge connection
    final class UITestTapCardTransport: TapcardTransportProtocol, @unchecked Sendable {
        private static let receiveTimeoutSeconds = 30

        private let bridge: UITestTapSignerBridge
        private let onMessage: @Sendable (String) -> Void
        private let queue = DispatchQueue(label: "cove.uitest.tapsigner-bridge")
        private let lock = NSLock()
        private var socketFd: Int32 = -1
        private var message = ""

        fileprivate init(bridge: UITestTapSignerBridge, onMessage: @escaping @Sendable (String) -> Void) {
            self.bridge = bridge
            self.onMessage = onMessage
        }

        deinit {
            close()
        }

        func setMessage(message: String) {
            let current = lock.withLock {
                self.message = message
                return self.message
            }

            onMessage(current)
        }

        func appendMessage(message: String) {
            let current = lock.withLock {
                self.message += message
                return self.message
            }

            onMessage(current)
        }

        func transmitApdu(commandApdu: Data) async throws -> Data {
            try await withCheckedThrowingContinuation { continuation in
                queue.async { [self] in
                    do {
                        try continuation.resume(returning: exchange(commandApdu))
                    } catch {
                        // a lost connection is what a card leaving the NFC field looks like
                        logger.error("TAPSIGNER bridge exchange failed: \(error)")
                        close()
                        continuation.resume(
                            throwing: TransportError.Transport(
                                "Tag connection lost, please hold your phone still"
                            )
                        )
                    }
                }
            }
        }

        func close() {
            lock.withLock {
                if socketFd >= 0 {
                    Darwin.close(socketFd)
                    socketFd = -1
                }
            }
        }

        private func exchange(_ apdu: Data) throws -> Data {
            let fd = try connectedSocket()

            var length = UInt32(apdu.count).bigEndian
            try writeAll(fd, Data(bytes: &length, count: 4) + apdu)

            let header = try readExactly(fd, count: 4)
            let responseLength = header.withUnsafeBytes { $0.loadUnaligned(as: UInt32.self) }.bigEndian
            return try readExactly(fd, count: Int(responseLength))
        }

        private func connectedSocket() throws -> Int32 {
            if let fd = lock.withLock({ socketFd >= 0 ? socketFd : nil }) {
                return fd
            }

            let fd = socket(AF_INET, SOCK_STREAM, 0)
            guard fd >= 0 else { throw BridgeError.posix("socket", errno) }

            var noSigPipe: Int32 = 1
            setsockopt(fd, SOL_SOCKET, SO_NOSIGPIPE, &noSigPipe, socklen_t(MemoryLayout<Int32>.size))

            var timeout = timeval(tv_sec: Self.receiveTimeoutSeconds, tv_usec: 0)
            setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &timeout, socklen_t(MemoryLayout<timeval>.size))

            var address = sockaddr_in()
            address.sin_family = sa_family_t(AF_INET)
            address.sin_port = bridge.port.bigEndian
            guard inet_pton(AF_INET, bridge.host, &address.sin_addr) == 1 else {
                Darwin.close(fd)
                throw BridgeError.invalidHost(bridge.host)
            }

            let result = withUnsafePointer(to: &address) {
                $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                    connect(fd, $0, socklen_t(MemoryLayout<sockaddr_in>.size))
                }
            }

            guard result == 0 else {
                let error = errno
                Darwin.close(fd)
                throw BridgeError.posix("connect", error)
            }

            lock.withLock { socketFd = fd }
            return fd
        }

        private func writeAll(_ fd: Int32, _ data: Data) throws {
            try data.withUnsafeBytes { buffer in
                var offset = 0
                while offset < buffer.count {
                    let written = send(fd, buffer.baseAddress! + offset, buffer.count - offset, 0)
                    guard written > 0 else { throw BridgeError.posix("send", errno) }
                    offset += written
                }
            }
        }

        private func readExactly(_ fd: Int32, count: Int) throws -> Data {
            var data = Data(count: count)
            var offset = 0

            try data.withUnsafeMutableBytes { buffer in
                while offset < count {
                    let read = recv(fd, buffer.baseAddress! + offset, count - offset, 0)
                    guard read > 0 else { throw BridgeError.closed }
                    offset += read
                }
            }

            return data
        }
    }

    private enum BridgeError: Error, CustomStringConvertible {
        case posix(String, Int32)
        case invalidHost(String)
        case closed

        var description: String {
            switch self {
            case let .posix(call, code): "\(call) failed: \(String(cString: strerror(code)))"
            case let .invalidHost(host): "\(host) is not an IPv4 address"
            case .closed: "the bridge closed the connection"
            }
        }
    }
#endif
