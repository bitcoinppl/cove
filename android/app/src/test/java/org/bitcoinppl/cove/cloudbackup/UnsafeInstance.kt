package org.bitcoinppl.cove.cloudbackup

/**
 * Allocates an instance without running its constructor
 *
 * JVM unit tests run against the stub android.jar, where framework constructors throw, so tests
 * that only need an identity for an activity or intent sender allocate one this way
 */
internal inline fun <reified T> unsafeInstance(): T {
    val field = sun.misc.Unsafe::class.java.getDeclaredField("theUnsafe")
    field.isAccessible = true
    val unsafe = field.get(null) as sun.misc.Unsafe
    return unsafe.allocateInstance(T::class.java) as T
}
