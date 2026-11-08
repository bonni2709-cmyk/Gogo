//! Культист: патруль, удержание средней дистанции и бросок ножа.
//!
//! FSM: Patrol → Chase (держит `KEEP_NEAR..KEEP_FAR`: отступает, если
//! игрок подобрался, сближается, если далеко) → Windup (замах) →
//! Strike (бросок: ставит флаг [`EnemyBrain::want_throw`], `EnemyWorld`
//! создаёт снаряд) → Recover → Chase. У обрывов не сходит с площадки.

use macroquad::color::Color;
use macroquad::shapes::{draw_rectangle, draw_rectangle_lines};

use super::{palette, timing, *};
use crate::physics::{move_toward, World};

/// Ближе этой дистанции — отступает.
const KEEP_NEAR: f32 = 115.0;
/// Дальше этой дистанции — сближается.
const KEEP_FAR: f32 = 190.0;
/// Диапазон броска (полоса, в которой культист держит цель).
const THROW_MIN: f32 = 120.0;
const THROW_MAX: f32 = 200.0;
/// Резкая остановка тела на замахе.
const WINDUP_STOP: f32 = 2600.0;

/// Шаг FSM культиста.
pub(super) fn update(
    brain: &mut EnemyBrain,
    t: &mut Transform,
    vel: &mut Velocity,
    world: &World,
    target: &TargetInfo,
    dt: f32,
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
            let p = params(brain.kind);
            let desired = if target.dist > KEEP_FAR {
                // Дальше досягаемости — сближаемся.
                toward * p.chase_speed
            } else if target.dist < KEEP_NEAR {
                // Слишком близко — отступаем спиной, но не в обрыв.
                let back = -toward;
                if brain.on_ground && ledge_ahead(world, t, back) {
                    0.0
                } else {
                    back * p.chase_speed * 0.85
                }
            } else {
                // Средняя полоса — стоим и работаем метательством.
                0.0
            };
            vel.0.x = desired;
            if desired.abs() > 1.0 {
                brain.dir = desired.signum();
            } else {
                // Смотрим в цель — бросок пойдёт по направлению взгляда.
                brain.dir = toward;
            }

            if !brain.aggro {
                brain.transition(EnemyState::Patrol);
            } else if target.los
                && target.dist >= THROW_MIN
                && target.dist <= THROW_MAX
                && brain.cooldown <= 0.0
                && brain.on_ground
            {
                brain.transition(EnemyState::Windup);
                vel.0.x = 0.0;
            }
        }
        EnemyState::Windup => {
            // Замах: тело замирает, рука с ножом поднята.
            vel.0.x = move_toward(vel.0.x, 0.0, WINDUP_STOP * dt);
            if brain.state_timer >= timing::THROW_WINDUP {
                brain.transition(EnemyState::Strike);
                // Снаряд создаётся на первом кадре Strike.
                brain.want_throw = true;
            }
        }
        EnemyState::Strike => {
            // Кадр броска: короткий, дальше откат.
            if brain.state_timer >= timing::THROW_STRIKE {
                brain.transition(EnemyState::Recover);
                brain.cooldown = timing::THROW_COOLDOWN;
            }
        }
        EnemyState::Recover => {
            vel.0.x = move_toward(vel.0.x, 0.0, 1800.0 * dt);
            if brain.state_timer >= timing::THROW_RECOVER {
                brain.transition(if brain.aggro {
                    EnemyState::Chase
                } else {
                    EnemyState::Patrol
                });
            }
        }
        // Лётные/чужие состояния культисту не приходят.
        _ => brain.transition(EnemyState::Patrol),
    }
}

/// Отрисовка культиста: роба, капюшон с костяной маской, нож по фазе FSM.
pub(super) fn draw(t: &Transform, brain: &EnemyBrain, health: &Health, time: f32) {
    let x = t.pos.x;
    let y = t.pos.y;
    let w = t.size.x;
    let h = t.size.y;
    let facing_right = t.facing >= 0.0;

    let robe = flash_color(palette::CULTIST_ROBE, health.hit_flash);
    let dark = flash_color(palette::CULTIST_DARK, health.hit_flash);
    let mask = flash_color(palette::CULTIST_MASK, health.hit_flash);
    let steel = flash_color(palette::STEEL, health.hit_flash);

    let fx = |lx: f32, ew: f32| -> f32 {
        if facing_right {
            x + lx
        } else {
            x + w - lx - ew
        }
    };

    // Тень.
    draw_rectangle(x + 3.0, y + h - 1.0, w - 6.0, 3.0, palette::SHADOW);

    // Роба: три полосы-складки, колышутся на ветру.
    let sway = (time * 5.0).sin() * 1.2;
    draw_rectangle(fx(3.0 + sway, 6.0), y + 10.0, 6.0, h - 12.0, robe);
    draw_rectangle(fx(9.0, 7.0), y + 9.0, 7.0, h - 11.0, dark);
    draw_rectangle(fx(15.0 - sway * 0.6, 6.0), y + 10.0, 6.0, h - 12.0, robe);
    // Рваный подол.
    for (hx, hh) in [(3.0, 4.0), (9.0, 6.0), (15.0, 3.0), (19.0, 5.0)] {
        draw_rectangle(fx(hx, 4.0), y + h - 5.0 - hh, 4.0, hh, dark);
    }

    // Сапоги.
    draw_rectangle(fx(5.0, 6.0), y + h - 6.0, 6.0, 6.0, palette::OUTLINE);
    draw_rectangle(fx(13.0, 6.0), y + h - 6.0, 6.0, 6.0, palette::OUTLINE);

    // Пояс с золотой пряжкой культа.
    draw_rectangle(fx(4.0, 16.0), y + 22.0, 16.0, 3.0, palette::OUTLINE);
    draw_rectangle(fx(10.0, 4.0), y + 22.0, 4.0, 3.0, palette::STEEL);
    draw_rectangle(fx(11.0, 2.0), y + 23.0, 2.0, 1.0, palette::GOLD);

    // Капюшон: острый верх, глубокая тень лица, костяная маска.
    let hood_top = y + 1.0;
    draw_rectangle(fx(4.0, 16.0) - 1.0, hood_top - 1.0, 18.0, 14.0, palette::OUTLINE);
    draw_rectangle(fx(4.0, 16.0), hood_top, 16.0, 12.0, robe);
    // Остроконечный верх капюшона.
    draw_rectangle(fx(7.0, 10.0), hood_top - 3.0, 10.0, 4.0, robe);
    draw_rectangle(fx(9.0, 6.0), hood_top - 5.0, 6.0, 3.0, dark);
    // Тень под капюшоном.
    draw_rectangle(fx(6.0, 12.0), hood_top + 4.0, 12.0, 7.0, palette::OUTLINE);
    // Маска: лоб и скулы.
    draw_rectangle(fx(7.0, 10.0), hood_top + 4.0, 10.0, 3.0, mask);
    draw_rectangle(fx(8.0, 8.0), hood_top + 7.0, 8.0, 3.0, mask);
    // Глаза-щели: тёмные прорези, при агро — холодная искра.
    let eye = if brain.aggro {
        Color::new(0.85, 0.78, 0.55, 1.0)
    } else {
        palette::SOCKET
    };
    draw_rectangle(fx(8.0, 3.0), hood_top + 5.0, 3.0, 2.0, eye);
    draw_rectangle(fx(13.0, 3.0), hood_top + 5.0, 3.0, 2.0, eye);
    // Прорезь рта маски.
    draw_rectangle(fx(10.0, 4.0), hood_top + 9.0, 4.0, 1.0, palette::SOCKET);

    // Рука с ножом — поза по фазе.
    let (hand_lx, hand_ly, blade_up, blade_fwd) = match brain.state {
        EnemyState::Windup => (w - 5.0, y - 4.0, true, false), // нож наверху
        EnemyState::Strike => (w - 1.0, y + 12.0, false, true), // рука вперёд
        EnemyState::Recover => (w - 6.0, y + 18.0, false, false),
        EnemyState::Hurt => (2.0, y + 16.0, false, false),
        _ => (w - 6.0, y + 15.0, false, false),
    };
    // Кисть в рукаве.
    draw_rectangle(fx(hand_lx, 5.0), hand_ly, 5.0, 5.0, dark);

    // Нож: стальной клинок + рукоять.
    let knife_lx = if facing_right {
        x + hand_lx + 4.0
    } else {
        x + w - hand_lx - 4.0 - 4.0
    };
    if blade_up {
        // Замах: клинок торчит вверх.
        draw_rectangle(knife_lx, hand_ly - 9.0, 3.0, 10.0, steel);
        draw_rectangle(knife_lx - 2.0, hand_ly - 1.0, 7.0, 2.0, palette::STEEL_DARK);
        draw_rectangle(knife_lx, hand_ly - 11.0, 3.0, 2.0, Color::new(0.9, 0.9, 0.93, 1.0));
    } else if blade_fwd {
        // Бросок: клинок вытянут горизонтально.
        let bx = if facing_right {
            knife_lx
        } else {
            knife_lx - 11.0
        };
        draw_rectangle(bx, hand_ly + 1.0, 11.0, 3.0, steel);
        draw_rectangle(bx, hand_ly + 1.0, 11.0, 1.0, Color::new(0.9, 0.9, 0.93, 1.0));
        draw_rectangle(
            bx + if facing_right { 9.0 } else { 0.0 },
            hand_ly,
            2.0,
            5.0,
            palette::STEEL_DARK,
        );
    } else {
        // Опущен: короткий клинок вдоль бедра.
        let bx = if facing_right {
            knife_lx
        } else {
            knife_lx - 7.0
        };
        draw_rectangle(bx, hand_ly + 3.0, 7.0, 2.0, steel);
        draw_rectangle(
            bx + if facing_right { 5.0 } else { 0.0 },
            hand_ly + 2.0,
            2.0,
            4.0,
            palette::STEEL_DARK,
        );
    }

    // Значок культа на груди (маленький ржавый диск).
    draw_rectangle(fx(10.0, 4.0), y + 13.0, 4.0, 4.0, palette::RUST);
    draw_rectangle(fx(11.0, 2.0), y + 14.0, 2.0, 2.0, palette::OUTLINE);
    // Обводка робы.
    draw_rectangle_lines(fx(3.0, 18.0), y + 9.0, 18.0, h - 10.0, 1.0, palette::OUTLINE);
}
