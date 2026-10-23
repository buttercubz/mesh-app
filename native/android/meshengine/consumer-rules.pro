# Consumer R8 rules of :meshengine (applied to the app).
# JNA reflects over Structure fields, Callback methods and Library interfaces.
-dontwarn java.awt.**
-keep class com.sun.jna.** { *; }
-keepclassmembers class * extends com.sun.jna.** { public *; }
# UniFFI bindings: JNA registers the native methods of these classes by name.
-keep class com.convlink.voice.core.** { *; }
# JNI entry points of the Oboe shim (meshaudio.cpp) call back into OboeBridge.
-keep class com.convlink.voice.engine.audio.OboeBridge { *; }
