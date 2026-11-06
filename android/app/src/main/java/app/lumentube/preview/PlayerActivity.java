package app.lumentube.preview;

import android.app.Activity;
import android.net.Uri;
import android.os.Bundle;
import android.view.WindowManager;
import android.widget.Toast;
import androidx.media3.common.MediaItem;
import androidx.media3.common.PlaybackException;
import androidx.media3.common.Player;
import androidx.media3.datasource.DefaultDataSource;
import androidx.media3.exoplayer.ExoPlayer;
import androidx.media3.exoplayer.source.DefaultMediaSourceFactory;
import androidx.media3.ui.PlayerView;

/** All network access is restricted to one Rust-issued loopback capability URL. */
@androidx.annotation.OptIn(markerClass = androidx.media3.common.util.UnstableApi.class)
public final class PlayerActivity extends Activity {
    private ExoPlayer player;
    private PlayerView view;
    private long sessionId;
    private Uri source;
    private long position;
    private boolean playWhenReady = true;
    private boolean closed;

    @Override public void onCreate(Bundle state) {
        super.onCreate(state);
        sessionId = getIntent().getLongExtra("sessionId", 0);
        try {
            source = Uri.parse(getIntent().getStringExtra("url"));
            LoopbackPolicy.validate(source.toString());
            if (sessionId <= 0) throw new IllegalArgumentException("session ID missing");
        } catch (RuntimeException error) {
            closeSession(); finish(); return;
        }
        if (state != null) {
            position = state.getLong("position", 0);
            playWhenReady = state.getBoolean("playWhenReady", true);
        }
        getWindow().addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
        view = new PlayerView(this);
        view.setUseController(true);
        setContentView(view);
    }

    @Override public void onStart() {
        super.onStart();
        if (source == null || closed || player != null) return;
        try {
            // Do not give Media3 a DefaultHttpDataSource: redirects/remote URLs are disallowed.
            LoopbackDataSource.Factory http = new LoopbackDataSource.Factory(source.toString());
            DefaultDataSource.Factory data = new DefaultDataSource.Factory(this, http);
            player = new ExoPlayer.Builder(this)
                .setMediaSourceFactory(new DefaultMediaSourceFactory(data)).build();
            player.addListener(new Player.Listener() {
                @Override public void onPlayerError(PlaybackException error) {
                    // Do not print error.getMessage(): it can contain the capability URL.
                    Toast.makeText(PlayerActivity.this, "Playback failed. Reopen the video from search.", Toast.LENGTH_LONG).show();
                    closeSession(); finish();
                }
            });
            view.setPlayer(player);
            player.setMediaItem(MediaItem.fromUri(source));
            player.seekTo(position);
            player.setPlayWhenReady(playWhenReady);
            player.prepare();
        } catch (RuntimeException error) {
            closeSession(); finish();
        }
    }

    @Override public void onSaveInstanceState(Bundle state) {
        state.putLong("position", player == null ? position : player.getCurrentPosition());
        state.putBoolean("playWhenReady", player == null ? playWhenReady : player.getPlayWhenReady());
        super.onSaveInstanceState(state);
    }

    @Override public void onStop() {
        if (player != null) {
            position = player.getCurrentPosition();
            playWhenReady = player.getPlayWhenReady();
            view.setPlayer(null);
            player.release();
            player = null;
        }
        super.onStop();
    }

    private void closeSession() {
        if (!closed && sessionId > 0) { closed = true; MainActivity.nativeCloseSession(sessionId); }
    }

    @Override public void onDestroy() {
        if (!isChangingConfigurations()) closeSession();
        super.onDestroy();
    }
}
