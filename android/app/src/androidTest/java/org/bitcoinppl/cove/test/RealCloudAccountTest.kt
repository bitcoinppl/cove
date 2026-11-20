package org.bitcoinppl.cove.test

/**
 * Marks a test that signs in to the real Google account holding the frozen cloud backup
 *
 * Those tests run only through `just android-cloud-backup-e2e`, which reads the account from this
 * Mac's Keychain. The CI recipe `android-instrumented-ci` excludes them by this annotation, and
 * any other run skips them because the account arguments are missing
 */
@Target(AnnotationTarget.CLASS)
@Retention(AnnotationRetention.RUNTIME)
annotation class RealCloudAccountTest
