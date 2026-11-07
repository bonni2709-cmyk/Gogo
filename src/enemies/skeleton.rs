//! Скелет-мечник: патруль, погоня и выпад с длинной фазой телеграфа
//! (Windup → Strike → Recover) — игрок успевает уйти или перехватить удар.

use macroquad::color::Color;
use macroquad::shapes::{draw_rectangle, draw_rectangle_lines};

use super::{palette, timing, *};
use crate::physics::{move_toward, World};

/// Дистанция, с которой скелет начинает атаку.
const ATTACK_RANGE: f32 = 48.0;
/// Скорость выпада в активной фазе.
const LUNGE_SPEED: f32 = 285.0;
/// Остановка тела на время замаха.
const WINDUP_STOP: f32 = 2600.0;

/// Шаг FSM скелета.
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
            // У обрыва стоим на месте — не сходим за целью в пропасть.
            let ledge = brain.on_ground && ledge_ahead(world, t, brain.dir);
            vel.0.x = if ledge {
                0.0
            } else {
                brain.dir * params(brain.kind).chase_speed
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
            // Телеграф: замирает, меч поднят. Тормозим резко.
            vel.0.x = move_toward(vel.0.x, 0.0, WINDUP_STOP * _dt);
            if brain.state_timer >= timing::WINDUP {
                brain.transition(EnemyState::Strike);
                vel.0.x = brain.dir * LUNGE_SPEED;
            }
        }
        EnemyState::Strike => {
            // Выпад: тело несёт вперёд, удар идёт по касанию.
            if brain.state_timer >= timing::STRIKE {
                brain.transition(EnemyState::Recover);
                brain.cooldown = timing::ATTACK_COOLDOWN;
                vel.0.x = 0.0;
            }
        }
        EnemyState::Recover => {
            // Откат: можно двигаться, но атака на кулдауне.
            vel.0.x = move_toward(vel.0.x, 0.0, 1800.0 * _dt);
            if brain.state_timer >= timing::RECOVER {
                brain.transition(if brain.aggro {
                    EnemyState::Chase
                } else {
                    EnemyState::Patrol
                });
            }
        }
        // Полётные состояния скелету не приходят.
        _ => brain.transition(EnemyState::Patrol),
    }
}

/// Отрисовка скелета: кость, рёбра, шлем-череп и стальной меч.
pub(super) fn draw(t: &Transform, brain: &EnemyBrain, health: &Health, time: f32) {
    let x = t.pos.x;
    let y = t.pos.y;
    let w = t.size.x;
    let h = t.size.y;
    let facing_right = t.facing >= 0.0;

    let bone = flash_color(palette::BONE, health.hit_flash);
    let bone_dark = flash_color(palette::BONE_DARK, health.hit_flash);
    let steel = flash_color(palette::STEEL, health.hit_flash);

    // Зеркалирование локальных координат.
    let fx = |lx: f32, ew: f32| -> f32 {
        if facing_right {
            x + lx
        } else {
            x + w - lx - ew
        }
    };

    // Тень.
    draw_rectangle(x + 3.0, y + h - 1.0, w - 6.0, 3.0, palette::SHADOW);

    // Ноги: шаг в патруле, стойка в бою.
    let walking = matches!(brain.state, EnemyState::Patrol | EnemyState::Chase);
    let swing = if walking { (time * 7.0).sin() * 3.0 } else { 0.0 };
    let (leg1, leg2) = if brain.state == EnemyState::Windup {
        (7.0, 14.0) // отступил, устойчиво
    } else {
        (5.0 + swing, 13.0 - swing)
    };
    draw_rectangle(fx(leg1, 5.0), y + h - 13.0, 5.0, 13.0, bone_dark);
    draw_rectangle(fx(leg2, 5.0), y + h - 13.0, 5.0, 13.0, bone_dark);
    // Коленные суставы.
    draw_rectangle(fx(leg1 + 1.0, 3.0), y + h - 9.0, 3.0, 3.0, palette::SOCKET);
    draw_rectangle(fx(leg2 + 1.0, 3.0), y + h - 9.0, 3.0, 3.0, palette::SOCKET);

    // Таз и торс (с обводкой).
    draw_rectangle(fx(5.0, 16.0), y + h - 16.0, 16.0, 5.0, bone_dark);
    draw_rectangle(fx(6.0, 14.0) - 1.0, y + 11.0, 16.0, 16.0, palette::OUTLINE);
    draw_rectangle(fx(6.0, 14.0), y + 12.0, 14.0, 14.0, bone);
    // Позвоночник по центру.
    draw_rectangle(fx(12.0, 2.0), y + 13.0, 2.0, 12.0, bone_dark);
    // Рёбра.
    for i in 0..3 {
        draw_rectangle(
            fx(7.0, 12.0),
            y + 14.0 + i as f32 * 4.0,
            12.0,
            2.0,
            bone_dark,
        );
    }
    // Рваные лохмотья поверх бёдер.
    draw_rectangle(fx(6.0, 4.0), y + h - 13.0, 4.0, 7.0, palette::TATTER);
    draw_rectangle(fx(11.0, 4.0), y + h - 13.0, 4.0, 9.0, palette::TATTER);
    draw_rectangle(fx(16.0, 3.0), y + h - 13.0, 3.0, 6.0, palette::TATTER);

    // Череп с обводкой.
    draw_rectangle(fx(5.0, 16.0) - 1.0, y - 1.0, 18.0, 14.0, palette::OUTLINE);
    draw_rectangle(fx(5.0, 16.0), y, 16.0, 12.0, bone);
    // Надбровье и трещина — характер «старого» скелета.
    draw_rectangle(fx(5.0, 16.0), y + 3.0, 16.0, 1.0, bone_dark);
    draw_rectangle(fx(8.0, 2.0), y + 1.0, 1.0, 3.0, palette::SOCKET);
    draw_rectangle(fx(9.0, 2.0), y + 3.0, 2.0, 1.0, palette::SOCKET);
    // Глазницы и челюсть.
    let socket_y = y + 4.0;
    if facing_right {
        draw_rectangle(x + w - 11.0, socket_y, 4.0, 3.0, palette::SOCKET);
        draw_rectangle(x + w - 5.0, socket_y, 3.0, 3.0, palette::SOCKET);
    } else {
        draw_rectangle(x + 2.0, socket_y, 4.0, 3.0, palette::SOCKET);
        draw_rectangle(x + 8.0, socket_y, 3.0, 3.0, palette::SOCKET);
    }
    draw_rectangle(fx(7.0, 10.0), y + 9.0, 10.0, 2.0, bone_dark);
    // Зубы в челюсти.
    for ti in 0..3 {
        draw_rectangle(fx(8.0 + ti as f32 * 3.0, 1.0), y + 9.0, 1.0, 2.0, palette::SOCKET);
    }
    // Полоса ржавчины на черепе — следы старых боёв.
    draw_rectangle(fx(6.0, 6.0), y + 1.0, 6.0, 2.0, palette::RUST);

    // Рука с мечом: поза зависит от фазы FSM.
    let (sword_lx, sword_ly, sword_len, sword_up) = match brain.state {
        EnemyState::Windup => (w - 4.0, y + 2.0, 16.0, true),
        EnemyState::Strike => (w - 2.0, y + 16.0, 22.0, false),
        EnemyState::Recover => (w - 6.0, y + 20.0, 14.0, false),
        EnemyState::Hurt => (2.0, y + 18.0, 12.0, false),
        _ => (w - 7.0, y + 17.0, 13.0, false),
    };
    // Кисть.
    draw_rectangle(fx(sword_lx, 4.0), sword_ly, 4.0, 4.0, bone);
    // Клинок.
    let blade_x = if facing_right {
        x + sword_lx + 3.0
    } else {
        x + w - sword_lx - 3.0 - 4.0
    };
    if sword_up {
        // Меч поднят вертикально — телеграф.
        draw_rectangle(blade_x, sword_ly - 14.0, 4.0, 15.0, steel);
        draw_rectangle(blade_x, sword_ly - 16.0, 4.0, 3.0, steel);
        // Крестовина.
        draw_rectangle(blade_x - 3.0, sword_ly + 1.0, 10.0, 3.0, palette::STEEL_DARK);
    } else {
        // Меч вытянут горизонтально.
        let bx = if facing_right {
            blade_x
        } else {
            blade_x - sword_len + 4.0
        };
        draw_rectangle(bx, sword_ly + 1.0, sword_len, 4.0, steel);
        // Кромка и дол клинка.
        draw_rectangle(bx, sword_ly + 1.0, sword_len, 1.0, Color::new(0.878, 0.855, 0.816, 1.0));
        draw_rectangle(bx + 3.0, sword_ly + 3.0, (sword_len - 6.0).max(2.0), 1.0, palette::STEEL_DARK);
        draw_rectangle(
            bx + if facing_right { sword_len - 2.0 } else { 0.0 },
            sword_ly + 1.0,
            2.0,
            4.0,
            palette::STEEL_DARK,
        );
    }
    // Ножны/ремень на торсе.
    draw_rectangle(fx(4.0, 18.0), y + 22.0, 18.0, 3.0, palette::RUST);
    draw_rectangle_lines(fx(5.0, 16.0), y + 12.0, 14.0, 14.0, 1.0, bone_dark);
}
