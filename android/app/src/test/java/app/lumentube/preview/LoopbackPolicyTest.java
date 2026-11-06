package app.lumentube.preview;

import org.junit.Test;
import static org.junit.Assert.*;

public class LoopbackPolicyTest {
    private static final String PATH = "/" + "a".repeat(64) + "/media";
    @Test public void acceptsOnlyExactLoopbackCapabilityShape() {
        assertEquals("127.0.0.1", LoopbackPolicy.validate("http://127.0.0.1:32123" + PATH).getHost());
        for (String url : new String[] {
                "https://127.0.0.1:32123" + PATH, "http://localhost:32123" + PATH,
                "http://evil.test:32123" + PATH, "http://127.0.0.1" + PATH,
                "http://user@127.0.0.1:32123" + PATH, "http://127.0.0.1:32123" + PATH + "?url=evil",
                "http://127.0.0.1:32123" + PATH + "#fragment", "http://127.0.0.1:32123/short/media" }) {
            assertThrows(IllegalArgumentException.class, () -> LoopbackPolicy.validate(url));
        }
    }
}
