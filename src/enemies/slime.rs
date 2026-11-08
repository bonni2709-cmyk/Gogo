//! Слизень: патруль с разворотом у обрывов, погоня и прыжок-выпад.

use macroquad::color::Color;
use macroquad::shapes::{draw_circle, draw_rectangle};

use super::{palette, timing, *};
use crate::physics::World;

/// Дистанция, с которой слизень прыгает на игрока.
const HOP_TRIGGER_DIST: f32 = 95.0;
/// Скорость прыжка-выпада.
const HOP_SPEED_X: f32 = 178.0;
const HOP_SPEED_Y: f32 = -430.0;

/// Шаг FSM слизня (физика — общая, в [`EnemyWorld::update`]).
pub(super) fn update(
    brain: &mut EnemyBrain,
    t: &mut Transform,
    vel: &mut Velocity,
    world: &World,
    target: &TargetInfo,
    _dt: f32,
) {
    match brain.state {
        EnemyState::Hurt => {
            // Оглушение: тело просто летит по инерции (физика снаружи).
            if brain.state_timer >= timing::HURT {
                brain.transition(if brain.aggro {
                    EnemyState::Chase
                } else {
                    EnemyState::Patrol
                });
            }
        }
        EnemyState::Patrol => {
            // Разворот у обрыва и у стены.
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
            // Стоим на месте у обрыва — не падаем за игроком в пропасть.
            let toward = (target.center.x - t.center().x).signum();
            if toward.abs() > f32::EPSILON {
                brain.dir = toward;
            }
            let ledge = brain.on_ground && ledge_ahead(world, t, brain.dir);
            vel.0.x = if ledge {
                0.0
            } else {
                brain.dir * params(brain.kind).chase_speed
            };

            if !brain.aggro {
                brain.transition(EnemyState::Patrol);
            } else if brain.on_ground
                && brain.cooldown <= 0.0
                && target.dist < HOP_TRIGGER_DIST
            {
                // Прыжок-выпад: подлетает к игроку дугой.
                brain.transition(EnemyState::Hop);
                brain.cooldown = timing::HOP_COOLDOWN;
                vel.0.x = brain.dir * HOP_SPEED_X;
                vel.0.y = HOP_SPEED_Y;
            }
        }
        EnemyState::Hop => {
            // В воздухе слегка доворачиваем к цели.
            if target.los && target.dist < 130.0 {
                let toward = (target.center.x - t.center().x).signum();
                if toward.abs() > f32::EPSILON {
                    brain.dir = toward;
                }
            }
            // Приземлились — назад в цикл.
            if brain.on_ground && brain.state_timer >= 0.12 {
                brain.transition(if brain.aggro {
                    EnemyState::Chase
                } else {
                    EnemyState::Patrol
                });
            }
        }
        // Состояния других видов слизню не приходят.
        _ => brain.transition(EnemyState::Patrol),
    }
}

/// Отрисовка слизня: приплюснутый блоб с глазами спереди.
pub(super) fn draw(t: &Transform, brain: &EnemyBrain, health: &Health, time: f32) {
    let x = t.pos.x;
    let y = t.pos.y;
    let w = t.size.x;
    let h = t.size.y;
    let facing_right = t.facing >= 0.0;
    let flash = flash_color(palette::SLIME, health.hit_flash);

    // Тень.
    draw_rectangle(x + 2.0, y + h - 1.0, w - 4.0, 3.0, palette::SHADOW);

    // «Дыхание» в покое, растяжение в прыжке.
    let (squash, stretch) = match brain.state {
        EnemyState::Hop => (0.88, 1.14),
        EnemyState::Chase => (0.96, 1.06),
        _ => {
            let breath = (time * 3.0).sin() * 0.5 + 0.5;
            (1.0 - breath * 0.06, 1.0 + breath * 0.06)
        }
    };
    let dw = w * squash;
    let dh = h * stretch;
    let dx = x + (w - dw) * 0.5;
    let dy = y + h - dh;

    // Обводка силуэта.
    draw_rectangle(dx - 1.0, dy - 1.0, dw + 2.0, dh + 2.0, palette::OUTLINE);

    // Тело и «спина» потемнее.
    draw_rectangle(dx, dy + dh * 0.35, dw, dh * 0.65, flash);
    draw_rectangle(dx + 2.0, dy, dw - 4.0, dh * 0.45, flash_color(palette::SLIME_DARK, health.hit_flash));
    // Брюхо — светлее.
    draw_rectangle(dx + 4.0, dy + dh * 0.62, dw - 8.0, dh * 0.24, flash_color(palette::SLIME_BELLY, health.hit_flash));

    // Влажный блик и пузыри внутри.
    draw_rectangle(dx + 3.0, dy + 2.0, dw * 0.34, 2.0, flash_color(palette::SLIME_GLOSS, health.hit_flash));
    draw_rectangle(dx + 4.0, dy + 4.0, 2.0, 2.0, flash_color(palette::SLIME_GLOSS, health.hit_flash));
    let bubble = (time * 2.2 + x * 0.13).sin() * 1.5;
    draw_circle(
        dx + dw * 0.62,
        dy + dh * 0.55 + bubble,
        2.4,
        Color::new(0.5, 0.56, 0.34, 0.55),
    );
    // Тёмные пятна-вкрапления.
    draw_rectangle(dx + dw * 0.55, dy + dh * 0.72, 4.0, 3.0, palette::SLIME_DARK);
    draw_rectangle(dx + dw * 0.2, dy + dh * 0.8, 3.0, 2.0, palette::SLIME_DARK);
    // Ядро, просвечивающее сквозь желе.
    draw_rectangle(
        dx + dw * 0.40,
        dy + dh * 0.50 + bubble * 0.5,
        5.0,
        4.0,
        Color::new(0.42, 0.50, 0.26, 0.6),
    );
    // Капли слизи, стекающие на землю.
    for i in 0..2 {
        let drop_phase = (time * 1.3 + i as f32 * 0.7 + x * 0.07).rem_euclid(1.0);
        let drop_x = dx + 6.0 + i as f32 * (dw - 14.0).max(4.0);
        let drop_y = dy + dh - 2.0 + drop_phase * 6.0;
        draw_rectangle(
            drop_x,
            drop_y,
            2.0,
            3.0,
            Color::new(0.42, 0.50, 0.26, (1.0 - drop_phase) * 0.7),
        );
    }

    // Глаза смотрит в сторону движения.
    let eye_y = dy + dh * 0.42;
    let (eye1, eye2) = if facing_right {
        (dx + dw - 9.0, dx + dw - 4.0)
    } else {
        (dx + 3.0, dx + 8.0)
    };
    draw_rectangle(eye1, eye_y, 3.0, 3.0, palette::SOCKET);
    draw_rectangle(eye2, eye_y, 3.0, 3.0, palette::SOCKET);

    // Усы-капли спереди в движении.
    if matches!(brain.state, EnemyState::Patrol | EnemyState::Chase) {
        let nose_x = if facing_right { dx + dw } else { dx - 2.0 };
        draw_rectangle(nose_x, dy + dh * 0.55, 2.0, 3.0, flash);
    }
}
