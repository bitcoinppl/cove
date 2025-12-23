# JNA - required for UniFFI native library loading
-keep class com.sun.jna.** { *; }
-keep class * implements com.sun.jna.Callback { *; }
-keep class * implements com.sun.jna.Structure { *; }
-keep class * implements com.sun.jna.Library { *; }
-keepclassmembers class * extends com.sun.jna.Structure {
    public *;
}

# UniFFI generated code - keep all FFI bindings
-keep class org.bitcoinppl.cove_core.** { *; }
-keep class uniffi.** { *; }

# Keep native methods
-keepclasseswithmembernames class * {
    native <methods>;
}

# Keep Kotlin coroutines (used by UniFFI async)
-keepclassmembernames class kotlinx.** {
    volatile <fields>;
}

# Preserve line numbers for debugging crash reports
-keepattributes SourceFile,LineNumberTable
-renamesourcefileattribute SourceFile
