import SwiftUI
import UIKit

private enum SubmittedDiagnosticsAlert: Identifiable, Equatable {
    case confirmClear
    case error(String)

    var id: String {
        switch self {
        case .confirmClear:
            "confirm-clear"
        case let .error(message):
            "error-\(message)"
        }
    }
}

struct SubmittedDiagnosticsSheet: View {
    @Environment(\.dismiss) private var dismiss

    @State private var records: [DiagnosticsReportRecord]
    @State private var alertState: SubmittedDiagnosticsAlert? = nil

    let onRecordsChanged: () -> Void

    init(records: [DiagnosticsReportRecord], onRecordsChanged: @escaping () -> Void) {
        _records = State(initialValue: records)
        self.onRecordsChanged = onRecordsChanged
    }

    var body: some View {
        NavigationStack {
            Group {
                if records.isEmpty {
                    VStack(spacing: 8) {
                        Text("No submitted diagnostics")
                            .font(.headline)
                        Text("Submitted report IDs will appear here.")
                            .font(.subheadline)
                            .foregroundStyle(.secondary)
                    }
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                } else {
                    List(records, id: \.reportId) { record in
                        SubmittedDiagnosticsRow(record: record)
                    }
                }
            }
            .navigationTitle("Submitted Diagnostics")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Done") { dismiss() }
                }

                ToolbarItem(placement: .primaryAction) {
                    Button("Clear", role: .destructive) {
                        alertState = .confirmClear
                    }
                    .disabled(records.isEmpty)
                }
            }
        }
        .alert(item: $alertState) { alert in
            switch alert {
            case .confirmClear:
                Alert(
                    title: Text("Clear Submitted Diagnostics?"),
                    message: Text("This removes saved report IDs from this device."),
                    primaryButton: .destructive(Text("Clear")) {
                        clearHistory()
                    },
                    secondaryButton: .cancel()
                )

            case let .error(message):
                Alert(
                    title: Text("Something went wrong"),
                    message: Text(message),
                    dismissButton: .default(Text("OK"))
                )
            }
        }
    }

    private func clearHistory() {
        do {
            try Database().diagnosticsReports().clear()
            records = []
            onRecordsChanged()
        } catch {
            alertState = .error(error.localizedDescription)
        }
    }
}

private struct SubmittedDiagnosticsRow: View {
    let record: DiagnosticsReportRecord

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(alignment: .firstTextBaseline, spacing: 8) {
                Text(record.reportId)
                    .font(.system(.callout, design: .monospaced))
                    .textSelection(.enabled)

                Spacer()

                Button {
                    UIPasteboard.general.string = record.reportId
                } label: {
                    Image(systemName: "doc.on.doc")
                }
                .buttonStyle(.borderless)
                .accessibilityLabel("Copy Report ID")
            }

            Text(Self.formattedDate(record.submittedAt))
                .font(.footnote)
                .foregroundStyle(.secondary)

            if let description = record.description, !description.isEmpty {
                Text(description)
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                    .lineLimit(2)
            }
        }
        .padding(.vertical, 4)
    }

    private static func formattedDate(_ timestamp: UInt64) -> String {
        let date = Date(timeIntervalSince1970: TimeInterval(timestamp))

        return date.formatted(date: .abbreviated, time: .shortened)
    }
}

#Preview {
    SubmittedDiagnosticsSheet(
        records: [
            DiagnosticsReportRecord(
                reportId: "diag_01JZV1ABCDEF",
                submittedAt: 1_783_529_280,
                description: "App froze after scanning a QR code on the send screen."
            ),
            DiagnosticsReportRecord(
                reportId: "diag_01JZV1XYZ123",
                submittedAt: 1_783_532_400,
                description: nil
            ),
        ],
        onRecordsChanged: {}
    )
}
