package app.lumentube.preview;

import android.content.Intent;
import androidx.lifecycle.Lifecycle;
import androidx.media3.common.Player;
import androidx.media3.exoplayer.ExoPlayer;
import androidx.test.core.app.ActivityScenario;
import androidx.test.ext.junit.runners.AndroidJUnit4;
import androidx.test.platform.app.InstrumentationRegistry;
import java.lang.reflect.Field;
import java.util.concurrent.atomic.AtomicBoolean;
import org.junit.Test;
import org.junit.runner.RunWith;
import static org.junit.Assert.*;

/** Offline runtime smoke. Does not claim to test YouTube, DPI or the Rust CDN path. */
@RunWith(AndroidJUnit4.class)
@androidx.annotation.OptIn(markerClass = androidx.media3.common.util.UnstableApi.class)
public class PlayerSmokeTest {
    private static ExoPlayer player(PlayerActivity activity) {
        try {
            Field field = PlayerActivity.class.getDeclaredField("player");
            field.setAccessible(true);
            return (ExoPlayer) field.get(activity);
        } catch (ReflectiveOperationException error) { throw new AssertionError(error); }
    }
    private static void awaitReady(ActivityScenario<PlayerActivity> scenario) throws InterruptedException {
        long deadline = android.os.SystemClock.elapsedRealtime() + 20000;
        AtomicBoolean ready = new AtomicBoolean();
        while (android.os.SystemClock.elapsedRealtime() < deadline) {
            scenario.onActivity(activity -> {
                ExoPlayer p = player(activity);
                ready.set(p != null && p.getPlaybackState() == Player.STATE_READY && p.isPlaying());
            });
            if (ready.get()) return;
            Thread.sleep(100);
        }
        fail("Media3 did not reach READY/playing for the local fixture");
    }

    @Test public void nativeHostAndMedia3CanStartSeekStopAndResume() throws Exception {
        var instrumentation = InstrumentationRegistry.getInstrumentation();
        // Loads liblumen_android and calls NativeActivity entry point + nativeSetActivity.
        try (ActivityScenario<MainActivity> host = ActivityScenario.launch(MainActivity.class);
             FixtureServer server = new FixtureServer(instrumentation.getContext().getAssets().open("fixture.mp4"))) {
            host.onActivity(activity -> assertFalse(activity.isFinishing()));
            Thread.sleep(1500); // Give the native event loop time to start/fail.
            host.onActivity(activity -> assertTrue(activity.getWindow().getDecorView().isShown()));
            Intent intent = new Intent(instrumentation.getTargetContext(), PlayerActivity.class);
            intent.putExtra("url", server.url());
            // Deliberately unknown ID exercises the no-op close callback without a fake Rust session.
            intent.putExtra("sessionId", Long.MAX_VALUE);
            try (ActivityScenario<PlayerActivity> scenario = ActivityScenario.launch(intent)) {
                awaitReady(scenario);
                scenario.onActivity(activity -> player(activity).seekTo(1000));
                awaitReady(scenario);
                scenario.onActivity(activity -> assertTrue(player(activity).getCurrentPosition() >= 900));
                scenario.moveToState(Lifecycle.State.CREATED);
                scenario.onActivity(activity -> assertNull("onStop must release ExoPlayer", player(activity)));
                scenario.moveToState(Lifecycle.State.RESUMED);
                awaitReady(scenario);
                assertTrue("fixture was never requested", server.requests.get() > 0);
            } // onDestroy calls nativeCloseSession; a missing JNI symbol fails the test.
            host.onActivity(activity -> assertFalse(activity.isFinishing()));
        }
    }
}
