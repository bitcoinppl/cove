package org.bitcoinppl.cove.test

/**
 * Excludes a test from the CI emulator run while it fails or flakes there for a known reason
 *
 * Local `connected*AndroidTest` runs still include it. Remove the annotation once the
 * linked fix lands and the test passes repeatedly on the CI emulator
 *
 * CI's `quarantine` job reads every use from the source and fails once one is past [expires], so
 * a quarantine is either fixed or looked at again; write all three arguments by name as string
 * literals so it can read them. See the quarantine rules in docs/testing.md
 */
@Target(AnnotationTarget.CLASS, AnnotationTarget.FUNCTION)
@Retention(AnnotationRetention.RUNTIME)
annotation class QuarantinedInCi(
    val reason: String,
    /** The bitcoinppl/cove issue that tracks bringing the test back, as its full url */
    val issue: String,
    /** The last day the quarantine holds, as YYYY-MM-DD, at most 60 days after it's set */
    val expires: String,
)
