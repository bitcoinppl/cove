import Foundation

extension ScanManager {
    @MainActor
    func handleKeyTeleportText(_ text: String) -> Bool {
        do {
            let multiFormat = try StringOrData(text).toMultiFormat()
            switch multiFormat {
            case .keyTeleportReceiver, .keyTeleportSender:
                handleMultiFormat(multiFormat)
                return true
            default:
                return false
            }
        } catch let error as MultiFormatError {
            return handleKeyTeleportTextError(error, text: text)
        } catch {
            return false
        }
    }

    @MainActor
    func handleKeyTeleportUrl(_ url: URL) -> Bool {
        guard isKeyTeleportHost(url.host) else {
            return false
        }

        do {
            let multiFormat = try StringOrData(url.absoluteString).toMultiFormat()
            handleMultiFormat(multiFormat)
            return true
        } catch {
            app.alertState = .init(
                .invalidFormat(message: "This KeyTeleport link is not supported.")
            )
            return true
        }
    }

    @MainActor
    func handleKeyTeleportReceiver(_ packet: KeyTeleportReceiverPacket) {
        guard canIngestKeyTeleportPacket(requiring: .send) else {
            showKeyTeleportDirectionConflict(requiredDirection: .send)
            return
        }

        let manager = app.ensureKeyTeleportManager()
        manager.ingest(packet)
        app.navigateToKeyTeleport(.send)
    }

    @MainActor
    func handleKeyTeleportSender(_ packet: KeyTeleportSenderPacket) {
        guard canIngestKeyTeleportPacket(requiring: .receive) else {
            showKeyTeleportDirectionConflict(requiredDirection: .receive)
            return
        }

        let manager = app.ensureKeyTeleportManager()
        manager.ingest(packet)
        app.navigateToKeyTeleport(.receive)
    }

    @MainActor
    private func handleKeyTeleportTextError(_ error: MultiFormatError, text: String) -> Bool {
        if case .KeyTeleportPsbtNotSupported = error {
            app.alertState = TaggedItem(
                .invalidFormat(message: "KeyTeleport PSBT packets are not supported yet.")
            )
            return true
        }

        guard looksLikeKeyTeleportPacket(text) else {
            return false
        }

        app.alertState = TaggedItem(
            .invalidFormat(message: "This KeyTeleport packet could not be read.")
        )
        return true
    }

    private func looksLikeKeyTeleportPacket(_ text: String) -> Bool {
        let normalized = text.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()

        return normalized.contains("keyteleport.com")
            || normalized.hasPrefix("b$2r")
            || normalized.hasPrefix("b$2s")
            || normalized.hasPrefix("b$2p")
    }

    private func isKeyTeleportHost(_ host: String?) -> Bool {
        guard let host = host?.lowercased().trimmingCharacters(in: CharacterSet(charactersIn: "."))
        else {
            return false
        }

        return host == "keyteleport.com" || host.hasSuffix(".keyteleport.com")
    }

    private func canIngestKeyTeleportPacket(
        requiring requiredDirection: KeyTeleportFlowDirection
    ) -> Bool {
        let routeDirections = [app.router.default.keyTeleportFlowDirection]
            + app.router.routes.map(\.keyTeleportFlowDirection)
        let activeDirections = routeDirections.compactMap(\.self)
            + [app.keyTeleportManager?.flowDirection].compactMap(\.self)

        return KeyTeleportPacketRoutingDecision.resolve(
            activeDirections: activeDirections,
            requiredDirection: requiredDirection
        ) == .accept
    }

    private func showKeyTeleportDirectionConflict(
        requiredDirection: KeyTeleportFlowDirection
    ) {
        let activeFlow = requiredDirection == .send ? "receive" : "send"
        let requestedFlow = requiredDirection == .send ? "send" : "receive"
        app.alertState = .init(
            .general(
                title: "KeyTeleport Session Active",
                message: "End the active \(activeFlow) session before starting a \(requestedFlow) session."
            )
        )
    }
}
