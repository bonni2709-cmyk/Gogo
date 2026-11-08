//! Некрорыцарь-босс: длинные телеграфы, тяжёлый выпад с лязгом стали
//! и ярость ниже половины HP (фазы укорачиваются, погоня ускоряется).
//!
//! FSM повторяет скелета (Patrol → Chase → Windup → Strike → Recover),
//! но с босскими таймингами [`timing::BOSS_*`] и множителем ярости.
//! Полоса HP рисуется над головой — игрок всегда видит остаток боя.

use macroquad::color::Color;
use macroquad::shapes::{draw_rectangle, draw_rectangle_lines};

use super::{palette, timing, *};
use crate::physics::{move_toward, World};

/// Дистанция, с которой босс начинает выпад (дальше обычного скелета).
const ATTACK_RANGE: f32 = 62.0;
/// Скорость выпада в активной фазе.
const LUNGE_SPEED: f32 = 245.0;
/// Остановка тела на замахе.
const WINDUP_STOP: f32 = 2800.0;
/// Длительность активной фазы выпада.
const STRIKE_TIME: f32 = 0.16;

/// Шаг FSM некрорыцаря.
pub(super) fn update(
    brain: &mut EnemyBrain,
    t: &mut Transform,
    vel: &mut Velocity,
    world: &World,
    target: &TargetInfo,
    dt: f32,
) {
    // Множители ярости: короче замахи, злее погоня.
    let rage = if brain.enraged { 1.0 } else { 0.0 };
    let windup_end = timing::BOSS_WINDUP * (1.0 - 0.38 * rage);
    let recover_end = timing::BOSS_RECOVER * (1.0 - 0.30 * rage);
    let cooldown = timing::BOSS_COOLDOWN * (1.0 - 0.35 * rage);
    let chase_mul = 1.0 + 0.35 * rage;

    match brain.state {
        EnemyState::Hurt => {
            if brain.state_timer >= timing::HURT {
                brain.transition(if brain.aggro {
                    EnemyState::Chase
                } else {
                    EnemyState::Patrol
                });
            }
        }
        EnemyState::Patrol => {
            if brain.on_ground
                && (ledge_ahead(world, t, brain.dir) || wall_ahead(world, t, brain.dir))
            {
                brain.dir = -brain.dir;
            }
            vel.0.x = brain.dir * params(brain.kind).patrol_speed;
            if brain.aggro {
                brain.transition(EnemyState::Chase);
            }
        }
        EnemyState::Chase => {
            let toward = (target.center.x - t.center().x).signum();
            if toward.abs() > f32::EPSILON {
                brain.dir = toward;
            }
            // У обрыва стоим — босс не сходит с площадки за целью.
            let ledge = brain.on_ground && ledge_ahead(world, t, brain.dir);
            vel.0.x = if ledge {
                0.0
            } else {
                brain.dir * params(brain.kind).chase_speed * chase_mul
            };

            if !brain.aggro {
                brain.transition(EnemyState::Patrol);
            } else if target.los
                && target.dist <= ATTACK_RANGE
                && brain.cooldown <= 0.0
                && brain.on_ground
            {
                brain.transition(EnemyState::Windup);
                vel.0.x = 0.0;
            }
        }
        EnemyState::Windup => {
            // Длинный телеграф: меч поднят, тело замирает.
            vel.0.x = move_toward(vel.0.x, 0.0, WINDUP_STOP * dt);
            if brain.state_timer >= windup_end {
                brain.transition(EnemyState::Strike);
                vel.0.x = brain.dir * LUNGE_SPEED * (1.0 + 0.25 * rage);
            }
        }
        EnemyState::Strike => {
            // Тяжёлый выпад: удар идёт по касанию тела.
            if brain.state_timer >= STRIKE_TIME {
                brain.transition(EnemyState::Recover);
                brain.cooldown = cooldown;
                vel.0.x = 0.0;
            }
        }
        EnemyState::Recover => {
            vel.0.x = move_toward(vel.0.x, 0.0, 1600.0 * dt);
            if brain.state_timer >= recover_end {
                brain.transition(if brain.aggro {
                    EnemyState::Chase
                } else {
                    EnemyState::Patrol
                });
            }
        }
        // Лётные/чужие состояния боссу не приходят.
        _ => brain.transition(EnemyState::Patrol),
    }
}

/// Отрисовка некрорыцаря: рогатый шлем, пластины, плащ, клеймор и полоса HP.
pub(super) fn draw(t: &Transform, brain: &EnemyBrain, health: &Health, time: f32) {
    let x = t.pos.x;
    let y = t.pos.y;
    let w = t.size.x;
    let h = t.size.y;
    let facing_right = t.facing >= 0.0;

    let plate = flash_color(palette::BOSS_PLATE, health.hit_flash);
    let plate_dark = flash_color(palette::BOSS_PLATE_DARK, health.hit_flash);
    let cape = flash_color(palette::BOSS_CAPE, health.hit_flash);
    let bone = flash_color(palette::BONE, health.hit_flash);
    let bone_dark = flash_color(palette::BONE_DARK, health.hit_flash);
    let steel = flash_color(palette::STEEL, health.hit_flash);

    let fx = |lx: f32, ew: f32| -> f32 {
        if facing_right {
            x + lx
        } else {
            x + w - lx - ew
        }
    };

    // Тень.
    draw_rectangle(x + 4.0, y + h - 1.0, w - 8.0, 4.0, palette::SHADOW);

    // --- плащ: рваные полосы, колышутся; в ярости треплет сильнее ---
    let wind = if brain.enraged { 2.2 } else { 1.3 };
    let sway = (time * 4.0).sin() * wind;
    draw_rectangle(fx(2.0 + sway, 7.0), y + 16.0, 7.0, h - 20.0, cape);
    draw_rectangle(fx(8.0 + sway * 0.6, 6.0), y + 18.0, 6.0, h - 24.0, palette::BOSS_PLATE_DARK);
    draw_rectangle(fx(13.0 - sway * 0.4, 5.0), y + 17.0, 5.0, h - 21.0, cape);
    // Рваные края плаща.
    for (hx, hh) in [(2.0, 6.0), (8.0, 9.0), (13.0, 5.0)] {
        draw_rectangle(fx(hx, 5.0), y + h - 8.0 - hh, 5.0, hh, cape);
    }

    // --- ноги: латные поножи с заклёпками ---
    let walking = matches!(brain.state, EnemyState::Patrol | EnemyState::Chase);
    let swing = if walking { (time * 5.5).sin() * 3.5 } else { 0.0 };
    let (leg1, leg2) = if brain.state == EnemyState::Windup {
        (8.0, 22.0)
    } else {
        (7.0 + swing, 21.0 - swing)
    };
    draw_rectangle(fx(leg1, 9.0) - 1.0, y + h - 24.0, 11.0, 25.0, palette::OUTLINE);
    draw_rectangle(fx(leg2, 9.0) - 1.0, y + h - 24.0, 11.0, 25.0, palette::OUTLINE);
    draw_rectangle(fx(leg1, 9.0), y + h - 23.0, 9.0, 23.0, plate_dark);
    draw_rectangle(fx(leg2, 9.0), y + h - 23.0, 9.0, 23.0, plate_dark);
    // Светлая кромка голени.
    draw_rectangle(fx(leg1 + 1.0, 7.0), y + h - 16.0, 7.0, 2.0, plate);
    draw_rectangle(fx(leg2 + 1.0, 7.0), y + h - 16.0, 7.0, 2.0, plate);
    // Заклёпки и стальные сапоги.
    draw_rectangle(fx(leg1 + 3.0, 3.0), y + h - 21.0, 3.0, 3.0, palette::GOLD);
    draw_rectangle(fx(leg2 + 3.0, 3.0), y + h - 21.0, 3.0, 3.0, palette::GOLD);
    draw_rectangle(fx(leg1, 11.0), y + h - 6.0, 11.0, 6.0, palette::STEEL_DARK);
    draw_rectangle(fx(leg2, 11.0), y + h - 6.0, 11.0, 6.0, palette::STEEL_DARK);

    // --- корпус: нагрудник с рёбрами-гребнем ---
    let tl = 6.0;
    draw_rectangle(fx(tl - 1.0, w - 10.0), y + 14.0, w - 10.0, 24.0, palette::OUTLINE);
    draw_rectangle(fx(tl, w - 12.0), y + 15.0, w - 12.0, 22.0, plate);
    // Свет/тень по краям пластины.
    draw_rectangle(fx(tl, w - 12.0), y + 15.0, w - 12.0, 2.0, Color::new(0.47, 0.46, 0.50, 1.0));
    draw_rectangle(fx(tl, w - 12.0), y + 35.0, w - 12.0, 2.0, plate_dark);
    // Гребень и сегменты.
    draw_rectangle(fx(18.0, 4.0), y + 16.0, 4.0, 20.0, plate_dark);
    draw_rectangle(fx(tl + 2.0, w - 16.0), y + 22.0, w - 16.0, 2.0, plate_dark);
    draw_rectangle(fx(tl + 2.0, w - 16.0), y + 28.0, w - 16.0, 2.0, plate_dark);
    // Ржавые заклёпки по углам.
    for (rx, ry) in [(tl + 2.0, 17.0), (w - 10.0, 17.0), (tl + 2.0, 32.0), (w - 10.0, 32.0)] {
        draw_rectangle(fx(rx, 3.0), y + ry, 3.0, 3.0, palette::RUST);
    }

    // --- пояс ---
    draw_rectangle(fx(tl, w - 12.0), y + 36.0, w - 12.0, 4.0, palette::OUTLINE);
    draw_rectangle(fx(18.0, 5.0), y + 36.0, 5.0, 4.0, palette::GOLD);

    // --- наплечники: два яруса each сторона ---
    draw_rectangle(fx(1.0, 12.0) - 1.0, y + 12.0, 14.0, 10.0, palette::OUTLINE);
    draw_rectangle(fx(1.0, 12.0), y + 13.0, 12.0, 8.0, plate);
    draw_rectangle(fx(1.0, 12.0), y + 13.0, 12.0, 2.0, Color::new(0.47, 0.46, 0.50, 1.0));
    draw_rectangle(fx(2.0, 10.0), y + 21.0, 10.0, 5.0, plate_dark);
    draw_rectangle(fx(w - 13.0, 12.0) - 1.0, y + 12.0, 14.0, 10.0, palette::OUTLINE);
    draw_rectangle(fx(w - 13.0, 12.0), y + 13.0, 12.0, 8.0, plate);
    draw_rectangle(fx(w - 13.0, 12.0), y + 13.0, 12.0, 2.0, Color::new(0.47, 0.46, 0.50, 1.0));
    draw_rectangle(fx(w - 12.0, 10.0), y + 21.0, 10.0, 5.0, plate_dark);
    // Заклёпки на плечах.
    draw_rectangle(fx(4.0, 3.0), y + 15.0, 3.0, 3.0, palette::GOLD);
    draw_rectangle(fx(w - 7.0, 3.0), y + 15.0, 3.0, 3.0, palette::GOLD);

    // --- руки: передняя держит клеймор ---
    draw_rectangle(fx(tl - 2.0, 7.0), y + 24.0, 7.0, 12.0, plate_dark);
    draw_rectangle(fx(w - 12.0, 7.0), y + 23.0, 7.0, 13.0, plate);
    draw_rectangle(fx(w - 11.0, 5.0), y + 33.0, 5.0, 6.0, palette::STEEL_DARK); // кисть

    // --- шлем: рога, черепное лицо, тлеющие глаза ---
    let hl = 9.0;
    draw_rectangle(fx(hl, 22.0) - 1.0, y - 2.0, 24.0, 18.0, palette::OUTLINE);
    draw_rectangle(fx(hl, 22.0), y - 1.0, 22.0, 16.0, plate);
    // Свод шлема.
    draw_rectangle(fx(hl, 22.0), y - 1.0, 22.0, 3.0, Color::new(0.47, 0.46, 0.50, 1.0));
    // Рога: костяные, вверх-внешне.
    let horn = bone;
    draw_rectangle(fx(hl - 4.0, 6.0), y - 6.0, 6.0, 8.0, horn);
    draw_rectangle(fx(hl - 6.0, 5.0), y - 9.0, 5.0, 5.0, bone_dark);
    draw_rectangle(fx(w - hl - 2.0, 6.0), y - 6.0, 6.0, 8.0, horn);
    draw_rectangle(fx(w - hl - 1.0, 5.0), y - 9.0, 5.0, 5.0, bone_dark);
    // Лицо-череп в шлеме.
    draw_rectangle(fx(hl + 3.0, 16.0), y + 5.0, 16.0, 10.0, bone);
    draw_rectangle(fx(hl + 3.0, 16.0), y + 5.0, 16.0, 2.0, bone_dark);
    // Назальная перемычка.
    draw_rectangle(fx(hl + 10.0, 3.0), y + 6.0, 3.0, 6.0, plate_dark);
    // Глаза: тлеющие угольки; в ярости — ярче и больше.
    let eye_col = if brain.enraged {
        Color::new(1.0, 0.55, 0.22, 1.0)
    } else {
        palette::BOSS_EYE
    };
    let eye_w = if brain.enraged { 5.0 } else { 4.0 };
    draw_rectangle(fx(hl + 4.0, eye_w), y + 7.0, eye_w, 3.0, eye_col);
    draw_rectangle(fx(hl + 14.0, eye_w), y + 7.0, eye_w, 3.0, eye_col);
    // Свечение в ярости.
    if brain.enraged {
        draw_rectangle(fx(hl + 3.0, eye_w + 2.0), y + 6.0, eye_w + 2.0, 5.0, Color::new(1.0, 0.45, 0.15, 0.30));
        draw_rectangle(fx(hl + 13.0, eye_w + 2.0), y + 6.0, eye_w + 2.0, 5.0, Color::new(1.0, 0.45, 0.15, 0.30));
    }
    // Челюсть с зубами.
    draw_rectangle(fx(hl + 4.0, 16.0), y + 12.0, 16.0, 4.0, bone);
    for ti in 0..4 {
        draw_rectangle(fx(hl + 5.0 + ti as f32 * 4.0, 2.0), y + 13.0, 2.0, 3.0, palette::SOCKET);
    }

    // Меховая горжетка на плечах.
    draw_rectangle(fx(2.0, 12.0), y + 20.0, 12.0, 3.0, bone);
    draw_rectangle(fx(w - 14.0, 12.0), y + 20.0, 12.0, 3.0, bone);
    draw_rectangle(fx(4.0, 8.0), y + 23.0, 8.0, 2.0, bone_dark);
    draw_rectangle(fx(w - 12.0, 8.0), y + 23.0, 8.0, 2.0, bone_dark);
    // Цепь через грудь с подвеской-реликвией.
    draw_rectangle(fx(7.0, 26.0), y + 18.0, 26.0, 2.0, palette::STEEL_DARK);
    for l in 0..4 {
        draw_rectangle(fx(8.0 + l as f32 * 6.0, 3.0), y + 17.0, 3.0, 4.0, palette::STEEL);
    }
    draw_rectangle(fx(19.0, 5.0), y + 27.0, 5.0, 6.0, palette::GOLD);
    draw_rectangle(fx(20.0, 3.0), y + 28.0, 3.0, 4.0, palette::RUST);
    // Ярость: пульсирующая аура углей.
    if brain.enraged {
        let pulse = 0.5 + 0.5 * (time * 5.0).sin();
        draw_rectangle(
            x + 2.0,
            y + h - 3.0,
            w - 4.0,
            2.0,
            Color::new(0.90, 0.45, 0.15, 0.20 + 0.25 * pulse),
        );
        draw_rectangle(
            x + 4.0,
            y + 10.0,
            w - 8.0,
            1.5,
            Color::new(0.90, 0.45, 0.15, 0.15 + 0.20 * pulse),
        );
    }

    // --- клеймор по фазе FSM ---
    let (sx, sy, sword_len, raised) = match brain.state {
        EnemyState::Windup => (w - 6.0, y + 26.0, 24.0, true),
        EnemyState::Strike => (w - 2.0, y + 30.0, 34.0, false),
        EnemyState::Recover => (w - 8.0, y + 38.0, 22.0, false),
        EnemyState::Hurt => (3.0, y + 36.0, 18.0, false),
        _ => (w - 9.0, y + 34.0, 24.0, false),
    };
    // Кисть на рукояти.
    draw_rectangle(fx(sx, 6.0), sy, 6.0, 6.0, plate_dark);
    let blade_x = if facing_right {
        x + sx + 5.0
    } else {
        x + w - sx - 5.0 - 4.0
    };
    if raised {
        // Меч наверху — телеграф.
        draw_rectangle(blade_x, sy - 20.0, 5.0, 22.0, steel);
        draw_rectangle(blade_x, sy - 23.0, 5.0, 4.0, Color::new(0.85, 0.84, 0.86, 1.0));
        // Крестовина.
        draw_rectangle(blade_x - 4.0, sy - 1.0, 13.0, 4.0, palette::STEEL_DARK);
        draw_rectangle(blade_x - 4.0, sy - 1.0, 13.0, 1.0, palette::GOLD);
    } else {
        // Клинок вытянут.
        let bx = if facing_right {
            blade_x
        } else {
            blade_x - sword_len + 5.0
        };
        draw_rectangle(bx, sy + 1.0, sword_len, 5.0, steel);
        // Кромка и дол.
        draw_rectangle(bx, sy + 1.0, sword_len, 1.0, Color::new(0.90, 0.89, 0.91, 1.0));
        draw_rectangle(
            bx + 4.0,
            sy + 3.0,
            (sword_len - 8.0).max(2.0),
            1.0,
            palette::STEEL_DARK,
        );
        // Крестовина у рукояти.
        draw_rectangle(
            bx + if facing_right { 0.0 } else { sword_len - 5.0 },
            sy - 1.0,
            5.0,
            8.0,
            palette::STEEL_DARK,
        );
    }

    // --- полоса HP босса над головой ---
    let max_hp = params(brain.kind).hp as f32;
    let frac = (health.hp as f32 / max_hp).clamp(0.0, 1.0);
    let bar_w = w + 16.0;
    let bar_x = x - 8.0;
    let bar_y = y - 18.0;
    draw_rectangle(bar_x - 1.0, bar_y - 1.0, bar_w + 2.0, 7.0, palette::OUTLINE);
    draw_rectangle(bar_x, bar_y, bar_w, 5.0, Color::new(0.10, 0.08, 0.07, 0.92));
    let fill_col = if brain.enraged {
        Color::new(0.75, 0.30, 0.16, 1.0)
    } else {
        Color::new(0.71, 0.55, 0.26, 1.0)
    };
    draw_rectangle(bar_x, bar_y, bar_w * frac, 5.0, fill_col);
    // Деления полосы — сегменты HP (по 4 единицы).
    let seg = bar_w / max_hp * 4.0;
    let mut sx2 = bar_x + seg;
    while sx2 < bar_x + bar_w - 1.0 {
        draw_rectangle(sx2, bar_y, 1.0, 5.0, Color::new(0.10, 0.08, 0.07, 0.9));
        sx2 += seg;
    }
}
