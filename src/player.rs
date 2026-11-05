//! Контроллер игрока: ощущение от управления уровня Apple Knight / Dead Cells.
//!
//! * Кривые разгона и торможения (инерция, скольжение).
//! * Coyote Time — прыжок в течение [`COYOTE_TIME`] после схода с края.
//! * Jump Buffer — прыжок, нажатый за [`JUMP_BUFFER_TIME`] до приземления.
//! * Variable Jump Height — при отпускании кнопки гравитация растёт.
//! * Wall Slide & Wall Jump — скольжение по стене и отскок от неё.
//! * Dash — рывок с i-frames [`DASH_IFRAMES`], кулдауном и воздуховым лимитом.
//!
//! Всё считается на фиксированном [`crate::physics::FIXED_DT`].

use macroquad::math::Vec2;

use crate::physics::{apply_gravity, move_toward, Aabb, World, GRAVITY, MAX_FALL_SPEED};

// ---------------------------------------------------------------------------
// Тюнинг игрока (пиксели, секунды)
// ---------------------------------------------------------------------------

pub const PLAYER_WIDTH: f32 = 24.0;
pub const PLAYER_HEIGHT: f32 = 40.0;

/// Максимальная скорость бега.
pub const RUN_SPEED: f32 = 210.0;
/// Разгон на земле.
pub const GROUND_ACCEL: f32 = 2600.0;
/// Торможение на земле.
pub const GROUND_DECEL: f32 = 3400.0;
/// Разгон в воздухе.
pub const AIR_ACCEL: f32 = 1900.0;
/// Торможение в воздухе.
pub const AIR_DECEL: f32 = 1500.0;

/// Начальная вертикальная скорость прыжка (вверх — отрицательная).
pub const JUMP_SPEED: f32 = -640.0;
/// Множитель гравитации при падении (снижение «висения»).
pub const FALL_GRAVITY_MULT: f32 = 1.45;
/// Множитель гравитации, пока игрок отпустил кнопку прыжка и ещё летит вверх
/// — чем раньше отпустил, тем ниже прыжок.
pub const JUMP_CUT_GRAVITY_MULT: f32 = 2.6;

/// Окно Coyote Time, с.
pub const COYOTE_TIME: f32 = 0.1;
/// Окно Jump Buffer, с.
pub const JUMP_BUFFER_TIME: f32 = 0.1;

/// Предельная скорость скольжения по стене.
pub const WALL_SLIDE_SPEED: f32 = 130.0;
/// Плавность перехода в скольжение, px/с².
pub const WALL_SLIDE_LERP: f32 = 4200.0;
/// Горизонтальная скорость отскока от стены.
pub const WALL_JUMP_SPEED_X: f32 = 280.0;
/// Вертикальная скорость отскока от стены.
pub const WALL_JUMP_SPEED_Y: f32 = -620.0;
/// Блокировка смены направления после отскока, с.
pub const WALL_JUMP_LOCK_TIME: f32 = 0.14;
/// Задержка повторного «прилипания» к стене после отскока, с.
pub const WALL_REGRAB_DELAY: f32 = 0.16;
/// Дальность пробы стены, px.
pub const WALL_PROBE_DISTANCE: f32 = 3.0;

/// Скорость рывка, px/с.
pub const DASH_SPEED: f32 = 540.0;
/// Длительность рывка, с.
pub const DASH_TIME: f32 = 0.18;
/// Кулдаун рывка, с.
pub const DASH_COOLDOWN: f32 = 0.45;
/// Кадры неуязвимости во время рывка, с.
pub const DASH_IFRAMES: f32 = 0.2;
/// Скорость выката после рывка (доля от бега).
pub const DASH_CARRY_SPEED: f32 = 0.55;

/// Ввод, необходимый игроку. Формируется каждый кадр:
/// сейчас — клавиатура, позже — тач-джойстик и кнопки из `ui`.
#[derive(Clone, Copy, Debug, Default)]
pub struct PlayerInput {
    /// Горизонтальное направление: `-1 ..= 1`.
    pub move_x: f32,
    /// Кнопка прыжка нажата в этом шаге (edge).
    pub jump_pressed: bool,
    /// Кнопка прыжка удерживается (для Variable Jump Height).
    pub jump_held: bool,
    /// Кнопка рывка нажата в этом шаге (edge).
    pub dash_pressed: bool,
}

/// Состояния конечного автомата игрока (кадры анимации).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayerState {
    Idle,
    Run,
    Jump,
    Fall,
    WallSlide,
    Dash,
}

pub struct Player {
    /// Левый верхний угол хитбокса.
    pub pos: Vec2,
    pub vel: Vec2,
    pub size: Vec2,
    /// Куда смотрит игрок: `-1.0` или `1.0`.
    pub facing: f32,
    /// Текущее состояние для анимации.
    pub state: PlayerState,
    /// Время в текущем состоянии, с.
    pub state_time: f32,
    /// Стоит ли на земле.
    pub on_ground: bool,
    /// Ближайшая стена: `-1` — слева, `1` — справа, `0` — нет.
    pub wall_dir: f32,

    // --- внутренние таймеры ---
    coyote_timer: f32,
    jump_buffer: f32,
    dash_timer: f32,
    dash_cooldown: f32,
    wall_jump_lock: f32,
    wall_regrab_delay: f32,
    /// Использован ли рывок в воздухе (сбрасывается на земле).
    air_dash_used: bool,
    dashing: bool,
    wall_sliding: bool,
}

impl Player {
    pub fn new(spawn: Vec2) -> Self {
        Self {
            pos: spawn,
            vel: Vec2::ZERO,
            size: Vec2::new(PLAYER_WIDTH, PLAYER_HEIGHT),
            facing: 1.0,
            state: PlayerState::Idle,
            state_time: 0.0,
            on_ground: false,
            wall_dir: 0.0,
            coyote_timer: 0.0,
            jump_buffer: 0.0,
            dash_timer: 0.0,
            dash_cooldown: 0.0,
            wall_jump_lock: 0.0,
            wall_regrab_delay: 0.0,
            air_dash_used: false,
            dashing: false,
            wall_sliding: false,
        }
    }

    /// Текущий хитбокс.
    pub fn body(&self) -> Aabb {
        Aabb {
            pos: self.pos,
            size: self.size,
        }
    }

    /// Полный сброс при респавне.
    pub fn respawn(&mut self, spawn: Vec2) {
        *self = Self::new(spawn);
    }

    /// Идёт ли рывок прямо сейчас.
    pub fn is_dashing(&self) -> bool {
        self.dashing
    }

    /// Есть ли кадры неуязвимости от рывка (читается боевой системой).
    pub fn has_dash_iframes(&self) -> bool {
        self.dash_timer > 0.0
    }

    /// Скольжется ли игрок по стене.
    pub fn is_wall_sliding(&self) -> bool {
        self.wall_sliding
    }

    /// Полный шаг симуляции игрока на фиксированном dt.
    pub fn update(&mut self, input: &PlayerInput, world: &World, dt: f32) {
        debug_assert!(dt > 0.0, "шаг симуляции должен быть положительным");

        // --- таймеры ---
        self.state_time += dt;
        self.coyote_timer -= dt;
        self.jump_buffer -= dt;
        self.dash_cooldown -= dt;
        self.wall_jump_lock -= dt;
        self.wall_regrab_delay -= dt;
        self.wall_sliding = false;

        let move_x = input.move_x.clamp(-1.0, 1.0);

        // --- проба стен (до логики прыжка/скольжения) ---
        self.wall_dir = self.probe_wall(world);

        if self.dashing {
            // ---------- DASH ----------
            self.dash_timer -= dt;
            self.vel.x = self.facing * DASH_SPEED;
            self.vel.y = 0.0;
            if self.dash_timer <= 0.0 {
                self.dashing = false;
                self.vel.x = self.facing * RUN_SPEED * DASH_CARRY_SPEED;
            }
        } else if input.dash_pressed && self.dash_cooldown <= 0.0 && !self.air_dash_used {
            // ---------- СТАРТ DASH ----------
            if move_x.abs() > 0.1 {
                self.facing = move_x.signum();
            }
            self.dashing = true;
            self.dash_timer = DASH_TIME;
            self.dash_cooldown = DASH_COOLDOWN;
            if !self.on_ground {
                self.air_dash_used = true;
            }
        } else {
            // ---------- ОБЫЧНОЕ ДВИЖЕНИЕ ----------
            // Кривые разгона/торможения: тело не останавливается мгновенно.
            let target = move_x * RUN_SPEED;
            let accel = if move_x.abs() > 0.01 {
                if self.on_ground {
                    GROUND_ACCEL
                } else {
                    AIR_ACCEL
                }
            } else if self.on_ground {
                GROUND_DECEL
            } else {
                AIR_DECEL
            };
            self.vel.x = move_toward(self.vel.x, target, accel * dt);

            if move_x.abs() > 0.1 && self.wall_jump_lock <= 0.0 {
                self.facing = move_x.signum();
            }

            // --- прыжок: буфер + койот + отскок от стены ---
            if input.jump_pressed {
                self.jump_buffer = JUMP_BUFFER_TIME;
            }

            if self.jump_buffer > 0.0 {
                if self.on_ground || self.coyote_timer > 0.0 {
                    self.vel.y = JUMP_SPEED;
                    self.on_ground = false;
                    self.coyote_timer = 0.0;
                    self.jump_buffer = 0.0;
                } else if self.wall_dir != 0.0 && self.wall_regrab_delay <= 0.0 {
                    // Отскок от стены: толчок назад, взгляд от стены.
                    self.vel.x = -self.wall_dir * WALL_JUMP_SPEED_X;
                    self.vel.y = WALL_JUMP_SPEED_Y;
                    self.facing = -self.wall_dir;
                    self.wall_jump_lock = WALL_JUMP_LOCK_TIME;
                    self.wall_regrab_delay = WALL_REGRAB_DELAY;
                    self.jump_buffer = 0.0;
                }
            }

            // --- гравитация + variable jump height ---
            let mut gravity = GRAVITY;
            if self.vel.y > 0.0 {
                gravity *= FALL_GRAVITY_MULT;
            } else if self.vel.y < 0.0 && !input.jump_held {
                gravity *= JUMP_CUT_GRAVITY_MULT;
            }
            apply_gravity(&mut self.vel, dt, gravity, MAX_FALL_SPEED);

            // --- wall slide: падение вдоль стены при нажатии в неё ---
            let pressing_into_wall = (self.wall_dir < 0.0 && move_x < -0.3)
                || (self.wall_dir > 0.0 && move_x > 0.3);
            if !self.on_ground
                && self.wall_dir != 0.0
                && pressing_into_wall
                && self.vel.y > 0.0
                && self.wall_regrab_delay <= 0.0
            {
                if self.vel.y > WALL_SLIDE_SPEED {
                    self.vel.y = move_toward(self.vel.y, WALL_SLIDE_SPEED, WALL_SLIDE_LERP * dt);
                }
                self.wall_sliding = true;
            }
        }

        // ---------- интеграция движения (разделение осей X → Y) ----------
        let (new_pos, new_vel, hit) = world.move_body(self.pos, self.size, self.vel, dt);
        self.pos = new_pos;
        self.vel = new_vel;
        self.on_ground = hit.on_ground;

        // Рывок, упершийся в стену, обрывается сразу — нет «упирания» в воздухе.
        if self.dashing && hit.wall_dir != 0.0 && hit.wall_dir == self.facing {
            self.dashing = false;
            self.vel.x = 0.0;
        }

        if self.on_ground {
            self.coyote_timer = COYOTE_TIME;
            self.air_dash_used = false;
        }

        // ---------- производное состояние для анимации ----------
        let new_state = if self.dashing {
            PlayerState::Dash
        } else if self.on_ground {
            if self.vel.x.abs() > 12.0 {
                PlayerState::Run
            } else {
                PlayerState::Idle
            }
        } else if self.wall_sliding {
            PlayerState::WallSlide
        } else if self.vel.y < -10.0 {
            PlayerState::Jump
        } else {
            PlayerState::Fall
        };

        if new_state != self.state {
            self.state = new_state;
            self.state_time = 0.0;
        }
    }

    /// Проба стены слева/справа (не цепляется за пол и потолок).
    fn probe_wall(&self, world: &World) -> f32 {
        let body = self.body();
        let inset_y = 6.0;
        let probe_h = body.size.y - inset_y * 2.0;

        let left = Aabb {
            pos: Vec2::new(body.left() - WALL_PROBE_DISTANCE, body.top() + inset_y),
            size: Vec2::new(WALL_PROBE_DISTANCE, probe_h),
        };
        if world.overlaps_any(left) {
            return -1.0;
        }

        let right = Aabb {
            pos: Vec2::new(body.right(), body.top() + inset_y),
            size: Vec2::new(WALL_PROBE_DISTANCE, probe_h),
        };
        if world.overlaps_any(right) {
            return 1.0;
        }

        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::physics::{Aabb, World, FIXED_DT};

    fn flat_world() -> (World, Vec2) {
        let mut world = World::new();
        world.add_solid(Aabb::new(-500.0, 300.0, 2000.0, 64.0));
        let spawn = Vec2::new(0.0, 300.0 - PLAYER_HEIGHT);
        (world, spawn)
    }

    #[test]
    fn lands_and_reports_ground() {
        let (world, mut spawn) = flat_world();
        spawn.y = 100.0;
        let mut player = Player::new(spawn);
        let idle = PlayerInput::default();
        for _ in 0..180 {
            player.update(&idle, &world, FIXED_DT);
        }
        assert!(player.on_ground);
        assert!((player.pos.y + PLAYER_HEIGHT - 300.0).abs() < 0.01);
        assert_eq!(player.state, PlayerState::Idle);
    }

    #[test]
    fn jump_rises_then_lands() {
        let (world, spawn) = flat_world();
        let mut player = Player::new(spawn);
        let idle = PlayerInput::default();
        // Приземлиться перед стартом.
        for _ in 0..30 {
            player.update(&idle, &world, FIXED_DT);
        }
        assert!(player.on_ground);

        let jump = PlayerInput {
            jump_pressed: true,
            jump_held: true,
            ..Default::default()
        };
        player.update(&jump, &world, FIXED_DT);
        assert!(!player.on_ground);
        assert!(player.vel.y < 0.0, "после нажатия игрок должен лететь вверх");

        let mut peak = player.pos.y;
        let held = PlayerInput {
            jump_held: true,
            ..Default::default()
        };
        for _ in 0..240 {
            player.update(&held, &world, FIXED_DT);
            peak = peak.min(player.pos.y);
        }
        let rise = spawn.y - peak;
        assert!(rise > 40.0, "прыжок слишком низкий: {rise}");
        assert!(rise < 160.0, "прыжок слишком высокий: {rise}");
        assert!(player.on_ground, "игрок должен приземлиться");
    }

    #[test]
    fn variable_jump_height_shorter_when_released() {
        let (world, spawn) = flat_world();

        let mut held_player = Player::new(spawn);
        let mut tap_player = Player::new(spawn);
        let idle = PlayerInput::default();
        for _ in 0..30 {
            held_player.update(&idle, &world, FIXED_DT);
            tap_player.update(&idle, &world, FIXED_DT);
        }

        let press = PlayerInput {
            jump_pressed: true,
            jump_held: true,
            ..Default::default()
        };
        held_player.update(&press, &world, FIXED_DT);
        tap_player.update(&press, &world, FIXED_DT);

        // Через 3 кадра «тап» отпускает кнопку, «удержание» держит до вершины.
        let held = PlayerInput {
            jump_held: true,
            ..Default::default()
        };
        let release = PlayerInput::default();
        for _ in 0..3 {
            held_player.update(&held, &world, FIXED_DT);
            tap_player.update(&release, &world, FIXED_DT);
        }

        let mut held_peak = held_player.pos.y;
        let mut tap_peak = tap_player.pos.y;
        for _ in 0..240 {
            held_player.update(&held, &world, FIXED_DT);
            held_peak = held_peak.min(held_player.pos.y);
            tap_player.update(&release, &world, FIXED_DT);
            tap_peak = tap_peak.min(tap_player.pos.y);
        }

        let held_rise = spawn.y - held_peak;
        let tap_rise = spawn.y - tap_peak;
        assert!(
            tap_rise < held_rise,
            "тап должен прыгать ниже: tap={tap_rise}, held={held_rise}"
        );
        assert!(held_rise > tap_rise + 10.0, "разница должна быть ощутимой");
    }

    #[test]
    fn coyote_time_allows_jump_after_leaving_ledge() {
        let mut world = World::new();
        // Широкая платформа: игрок сходит с правого края шагом.
        world.add_solid(Aabb::new(-100.0, 300.0, 196.0, 64.0));
        let spawn = Vec2::new(0.0, 300.0 - PLAYER_HEIGHT);
        let mut player = Player::new(spawn);
        let idle = PlayerInput::default();
        for _ in 0..30 {
            player.update(&idle, &world, FIXED_DT);
        }
        assert!(player.on_ground);

        // Шагает вправо, пока не сойдёт с края.
        let walk = PlayerInput {
            move_x: 1.0,
            ..Default::default()
        };
        let mut frames = 0;
        while player.on_ground && frames < 120 {
            player.update(&walk, &world, FIXED_DT);
            frames += 1;
        }
        assert!(!player.on_ground, "игрок должен сойти с края платформы");

        // Сразу после схода Coyote Time ещё действует — прыжок обязан сработать.
        let jump = PlayerInput {
            jump_pressed: true,
            jump_held: true,
            ..Default::default()
        };
        player.update(&jump, &world, FIXED_DT);
        assert!(
            player.vel.y < 0.0,
            "в пределах Coyote Time прыжок обязан сработать"
        );
    }
}
