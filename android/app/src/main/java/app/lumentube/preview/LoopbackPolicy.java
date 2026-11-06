package app.lumentube.preview;

import java.net.URI;

/** Pure-Java policy, also exercised by JVM unit tests. */
final class LoopbackPolicy {
    static URI validate(String source) {
        if (source == null) throw new IllegalArgumentException("missing loopback URL");
        URI uri = URI.create(source);
        if (!"http".equals(uri.getScheme()) || !"127.0.0.1".equals(uri.getHost())
                || uri.getPort() <= 0 || uri.getPort() > 65535 || uri.getRawUserInfo() != null
                || uri.getRawQuery() != null || uri.getRawFragment() != null
                || uri.getRawPath() == null || !uri.getRawPath().matches("/[0-9a-f]{64}/media")) {
            throw new IllegalArgumentException("not an approved loopback media URL");
        }
        return uri;
    }
}
