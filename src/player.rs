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
    /// Кнопка атаки нажата в этом шаге (edge).
    pub attack_pressed: bool,
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
    /// Короткое оглушение после получения урона.
    Hurt,
    /// Удар мечом (3-х ударное комбо).
    Attack,
}

/// Длительность одного свинга атаки, с.
pub const ATTACK_TIME: f32 = 0.30;
/// Начало активных кадров (хитбокс включён), с.
pub const ATTACK_ACTIVE_START: f32 = 0.07;
/// Конец активных кадров, с.
pub const ATTACK_ACTIVE_END: f32 = 0.17;
/// Окно комбо: нажатие с этого момента запускает следующий свинг, с.
pub const COMBO_WINDOW_START: f32 = 0.10;
/// Максимальная длина комбо.
pub const COMBO_MAX: u32 = 3;

/// Максимум очков здоровья («сердец» в HUD).
pub const PLAYER_MAX_HP: u32 = 5;
/// Кадры неуязвимости после получения урона, с.
pub const PLAYER_IFRAMES: f32 = 1.0;
/// Потеря управления после удара (отброс), с.
pub const HURT_LOCK_TIME: f32 = 0.32;
/// Сила отброса при получении урона.
const HURT_KNOCK_X: f32 = 300.0;
const HURT_KNOCK_Y: f32 = 330.0;
/// Шаг вперёд при свинге (затухает к концу удара).
const ATTACK_STEP_SPEED: f32 = 95.0;

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
    /// Текущее здоровье (очки).
    pub hp: u32,
    /// Максимум здоровья.
    pub max_hp: u32,

    // --- внутренние таймеры ---
    /// i-frames от полученного урона (не от рывка).
    iframe_timer: f32,
    /// Потеря управления после удара.
    hurt_timer: f32,
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

    // --- боёвка ---
    /// Идёт ли свинг прямо сейчас.
    attacking: bool,
    /// Текущий свинг отсчитывается от нуля.
    attack_timer: f32,
    /// Запланирован ли следующий удар комбо (нажатие в окне).
    attack_queued: bool,
    /// Номер текущего свинга в комбо (1..=3).
    combo_index: u32,
    /// Номер свинга: растёт при каждом ударе — враги помнят, куда били.
    swing_id: u32,
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
            hp: PLAYER_MAX_HP,
            max_hp: PLAYER_MAX_HP,
            iframe_timer: 0.0,
            hurt_timer: 0.0,
            coyote_timer: 0.0,
            jump_buffer: 0.0,
            dash_timer: 0.0,
            dash_cooldown: 0.0,
            wall_jump_lock: 0.0,
            wall_regrab_delay: 0.0,
            air_dash_used: false,
            dashing: false,
            wall_sliding: false,
            attacking: false,
            attack_timer: 0.0,
            attack_queued: false,
            combo_index: 0,
            swing_id: 0,
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

    /// Есть ли кадры неуязвимости от полученного урона (мигание при рендере).
    pub fn is_hurt_flashing(&self) -> bool {
        self.iframe_timer > 0.0
    }

    /// Неуязвим ли сейчас: i-frames после удара или кадры рывка.
    pub fn is_invulnerable(&self) -> bool {
        self.iframe_timer > 0.0 || self.has_dash_iframes()
    }

    /// Идёт ли свинг атаки.
    pub fn is_attacking(&self) -> bool {
        self.attacking
    }

    /// Номер свинга (для учёта попаданий: один свинг — один удар по врагу).
    pub fn swing_id(&self) -> u32 {
        self.swing_id
    }

    /// Прогресс текущего свинга: `0.0 ..= 1.0` по [`ATTACK_TIME`].
    pub fn attack_progress(&self) -> f32 {
        if self.attacking {
            (self.attack_timer / ATTACK_TIME).clamp(0.0, 1.0)
        } else {
            0.0
        }
    }

    /// Нормализованный момент активных кадров `0.0 ..= 1.0`,
    /// `None` — хитбокс выключен.
    pub fn attack_active_t(&self) -> Option<f32> {
        if !self.attacking
            || self.attack_timer < ATTACK_ACTIVE_START
            || self.attack_timer > ATTACK_ACTIVE_END
        {
            return None;
        }
        Some(
            (self.attack_timer - ATTACK_ACTIVE_START)
                / (ATTACK_ACTIVE_END - ATTACK_ACTIVE_START),
        )
    }

    /// Есть ли сейчас хитбокс удара.
    pub fn is_attack_active(&self) -> bool {
        self.attack_active_t().is_some()
    }

    /// Хитбокс активных кадров: перед персонажом.
    pub fn attack_hitbox(&self) -> Option<Aabb> {
        self.attack_active_t()?;
        let reach = 34.0;
        let x = if self.facing >= 0.0 {
            self.pos.x + self.size.x - 4.0
        } else {
            self.pos.x - reach + 4.0
        };
        Some(Aabb::new(x, self.pos.y + 6.0, reach, self.size.y - 14.0))
    }

    /// Урон текущего свинга: 1-й и 2-й удар — 1, финальный — 2.
    pub fn attack_damage(&self) -> u32 {
        match self.combo_index {
            1 => 1,
            2 => 1,
            _ => 2,
        }
    }

    /// Начать первый свинг комбо.
    fn start_attack(&mut self) {
        self.attacking = true;
        self.attack_timer = 0.0;
        self.attack_queued = false;
        self.combo_index = 1;
        self.swing_id = self.swing_id.wrapping_add(1);
    }

    /// Отменить атаку (рывок, получение урона) с инвалидацией хитбокса.
    fn cancel_attack(&mut self) {
        if self.attacking || self.combo_index != 0 {
            self.attacking = false;
            self.attack_queued = false;
            self.combo_index = 0;
            self.swing_id = self.swing_id.wrapping_add(1);
        }
    }

    /// Получение урона: отбрасывание, i-frames и короткая потеря
    /// управления. `knock_dir` — направление отброса (`-1` — влево).
    /// Возвращает `true`, если урон применился (иначе — i-frames/смерть).
    pub fn take_damage(&mut self, amount: u32, knock_dir: f32) -> bool {
        if self.hp == 0 || self.is_invulnerable() {
            return false;
        }
        self.hp = self.hp.saturating_sub(amount);
        self.iframe_timer = PLAYER_IFRAMES;
        self.hurt_timer = HURT_LOCK_TIME;
        self.dashing = false;
        self.cancel_attack();
        self.vel = Vec2::new(knock_dir * HURT_KNOCK_X, -HURT_KNOCK_Y);
        self.state = PlayerState::Hurt;
        self.state_time = 0.0;
        true
    }

    /// Полный шаг симуляции игрока на фиксированном dt.
    pub fn update(&mut self, input: &PlayerInput, world: &World, dt: f32) {
        debug_assert!(dt > 0.0, "шаг симуляции должен быть положительным");

        // --- таймеры ---
        self.state_time += dt;
        self.iframe_timer -= dt;
        self.hurt_timer -= dt;
        self.coyote_timer -= dt;
        self.jump_buffer -= dt;
        self.dash_cooldown -= dt;
        self.wall_jump_lock -= dt;
        self.wall_regrab_delay -= dt;
        self.wall_sliding = false;

        // После удара управление отключено на HURT_LOCK_TIME — тело
        // только летит по инерции и падает под гравитацией.
        let stunned = self.hurt_timer > 0.0;

        // Старт удара: только вне оглушения, рывка и другого свинга.
        if input.attack_pressed && !stunned && !self.dashing && !self.attacking {
            self.start_attack();
        }

        let move_x = if stunned {
            0.0
        } else {
            input.move_x.clamp(-1.0, 1.0)
        };

        // --- проба стен (до логики прыжка/скольжения) ---
        self.wall_dir = self.probe_wall(world);

        if stunned {
            // ---------- ОГЛУШЕНИЕ (отброс после урона) ----------
            apply_gravity(&mut self.vel, dt, GRAVITY, MAX_FALL_SPEED);
        } else if self.dashing {
            // ---------- DASH ----------
            self.dash_timer -= dt;
            self.vel.x = self.facing * DASH_SPEED;
            self.vel.y = 0.0;
            if self.dash_timer <= 0.0 {
                self.dashing = false;
                self.vel.x = self.facing * RUN_SPEED * DASH_CARRY_SPEED;
            }
        } else if input.dash_pressed && self.dash_cooldown <= 0.0 && !self.air_dash_used {
            // ---------- СТАРТ DASH (рывок отменяет свинг) ----------
            if move_x.abs() > 0.1 {
                self.facing = move_x.signum();
            }
            self.cancel_attack();
            self.dashing = true;
            self.dash_timer = DASH_TIME;
            self.dash_cooldown = DASH_COOLDOWN;
            if !self.on_ground {
                self.air_dash_used = true;
            }
        } else if self.attacking {
            // ---------- АТАКА МЕЧОМ ----------
            self.attack_timer += dt;
            if self.on_ground {
                // Затухающий шаг вперёд — свинг «втягивает» тело.
                let step = ATTACK_STEP_SPEED * (1.0 - (self.attack_timer / ATTACK_TIME).min(1.0));
                self.vel.x = self.facing * step;
            } else {
                // В воздухе управление отдано инерции удара.
                self.vel.x = move_toward(self.vel.x, 0.0, AIR_DECEL * dt);
            }
            apply_gravity(&mut self.vel, dt, GRAVITY, MAX_FALL_SPEED);

            // Окно комбо: нажатие запускает следующий свинг.
            if input.attack_pressed && self.attack_timer >= COMBO_WINDOW_START {
                self.attack_queued = true;
            }
            if self.attack_timer >= ATTACK_TIME {
                if self.attack_queued && self.combo_index < COMBO_MAX {
                    self.combo_index += 1;
                    self.attack_timer = 0.0;
                    self.attack_queued = false;
                    self.swing_id = self.swing_id.wrapping_add(1);
                } else {
                    self.cancel_attack();
                }
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
        let new_state = if stunned {
            PlayerState::Hurt
        } else if self.dashing {
            PlayerState::Dash
        } else if self.attacking {
            PlayerState::Attack
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

    #[test]
    fn take_damage_knockback_and_iframes() {
        let (world, spawn) = flat_world();
        let mut player = Player::new(spawn);
        let idle = PlayerInput::default();

        assert_eq!(player.hp, PLAYER_MAX_HP);
        assert!(player.take_damage(1, -1.0));
        assert_eq!(player.hp, PLAYER_MAX_HP - 1);
        assert_eq!(player.state, PlayerState::Hurt);
        assert!(player.vel.x < 0.0, "отброс в сторону удара");
        assert!(player.vel.y < 0.0, "подброс вверх");
        assert!(player.is_invulnerable());
        assert!(player.is_hurt_flashing());

        // Второй удар в кадр i-frames не проходит.
        assert!(!player.take_damage(1, 1.0));
        assert_eq!(player.hp, PLAYER_MAX_HP - 1);

        // Через i-frames и время потери управления — полный контроль.
        let steps = ((PLAYER_IFRAMES + HURT_LOCK_TIME + 0.1) / FIXED_DT) as u32;
        for _ in 0..steps {
            player.update(&idle, &world, FIXED_DT);
        }
        assert!(!player.is_invulnerable());
        assert_ne!(player.state, PlayerState::Hurt);
        assert!(player.take_damage(1, 1.0), "после i-frames урон проходит");
    }

    #[test]
    fn respawn_restores_full_hp() {
        let (_world, spawn) = flat_world();
        let mut player = Player::new(spawn);
        assert!(player.take_damage(3, -1.0));
        assert!(player.hp < player.max_hp);

        player.respawn(spawn);
        assert_eq!(player.hp, player.max_hp);
        assert_eq!(player.state, PlayerState::Idle);
        assert!(!player.is_invulnerable());
    }

    /// Хитбокс включается только в активных кадрах свинга.
    #[test]
    fn attack_active_window_and_hitbox() {
        let (world, spawn) = flat_world();
        let mut player = Player::new(spawn);
        let idle = PlayerInput::default();
        for _ in 0..30 {
            player.update(&idle, &world, FIXED_DT);
        }

        let atk = PlayerInput {
            attack_pressed: true,
            ..Default::default()
        };
        player.update(&atk, &world, FIXED_DT);
        assert!(player.is_attacking());
        assert_eq!(player.state, PlayerState::Attack);
        assert!(!player.is_attack_active(), "до активных кадров — нет хитбокса");
        assert_eq!(player.attack_damage(), 1);

        // Кадр ~0.10 с — активное окно (0.07..=0.17) открыто.
        for _ in 0..5 {
            player.update(&idle, &world, FIXED_DT);
        }
        assert!(player.is_attack_active(), "окно активных кадров");
        let hitbox = player.attack_hitbox().expect("хитбокс активных кадров");
        assert!(hitbox.overlaps(&player.body()));

        // Свинг завершается по ATTACK_TIME и снимает состояние.
        for _ in 0..16 {
            player.update(&idle, &world, FIXED_DT);
        }
        assert!(!player.is_attacking(), "свинг должен завершиться");
        assert_ne!(player.state, PlayerState::Attack);
    }

    /// Нажатие в окне комбо запускает следующий свинг (до 3 ударов).
    #[test]
    fn combo_chains_up_to_three_swings() {
        let (world, spawn) = flat_world();
        let mut player = Player::new(spawn);
        let idle = PlayerInput::default();
        for _ in 0..30 {
            player.update(&idle, &world, FIXED_DT);
        }

        let atk = PlayerInput {
            attack_pressed: true,
            ..Default::default()
        };
        player.update(&atk, &world, FIXED_DT);
        let first_swing = player.swing_id();
        assert_eq!(player.combo_index, 1);

        // В окне комбо (>= 0.10 с) жмём атаку снова.
        for _ in 0..6 {
            player.update(&idle, &world, FIXED_DT);
        }
        player.update(&atk, &world, FIXED_DT);
        // Ждём конца свинга — буферизированное нажатие разворачивает второй.
        for _ in 0..12 {
            player.update(&idle, &world, FIXED_DT);
        }
        assert!(player.is_attacking(), "второй свинг должен начаться");
        assert_ne!(player.swing_id(), first_swing, "номер свинга изменился");
        assert_eq!(player.combo_index, 2);
        assert_eq!(player.attack_damage(), 1);

        // Без нажатий комбо обрывается после второго свинга.
        for _ in 0..24 {
            player.update(&idle, &world, FIXED_DT);
        }
        assert!(!player.is_attacking(), "без нажатия комбо не продолжается");
    }
}
