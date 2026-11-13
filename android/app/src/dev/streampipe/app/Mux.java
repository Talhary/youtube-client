package dev.streampipe.app;

import android.media.MediaCodec;
import android.media.MediaExtractor;
import android.media.MediaFormat;
import android.media.MediaMuxer;
import android.util.Log;

import java.nio.ByteBuffer;

/**
 * Joins a downloaded picture stream and sound stream into one playable file with Android's
 * own muxer. Nothing is re-encoded: the samples are copied as they are.
 */
final class Mux {
    private static final String TAG = "StreamPipe";

    private Mux() {}

    /** One input file's first track of the wanted kind. */
    private static final class Source {
        final MediaExtractor extractor = new MediaExtractor();
        int outTrack = -1;
        boolean done;

        Source(String path, String kind) throws Exception {
            extractor.setDataSource(path);
            for (int i = 0; i < extractor.getTrackCount(); i++) {
                String mime = extractor.getTrackFormat(i).getString(MediaFormat.KEY_MIME);
                if (mime != null && mime.startsWith(kind)) {
                    extractor.selectTrack(i);
                    format = extractor.getTrackFormat(i);
                    return;
                }
            }
            throw new Exception("no " + kind + " track in the downloaded stream");
        }

        MediaFormat format;
    }

    /**
     * @param video path of the picture stream, or "" for a sound-only file
     * @return null, or why it failed
     */
    static String run(String video, String audio, String out) {
        Source[] sources = null;
        MediaMuxer muxer = null;
        boolean started = false;
        try {
            sources = video.isEmpty()
                    ? new Source[] {new Source(audio, "audio/")}
                    : new Source[] {new Source(video, "video/"), new Source(audio, "audio/")};
            int container = out.endsWith(".webm") || out.endsWith(".opus")
                    ? MediaMuxer.OutputFormat.MUXER_OUTPUT_WEBM : MediaMuxer.OutputFormat.MUXER_OUTPUT_MPEG_4;
            muxer = new MediaMuxer(out, container);
            int largest = 1 << 20;
            for (Source s : sources) {
                s.outTrack = muxer.addTrack(s.format);
                if (s.format.containsKey(MediaFormat.KEY_MAX_INPUT_SIZE)) {
                    largest = Math.max(largest, s.format.getInteger(MediaFormat.KEY_MAX_INPUT_SIZE));
                }
            }
            muxer.start();
            started = true;
            ByteBuffer buffer = ByteBuffer.allocate(Math.max(largest, 4 << 20));
            MediaCodec.BufferInfo info = new MediaCodec.BufferInfo();
            // Always write the track that is furthest behind, so picture and sound stay
            // interleaved in the file.
            while (true) {
                Source next = null;
                for (Source s : sources) {
                    if (!s.done && (next == null || s.extractor.getSampleTime() < next.extractor.getSampleTime())) {
                        next = s;
                    }
                }
                if (next == null) {
                    break;
                }
                int size = next.extractor.readSampleData(buffer, 0);
                if (size < 0) {
                    next.done = true;
                    continue;
                }
                info.offset = 0;
                info.size = size;
                info.presentationTimeUs = next.extractor.getSampleTime();
                info.flags = (next.extractor.getSampleFlags() & MediaExtractor.SAMPLE_FLAG_SYNC) != 0
                        ? MediaCodec.BUFFER_FLAG_KEY_FRAME : 0;
                muxer.writeSampleData(next.outTrack, buffer, info);
                if (!next.extractor.advance()) {
                    next.done = true;
                }
            }
            muxer.stop();
            started = false;
            return null;
        } catch (Throwable e) {
            Log.w(TAG, "mux: " + e);
            new java.io.File(out).delete();
            String why = e.getMessage();
            return "Could not finish the file: " + (why == null ? e.getClass().getSimpleName() : why);
        } finally {
            if (sources != null) {
                for (Source s : sources) {
                    if (s != null) {
                        s.extractor.release();
                    }
                }
            }
            if (muxer != null) {
                try {
                    if (started) {
                        muxer.stop();
                    }
                } catch (Throwable ignored) {
                }
                muxer.release();
            }
        }
    }
}
