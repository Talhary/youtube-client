package dev.streampipe.app;

/** The StreamPipe core (Rust, libstreampipe.so). */
final class Native {
    interface Listener {
        /** One core event as JSON. Called on a core thread. */
        void onCoreEvent(String json);
    }

    static volatile Listener listener;

    static {
        System.loadLibrary("streampipe");
    }

    private Native() {}

    /** Start the core. Returns null, or why it could not start. */
    static native String start(String filesDir, String cacheDir, String downloadsDir, String version);

    /** Send one UI command (JSON). Returns null, or why it was not understood. */
    static native String send(String json);

    /** Path of the JSON file holding the ad-blocking rules. */
    static native String rulesFile(String cacheDir);

    /** Called from the core: join a download's picture and sound. Returns null, or why not. */
    @SuppressWarnings("unused")
    static String mux(String video, String audio, String out) {
        return Mux.run(video, audio, out);
    }

    /** Called from the core for every event. */
    @SuppressWarnings("unused")
    static void onEvent(String json) {
        Listener l = listener;
        if (l != null) {
            l.onCoreEvent(json);
        }
    }
}
