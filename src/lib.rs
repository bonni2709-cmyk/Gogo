//! VOXEL KNIGHT — тёмный 2D-платформер (метроидвания) на Rust + Macroquad.
//!
//! Модули:
//! * [`physics`] — AABB-физика с разделением осей, гравитация, рейкасты;
//! * [`player`] — контроллер с Coyote Time, Jump Buffer, Wall Slide, Dash,
//!   здоровье и i-frames;
//! * [`ecs`] — лёгкий ECS: сущности, поколения, хранилища компонентов;
//! * [`enemies`] — ИИ врагов на FSM (слизень, летучая мышь, скелет-мечник);
//! * [`game`] — главный цикл (фиксированный шаг 1/60), демо-уровень, камера,
//!   параллакс и HUD.
//!
//! Следующие этапы (по плану проекта): боёвка игрока, генерация уровней,
//! частицы, тач-интерфейс и сборка релиза.

pub mod ecs;
pub mod enemies;
pub mod game;
pub mod particles;
pub mod physics;
pub mod player;

use macroquad::prelude::Conf;

/// Точка входа игры: настольное окно и Android-активность используют её
/// одинаково — конфиг окна задаётся здесь, цикл живёт в [`game::run`].
pub fn run() {
    let conf = Conf {
        window_title: "VOXEL KNIGHT".to_string(),
        window_width: 1280,
        window_height: 720,
        high_dpi: true,
        ..Default::default()
    };
    macroquad::Window::from_config(conf, game::run());
}

/// Android-вход.
///
/// Java-активность miniquad (`quad_native.QuadNative` → `MainActivity`)
/// вызывает экспортируемый символ `quad_main` из нашего `.so`. Без него
/// приложение не стартует на устройстве; на десктопе символ просто
/// экспортируется и не используется.
mod android_entry {
    #[no_mangle]
    pub extern "C" fn quad_main() {
        crate::run();
    }
}
