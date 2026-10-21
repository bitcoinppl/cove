package org.bitcoinppl.cove.test

/**
 * Excludes a test from the CI emulator run while it fails or flakes there for a known reason
 *
 * Local `connected*AndroidTest` runs still include it. Remove the annotation once the
 * linked fix lands and the test passes repeatedly on the CI emulator
 */
@Target(AnnotationTarget.CLASS, AnnotationTarget.FUNCTION)
@Retention(AnnotationRetention.RUNTIME)
annotation class QuarantinedInCi(
    val reason: String,
)
