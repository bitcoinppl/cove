import SwiftUI
import UIKit

private let diagnosticsFilename = "cove-diagnostics.txt"
private let previewChunkSize = 4096

private struct DiagnosticsPreviewChunk: Identifiable {
    let id: Int
    let text: String
}

private enum DiagnosticsLoadState: Equatable {
    case loading
    case ready
    case failed(String)
}

private enum SendDiagnosticsAlert: Identifiable, Equatable {
    case confirmClear
    case error(String)
    case submitted(String)

    var id: String {
        switch self {
        case .confirmClear:
            "confirm-clear"
        case let .error(message):
            "error-\(message)"
        case let .submitted(reportId):
            "submitted-\(reportId)"
        }
    }
}

struct SendDiagnosticsSheet: View {
    @Environment(\.dismiss) private var dismiss

    @State private var report: DiagnosticsReport? = nil
    @State private var previewText = ""
    @State private var previewChunks: [DiagnosticsPreviewChunk] = []
    @State private var description = ""
    @State private var reportSize = ""
    @State private var reportId: String? = nil
    @State private var loadState = DiagnosticsLoadState.loading
    @State private var isSubmitting = false
    @State private var alertState: SendDiagnosticsAlert? = nil

    private var isReady: Bool {
        switch loadState {
        case .ready:
            true
        case .loading, .failed:
            false
        }
    }

    private var exportText: String {
        report?.previewTextForDescription(description: description) ?? previewText
    }

    var body: some View {
        NavigationStack {
            Group {
                switch loadState {
                case .loading:
                    ProgressView("Building diagnostics...")
                        .frame(maxWidth: .infinity, maxHeight: .infinity)

                case let .failed(message):
                    VStack(spacing: 16) {
                        Text("Diagnostics Unavailable")
                            .font(.headline)

                        Text(message)
                            .font(.subheadline)
                            .foregroundStyle(.secondary)
                            .multilineTextAlignment(.center)

                        Button("Retry") {
                            Task { await rebuildReport(clearStoredLogs: false) }
                        }
                        .buttonStyle(.borderedProminent)
                    }
                    .padding()
                    .frame(maxWidth: .infinity, maxHeight: .infinity)

                case .ready:
                    readyContent
                }
            }
            .navigationTitle("Send Diagnostics")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Done") {
                        guard !isSubmitting else { return }

                        dismiss()
                    }
                    .disabled(isSubmitting)
                }
            }
        }
        .task {
            if report == nil {
                await rebuildReport(clearStoredLogs: false)
            }
        }
        .onChange(of: description) { _, _ in
            refreshPreviewForCurrentDescription()
        }
        .alert(item: $alertState) { alert in
            switch alert {
            case .confirmClear:
                Alert(
                    title: Text("Clear Stored Logs?"),
                    message: Text("This deletes stored diagnostics logs on this device and rebuilds the preview."),
                    primaryButton: .destructive(Text("Clear")) {
                        Task { await rebuildReport(clearStoredLogs: true) }
                    },
                    secondaryButton: .cancel()
                )

            case let .error(message):
                Alert(
                    title: Text("Something went wrong"),
                    message: Text(message),
                    dismissButton: .default(Text("OK"))
                )

            case let .submitted(reportId):
                Alert(
                    title: Text("Diagnostics Sent"),
                    message: Text("Report ID: \(reportId)"),
                    primaryButton: .default(Text("Copy ID")) {
                        UIPasteboard.general.string = reportId
                    },
                    secondaryButton: .default(Text("Done")) {
                        dismiss()
                    }
                )
            }
        }
        .interactiveDismissDisabled(isSubmitting)
    }

    private var readyContent: some View {
        VStack(alignment: .leading, spacing: 14) {
            VStack(alignment: .leading, spacing: 8) {
                Text("Description")
                    .font(.headline)

                TextEditor(text: $description)
                    .frame(minHeight: 84, maxHeight: 120)
                    .padding(8)
                    .background(Color(.secondarySystemBackground))
                    .clipShape(RoundedRectangle(cornerRadius: 8))
            }

            HStack {
                Text("Preview")
                    .font(.headline)

                Spacer()

                if !reportSize.isEmpty {
                    Text(reportSize)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }

            ScrollView {
                LazyVStack(alignment: .leading, spacing: 0) {
                    ForEach(previewChunks) { chunk in
                        Text(chunk.text)
                            .font(.system(.caption, design: .monospaced))
                            .foregroundStyle(.primary)
                            .textSelection(.enabled)
                            .frame(maxWidth: .infinity, alignment: .leading)
                    }
                }
                .padding(12)
            }
            .background(Color(.secondarySystemBackground))
            .clipShape(RoundedRectangle(cornerRadius: 8))

            if let reportId {
                VStack(alignment: .leading, spacing: 8) {
                    Text("Diagnostics sent")
                        .font(.headline)

                    Text(reportId)
                        .font(.system(.callout, design: .monospaced))
                        .textSelection(.enabled)

                    HStack {
                        Button("Copy ID") {
                            UIPasteboard.general.string = reportId
                        }
                        .buttonStyle(.bordered)

                        Button("Done") {
                            dismiss()
                        }
                        .buttonStyle(.borderedProminent)
                    }
                }
                .padding(12)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(Color(.secondarySystemBackground))
                .clipShape(RoundedRectangle(cornerRadius: 8))
            }

            HStack(spacing: 12) {
                Button("Share") {
                    shareDiagnostics()
                }
                .buttonStyle(.bordered)
                .disabled(!isReady || isSubmitting)

                Button("Clear Stored Logs", role: .destructive) {
                    alertState = .confirmClear
                }
                .buttonStyle(.bordered)
                .disabled(isSubmitting)
            }

            Button {
                Task { await submitReport() }
            } label: {
                if isSubmitting {
                    ProgressView()
                        .frame(maxWidth: .infinity)
                } else {
                    Text("Submit")
                        .frame(maxWidth: .infinity)
                }
            }
            .buttonStyle(.borderedProminent)
            .disabled(report == nil || isSubmitting || reportId != nil)
        }
        .padding()
    }

    @MainActor
    private func rebuildReport(clearStoredLogs: Bool) async {
        loadState = .loading
        reportId = nil
        report = nil
        previewText = ""
        previewChunks = []
        reportSize = ""

        do {
            if clearStoredLogs {
                try clearDiagnosticsLogs()
            }

            let nextReport = try await buildDiagnosticsReport(
                platform: IOSDiagnostics.platformInfo(),
                platformLogs: IOSDiagnostics.platformLogs()
            )

            report = nextReport
            refreshPreview(report: nextReport)
            loadState = .ready
        } catch {
            loadState = .failed(error.localizedDescription)
        }
    }

    @MainActor
    private func submitReport() async {
        guard let report else { return }

        isSubmitting = true
        defer { isSubmitting = false }

        do {
            let nextReportId = try await report.submit(description: description)
            reportId = nextReportId
            alertState = .submitted(nextReportId)
        } catch {
            alertState = .error(error.localizedDescription)
        }
    }

    @MainActor
    private func shareDiagnostics() {
        ShareSheet.present(data: exportText, filename: diagnosticsFilename) { success in
            if !success { Log.warn("Diagnostics share cancelled or failed") }
        }
    }

    @MainActor
    private func refreshPreviewForCurrentDescription() {
        guard let report else { return }

        refreshPreview(report: report)
    }

    @MainActor
    private func refreshPreview(report: DiagnosticsReport) {
        let nextPreviewText = report.previewTextForDescription(description: description)

        previewText = nextPreviewText
        previewChunks = Self.chunks(for: nextPreviewText)
        reportSize = ByteCountFormatter.string(
            fromByteCount: Int64(report.sizeBytesForDescription(description: description)),
            countStyle: .file
        )
    }

    private static func chunks(for text: String) -> [DiagnosticsPreviewChunk] {
        var chunks: [DiagnosticsPreviewChunk] = []
        var start = text.startIndex
        var chunkId = 0

        while start < text.endIndex {
            let end = text.index(start, offsetBy: previewChunkSize, limitedBy: text.endIndex) ?? text.endIndex
            chunks.append(DiagnosticsPreviewChunk(id: chunkId, text: String(text[start ..< end])))
            start = end
            chunkId += 1
        }

        return chunks
    }
}

private enum IOSDiagnostics {
    static func platformInfo() -> DiagnosticsPlatformInfo {
        DiagnosticsPlatformInfo(
            platform: "iOS",
            buildNumber: bundleValue("CFBundleVersion"),
            osVersion: UIDevice.current.systemVersion,
            deviceModel: deviceModelIdentifier()
        )
    }

    static func platformLogs() -> String {
        [
            "iOS system logs are unavailable to sandboxed apps.",
            "Generated: \(ISO8601DateFormatter().string(from: Date()))",
            "App version: \(bundleValue("CFBundleShortVersionString"))",
            "Build: \(bundleValue("CFBundleVersion"))",
            "iOS: \(UIDevice.current.systemVersion)",
            "Device: \(deviceModelIdentifier())",
            "Low power mode: \(ProcessInfo.processInfo.isLowPowerModeEnabled)",
            "Thermal state: \(thermalStateDescription(ProcessInfo.processInfo.thermalState))",
        ].joined(separator: "\n")
    }

    private static func bundleValue(_ key: String) -> String {
        Bundle.main.object(forInfoDictionaryKey: key) as? String ?? "unknown"
    }

    private static func deviceModelIdentifier() -> String {
        var systemInfo = utsname()
        uname(&systemInfo)

        let mirror = Mirror(reflecting: systemInfo.machine)
        return mirror.children.reduce(into: "") { identifier, element in
            guard let value = element.value as? Int8, value != 0 else { return }
            identifier.append(String(UnicodeScalar(UInt8(value))))
        }
    }

    private static func thermalStateDescription(_ state: ProcessInfo.ThermalState) -> String {
        switch state {
        case .nominal:
            "nominal"
        case .fair:
            "fair"
        case .serious:
            "serious"
        case .critical:
            "critical"
        @unknown default:
            "unknown"
        }
    }
}
