//! Летучая мышь: парит у потолка, просыпается при приближении игрока,
//! преследует по прямой с волнистым маневром и возвращается к точке парения.

use macroquad::shapes::draw_rectangle;

use super::{palette, timing, *};
use crate::physics::World;

/// Плавность разворота вектора скорости к цели.
const STEER_LERP: f32 = 4.5;
/// Амплитуда волны поперёк курса.
const WOBBLE_AMPLITUDE: f32 = 42.0;
/// Скорость возврата к точке парения.
const RETURN_SPEED: f32 = 110.0;

/// Шаг FSM летучей мыши.
pub(super) fn update(
    brain: &mut EnemyBrain,
    t: &mut Transform,
    vel: &mut Velocity,
    _world: &World,
    target: &TargetInfo,
    dt: f32,
) {
    match brain.state {
        EnemyState::Hurt => {
            if brain.state_timer >= timing::HURT {
                brain.transition(if brain.aggro {
                    EnemyState::Chase
                } else {
                    EnemyState::Return
                });
            }
        }
        EnemyState::Rest => {
            // Парение: мягко держимся у точки, лёгкая покачка.
            vel.0 = Vec2::ZERO;
            let drift = Vec2::new(
                (brain.wobble * 1.7).sin() * 6.0,
                (brain.wobble * 2.3).cos() * 5.0,
            );
            let anchor = brain.anchor;
            let pull = (anchor - t.center()) * 2.4;
            vel.0 += pull + drift;
            // Ограничиваем скорость парения.
            let max_speed = 90.0;
            let speed2 = vel.0.length_squared();
            if speed2 > max_speed * max_speed {
                vel.0 *= max_speed / speed2.sqrt();
            }

            if brain.aggro {
                brain.transition(EnemyState::Chase);
            }
        }
        EnemyState::Chase => {
            let to_target = target.center - t.center();
            let speed = params(brain.kind).chase_speed;
            let mut desired = if to_target.length() > 6.0 {
                to_target.normalize() * speed
            } else {
                Vec2::ZERO
            };
            // Волнистый полёт — не даёт игроку «читать» прямую линию.
            if desired.length() > 1.0 {
                let perp = Vec2::new(-desired.y, desired.x).normalize();
                desired += perp * (brain.wobble * 7.0).sin() * WOBBLE_AMPLITUDE;
            }
            let k = (STEER_LERP * dt).clamp(0.0, 1.0);
            vel.0 += (desired - vel.0) * k;

            if !brain.aggro {
                brain.transition(EnemyState::Return);
            }
        }
        EnemyState::Return => {
            // Летим к точке парения; по пути агро может включиться снова.
            let to_anchor = brain.anchor - t.center();
            if to_anchor.length() < 8.0 {
                vel.0 = Vec2::ZERO;
                brain.transition(EnemyState::Rest);
            } else {
                let desired = to_anchor.normalize() * RETURN_SPEED;
                let k = (STEER_LERP * dt).clamp(0.0, 1.0);
                vel.0 += (desired - vel.0) * k;
            }
            if brain.aggro {
                brain.transition(EnemyState::Chase);
            }
        }
        // Наземные состояния летучей мыши не приходят.
        _ => brain.transition(EnemyState::Rest),
    }
}

/// Отрисовка летучей мыши: тело, крылья в такт полёту, тусклые глаза.
pub(super) fn draw(t: &Transform, brain: &EnemyBrain, health: &Health, time: f32) {
    let x = t.pos.x;
    let y = t.pos.y;
    let w = t.size.x;
    let h = t.size.y;
    let body = flash_color(palette::BAT, health.hit_flash);
    let wing = flash_color(palette::BAT_WING, health.hit_flash);

    // Фаза взмаха: у потолка крылья сложены, в полёте бьют часто.
    let flap = match brain.state {
        EnemyState::Rest => 0.0,
        _ => (time * 16.0 + brain.wobble * 3.0).sin(),
    };

    if brain.state == EnemyState::Rest {
        // Сложенные крылья — два тёмных «кокона» по бокам.
        draw_rectangle(x - 2.0, y + 3.0, 4.0, h - 5.0, wing);
        draw_rectangle(x + w - 2.0, y + 3.0, 4.0, h - 5.0, wing);
    } else {
        // Развёрнутые крылья: ступеньки пикселей вверх/вниз от взмаха.
        let lift = flap * 5.0;
        // Левое крыло.
        draw_rectangle(x - 7.0, y + 4.0 - lift, 7.0, 4.0, wing);
        draw_rectangle(x - 11.0, y + 7.0 - lift * 1.6, 4.0, 4.0, wing);
        // Правое крыло.
        draw_rectangle(x + w, y + 4.0 + lift, 7.0, 4.0, wing);
        draw_rectangle(x + w + 7.0, y + 7.0 + lift * 1.6, 4.0, 4.0, wing);
    }

    // Тело.
    draw_rectangle(x + 2.0, y + 2.0, w - 4.0, h - 4.0, body);
    draw_rectangle(x + 4.0, y, 4.0, 4.0, body); // ухо 1
    draw_rectangle(x + w - 8.0, y, 4.0, 4.0, body); // ухо 2

    // Глаза: тусклая янтарная искра (без неона).
    let eye_y = y + 6.0;
    if t.facing >= 0.0 {
        draw_rectangle(x + w - 8.0, eye_y, 2.0, 2.0, palette::BAT_EYE);
        draw_rectangle(x + w - 4.0, eye_y, 2.0, 2.0, palette::BAT_EYE);
    } else {
        draw_rectangle(x + 4.0, eye_y, 2.0, 2.0, palette::BAT_EYE);
        draw_rectangle(x + 8.0, eye_y, 2.0, 2.0, palette::BAT_EYE);
    }

    // Клыки в покое/полёте.
    if brain.state != EnemyState::Rest {
        let fang_y = y + h - 3.0;
        let (f1, f2) = if t.facing >= 0.0 {
            (x + w - 8.0, x + w - 4.0)
        } else {
            (x + 4.0, x + 8.0)
        };
        draw_rectangle(f1, fang_y, 2.0, 2.0, palette::BONE);
        draw_rectangle(f2, fang_y, 2.0, 2.0, palette::BONE);
    }
}
