package app.lumentube.preview;

import android.app.NativeActivity;
import android.content.Intent;
import android.os.Bundle;

/** NativeActivity hosts Slint; Media3 uses a separate, non-exported Activity. */
public final class MainActivity extends NativeActivity {
    static { System.loadLibrary("lumen_android"); }
    private native void nativeSetActivity();
    private native void nativeReleaseActivity();
    static native void nativeCloseSession(long sessionId);

    @Override public void onCreate(Bundle state) {
        nativeSetActivity();
        super.onCreate(state);
    }

    // Called from a Rust worker. Never expose this method through an exported intent.
    public void openPlayer(String url, long sessionId) {
        runOnUiThread(() -> {
            if (isFinishing() || isDestroyed()) {
                nativeCloseSession(sessionId);
                return;
            }
            try {
                Intent intent = new Intent(this, PlayerActivity.class);
                intent.putExtra("url", url);
                intent.putExtra("sessionId", sessionId);
                startActivity(intent);
            } catch (RuntimeException error) {
                nativeCloseSession(sessionId);
                android.widget.Toast.makeText(this, "Cannot open player", android.widget.Toast.LENGTH_LONG).show();
            }
        });
    }

    @Override public void onDestroy() {
        nativeReleaseActivity();
        super.onDestroy();
    }
}
