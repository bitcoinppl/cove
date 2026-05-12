package org.bitcoinppl.cove.flows.SettingsFlow

enum class TorStatus {
    Disabled,
    Bootstrapping,
    Ready,
    Error,
}

enum class TorMode {
    BuiltIn,
    Orbot,
    External,
}

enum class OrbotStatus {
    Checking,
    Detected,
    NotDetected,
}

data class TorUiState(
    val enabled: Boolean = false,
    val mode: TorMode = TorMode.BuiltIn,
    val status: TorStatus = TorStatus.Disabled,
    val progressPercent: Int = 0,
    val currentStep: String = "Disabled",
    val latestLogLine: String = "Tor is off",
    val logLines: List<String> = listOf("Tor is off"),
    val externalHost: String = "127.0.0.1",
    val externalPort: String = "9050",
    val externalValidationError: String? = null,
    val orbotStatus: OrbotStatus = OrbotStatus.Checking,
    val orbotVersion: String? = null,
)
