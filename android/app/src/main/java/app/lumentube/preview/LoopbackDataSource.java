package app.lumentube.preview;

import android.net.Uri;
import androidx.media3.common.C;
import androidx.media3.datasource.BaseDataSource;
import androidx.media3.datasource.DataSource;
import androidx.media3.datasource.DataSpec;
import java.io.EOFException;
import java.io.IOException;
import java.io.InputStream;
import java.net.HttpURLConnection;
import java.net.Proxy;

/** HTTP transport to one exact Rust capability endpoint, never a remote CDN. */
@androidx.annotation.OptIn(markerClass = androidx.media3.common.util.UnstableApi.class)
final class LoopbackDataSource extends BaseDataSource {
    static final class Factory implements DataSource.Factory {
        private final String source;
        Factory(String source) { LoopbackPolicy.validate(source); this.source = source; }
        @Override public DataSource createDataSource() { return new LoopbackDataSource(source); }
    }
    private final String source;
    private HttpURLConnection connection;
    private InputStream input;
    private long remaining;
    private boolean opened;
    LoopbackDataSource(String source) { super(true); this.source = source; }

    @Override public long open(DataSpec spec) throws IOException {
        transferInitializing(spec);
        if (!source.equals(spec.uri.toString()) || spec.httpMethod != DataSpec.HTTP_METHOD_GET
                || spec.httpBody != null || spec.position < 0 || (spec.length != C.LENGTH_UNSET && spec.length <= 0)) {
            throw new IOException("invalid local media request");
        }
        try {
            connection = (HttpURLConnection) LoopbackPolicy.validate(source).toURL().openConnection(Proxy.NO_PROXY);
            connection.setInstanceFollowRedirects(false);
            connection.setConnectTimeout(8000);
            connection.setReadTimeout(20000);
            connection.setRequestProperty("Accept-Encoding", "identity");
            boolean ranged = spec.position != 0 || spec.length != C.LENGTH_UNSET;
            if (ranged) {
                String end = spec.length == C.LENGTH_UNSET ? "" : Long.toString(Math.addExact(spec.position, spec.length - 1));
                connection.setRequestProperty("Range", "bytes=" + spec.position + "-" + end);
            }
            int status = connection.getResponseCode();
            if (status != (ranged ? 206 : 200)) throw new IOException("local gateway rejected request");
            String mime = connection.getContentType();
            if (mime == null || !mime.split(";", 2)[0].equals("video/mp4")) throw new IOException("local response is not MP4");
            remaining = connection.getContentLengthLong();
            if (remaining <= 0) throw new IOException("local response length missing");
            input = connection.getInputStream();
            opened = true;
            transferStarted(spec);
            return remaining;
        } catch (RuntimeException | IOException error) {
            close();
            // Never pass a URL-bearing HttpURLConnection exception to ExoPlayer logs.
            throw new IOException("local media open failed");
        }
    }

    @Override public int read(byte[] buffer, int offset, int length) throws IOException {
        if (length == 0) return 0;
        if (remaining == 0) return C.RESULT_END_OF_INPUT;
        if (input == null) throw new IOException("local media is closed");
        int count;
        try { count = input.read(buffer, offset, (int) Math.min(length, remaining)); }
        catch (IOException error) { throw new IOException("local media transfer interrupted"); }
        if (count < 0) throw new EOFException("local media ended early");
        remaining -= count;
        bytesTransferred(count);
        return count;
    }
    @Override public Uri getUri() { return opened ? Uri.parse(source) : null; }
    @Override public void close() {
        try { if (input != null) input.close(); } catch (IOException ignored) { }
        input = null;
        if (connection != null) connection.disconnect();
        connection = null;
        if (opened) { opened = false; transferEnded(); }
    }
}
