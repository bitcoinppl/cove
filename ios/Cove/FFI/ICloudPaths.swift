import CoveCore
import Foundation

struct ICloudPaths {
    private static let defaultDataSubdirectory = "Data"
    private static let defaultNamespacesSubdirectory = csppNamespacesSubdirectory()
    private static let defaultWalletsSubdirectory = csppWalletsDirectory()

    let containerURL: URL

    private let dataSubdirectory: String
    private let namespacesSubdirectory: String
    private let walletsSubdirectory: String

    init(
        containerURL: URL,
        dataSubdirectory: String = Self.defaultDataSubdirectory,
        namespacesSubdirectory: String = Self.defaultNamespacesSubdirectory,
        walletsSubdirectory: String = Self.defaultWalletsSubdirectory
    ) {
        self.containerURL = containerURL
        self.dataSubdirectory = dataSubdirectory
        self.namespacesSubdirectory = namespacesSubdirectory
        self.walletsSubdirectory = walletsSubdirectory
    }

    func dataDirectoryURL() -> URL {
        containerURL.appendingPathComponent(dataSubdirectory, isDirectory: true)
    }

    /// Root directory for all namespaces: Data/cspp-namespaces/
    func namespacesRootURL() -> URL {
        dataDirectoryURL()
            .appendingPathComponent(namespacesSubdirectory, isDirectory: true)
    }

    func validateNamespace(_ namespace: String) throws -> String {
        try Self.validateNamespace(namespace)
    }

    /// Directory for a specific namespace: Data/cspp-namespaces/{namespace}/
    func namespaceDirectoryURL(namespace: String) throws -> URL {
        let namespace = try validateNamespace(namespace)
        return namespacesRootURL()
            .appendingPathComponent(namespace, isDirectory: true)
    }

    func walletsDirectoryURL(namespace: String) throws -> URL {
        try namespaceDirectoryURL(namespace: namespace)
            .appendingPathComponent(walletsSubdirectory, isDirectory: true)
    }

    func walletLocation(filename: String) -> String {
        "\(walletsSubdirectory)/\(filename)"
    }

    func backupFileURL(namespace: String, location: RemoteBackupLocation) throws -> URL {
        let namespaceDirectory = try namespaceDirectoryURL(namespace: namespace)
        return try appendBackupLocation(location, to: namespaceDirectory)
    }

    static func isValidNamespaceDirectory(_ url: URL) -> Bool {
        url.hasDirectoryPath && isValidNamespaceID(url.lastPathComponent)
    }

    static func validateNamespace(_ namespace: String) throws -> String {
        guard isValidNamespaceID(namespace) else {
            throw CloudStorageError.InvalidNamespace("expected 32 lowercase hex characters")
        }

        return namespace
    }

    static func walletLocation(filename: String) -> String {
        "\(defaultWalletsSubdirectory)/\(filename)"
    }

    private func appendBackupLocation(
        _ location: RemoteBackupLocation,
        to namespaceDirectory: URL
    ) throws -> URL {
        let parts = location.relativePath.split(separator: "/").map(String.init)
        guard
            !parts.isEmpty,
            parts.allSatisfy({ !$0.isEmpty && $0 != "." && $0 != ".." })
        else {
            throw CloudStorageError.NotAvailable("invalid backup location")
        }

        var directory = namespaceDirectory
        for folder in parts.dropLast() {
            directory.appendPathComponent(folder, isDirectory: true)
        }

        return directory.appendingPathComponent(parts.last!)
    }

    private static func isValidNamespaceID(_ namespace: String) -> Bool {
        namespace.count == 32 && namespace.utf8.allSatisfy { byte in
            switch byte {
            case 48 ... 57, 97 ... 102:
                true
            default:
                false
            }
        }
    }
}
