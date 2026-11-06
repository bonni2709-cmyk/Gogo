//! JNI bridge. Java owns Activity/ExoPlayer lifecycle; Rust owns remote networking.
use anyhow::{bail, Result};
use jni::{objects::{GlobalRef, JObject, JValue}, sys::jlong, JNIEnv, JavaVM};
use lumen_youtube::PlaybackSession;
use std::sync::{atomic::{AtomicI64, Ordering}, Mutex, OnceLock};

struct Activity { vm: JavaVM, object: GlobalRef }
static ACTIVITY: OnceLock<Mutex<Option<Activity>>> = OnceLock::new();
static SESSION: Mutex<Option<(jlong, PlaybackSession)>> = Mutex::new(None);
static NEXT: AtomicI64 = AtomicI64::new(1);

pub fn available() -> bool {
    ACTIVITY.get().and_then(|slot| slot.lock().ok()).is_some_and(|slot| slot.is_some())
}

pub fn open(session: PlaybackSession) -> Result<()> {
    let activity = ACTIVITY.get().ok_or_else(|| anyhow::anyhow!("Android Java host is missing; use the Gradle APK"))?
        .lock().map_err(|_| anyhow::anyhow!("Android host lock failed"))?;
    let activity = activity.as_ref().ok_or_else(|| anyhow::anyhow!("Android host is stopped"))?;
    let id = NEXT.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
        .map_err(|_| anyhow::anyhow!("player session counter exhausted"))?;
    let url = session.local_url();
    {
        let mut active = SESSION.lock().map_err(|_| anyhow::anyhow!("player session lock failed"))?;
        if active.is_some() { bail!("a player session is already open"); }
        *active = Some((id, session));
    }
    let result = (|| -> jni::errors::Result<()> {
        let mut env = activity.vm.attach_current_thread()?;
        let source = JObject::from(env.new_string(url)?);
        let called = env.call_method(activity.object.as_obj(), "openPlayer", "(Ljava/lang/String;J)V", &[JValue::Object(&source), JValue::Long(id)]);
        if called.is_err() { let _ = env.exception_clear(); }
        called?;
        Ok(())
    })();
    if result.is_err() {
        close(id);
        bail!("could not schedule Android player Activity");
    }
    Ok(())
}

fn close(id: jlong) {
    let removed = SESSION.lock().ok().and_then(|mut slot| {
        if slot.as_ref().is_some_and(|(active, _)| *active == id) { slot.take() } else { None }
    });
    drop(removed);
}

#[no_mangle]
pub extern "system" fn Java_app_lumentube_preview_MainActivity_nativeSetActivity(env: JNIEnv, this: JObject) {
    let host = (|| -> jni::errors::Result<Activity> {
        Ok(Activity { vm: env.get_java_vm()?, object: env.new_global_ref(this)? })
    })();
    if let Ok(host) = host {
        if let Ok(mut slot) = ACTIVITY.get_or_init(|| Mutex::new(None)).lock() { *slot = Some(host); }
    }
}

#[no_mangle]
pub extern "system" fn Java_app_lumentube_preview_MainActivity_nativeReleaseActivity(env: JNIEnv, this: JObject) {
    let cleared = ACTIVITY.get().and_then(|slot| slot.lock().ok()).is_some_and(|mut slot| {
        let matches = slot.as_ref().is_some_and(|host| env.is_same_object(host.object.as_obj(), &this).unwrap_or(false));
        if matches { *slot = None; }
        matches
    });
    if cleared {
        let session = SESSION.lock().ok().and_then(|mut slot| slot.take());
        drop(session);
    }
}

#[no_mangle]
pub extern "system" fn Java_app_lumentube_preview_MainActivity_nativeCloseSession(_env: JNIEnv, _class: JObject, id: jlong) {
    close(id);
}
