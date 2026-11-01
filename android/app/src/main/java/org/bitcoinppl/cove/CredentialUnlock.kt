package org.bitcoinppl.cove

/**
 * A request for the user to open the app lock with a credential before secret content loads
 *
 * Captures the credential unlock count and the main or decoy mode at the time of the request, so
 * only a later PIN, decoy PIN, or biometric unlock in the same mode satisfies it. Create one with
 * [AuthManager.requestCredentialUnlock] and check it with [AuthManager.credentialUnlockStatus]
 */
class CredentialUnlockRequest internal constructor(
    internal val generation: Long,
    internal val isInDecoyMode: Boolean,
)

/** Where a [CredentialUnlockRequest] stands against the current app lock */
enum class CredentialUnlockStatus {
    /** The app lock is still waiting for a credential */
    PENDING,

    /**
     * The user opened the app lock with a credential after the request, in the same mode, and the
     * app has not locked or concealed its content since
     */
    SATISFIED,

    /**
     * The app lock opened without a credential since it last locked, such as the grace-period unlock
     * after a brief app switch
     */
    UNLOCKED_WITHOUT_CREDENTIAL,

    /** The app switched between main and decoy mode, so the request no longer applies */
    SUPERSEDED,
}
