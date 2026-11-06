package app.lumentube.preview;

import java.io.*;
import java.net.*;
import java.nio.charset.StandardCharsets;
import java.util.concurrent.*;
import java.util.concurrent.atomic.AtomicInteger;

/** Instrumentation-only loopback origin. NOT the production Rust gateway. */
final class FixtureServer implements AutoCloseable {
    private static final String PATH = "/" + "a".repeat(64) + "/media";
    private final byte[] video;
    private final ServerSocket listener;
    private final ExecutorService workers = Executors.newCachedThreadPool();
    final AtomicInteger requests = new AtomicInteger();
    FixtureServer(InputStream asset) throws IOException {
        ByteArrayOutputStream bytes = new ByteArrayOutputStream();
        try (asset) {
            byte[] buffer = new byte[8192]; int count;
            while ((count = asset.read(buffer)) != -1) bytes.write(buffer, 0, count);
        }
        video = bytes.toByteArray();
        listener = new ServerSocket(0, 8, InetAddress.getByName("127.0.0.1"));
        workers.execute(() -> {
            while (!listener.isClosed()) {
                try {
                    Socket socket = listener.accept();
                    workers.execute(() -> serve(socket));
                } catch (IOException ignored) { break; }
            }
        });
    }
    String url() { return "http://127.0.0.1:" + listener.getLocalPort() + PATH; }
    private void serve(Socket socket) {
        try (socket) {
            socket.setSoTimeout(5000);
            BufferedReader reader = new BufferedReader(new InputStreamReader(socket.getInputStream(), StandardCharsets.US_ASCII));
            String first = reader.readLine();
            if (first == null || !first.startsWith("GET " + PATH + " HTTP/1.")) return;
            long start = 0, end = video.length - 1; boolean ranged = false;
            int headerSize = first.length(); String line;
            while ((line = reader.readLine()) != null && !line.isEmpty()) {
                headerSize += line.length(); if (headerSize > 8192) return;
                if (line.regionMatches(true, 0, "Range: bytes=", 0, 13)) {
                    String[] bounds = line.substring(13).trim().split("-", -1);
                    if (bounds.length != 2) return;
                    start = Long.parseLong(bounds[0]);
                    if (!bounds[1].isEmpty()) end = Math.min(Long.parseLong(bounds[1]), end);
                    ranged = true;
                }
            }
            if (start < 0 || end < start || start >= video.length) return;
            requests.incrementAndGet();
            String headers = "HTTP/1.1 " + (ranged ? "206 Partial Content" : "200 OK")
                + "\r\nContent-Type: video/mp4\r\nAccept-Ranges: bytes\r\nConnection: close\r\nContent-Length: " + (end - start + 1)
                + (ranged ? "\r\nContent-Range: bytes " + start + "-" + end + "/" + video.length : "") + "\r\n\r\n";
            OutputStream output = socket.getOutputStream();
            output.write(headers.getBytes(StandardCharsets.US_ASCII));
            output.write(video, (int) start, (int) (end - start + 1));
            output.flush();
        } catch (IOException | RuntimeException ignored) { /* Cancellation is normal. */ }
    }
    @Override public void close() throws IOException {
        listener.close(); workers.shutdownNow();
    }
}
