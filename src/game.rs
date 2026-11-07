//! Главный игровой цикл: фиксированный шаг 60 Гц, сборка демо-уровня,
//! камера, многослойный параллакс, факелы и HUD.
//!
//! Модули врагов, генерации уровней, частиц и тач-интерфейса подключаются
//! следующими этапами и расширяют эту сцену, не меняя её контракт:
//! [`run`] владеет циклом, физика считается только на [`FIXED_DT`].

use macroquad::prelude::*;

use crate::enemies::{draw_enemies, EnemyKind, EnemyWorld};
use crate::physics::{Aabb, World, FIXED_DT};
use crate::player::{Player, PlayerInput, PlayerState};

/// Размер тайла, px.
pub const TILE: f32 = 32.0;
/// Экранных пикселей на игровой пиксель (пиксель-арт без сглаживания).
const PPU: f32 = 3.0;

// ---------------------------------------------------------------------------
// Палитра тёмного фэнтези: камень, мох, ржавое железо, тусклое золото.
// Кислотные и неоновые цвета исключены.
// ---------------------------------------------------------------------------
const CLEAR_COLOR: Color = Color::new(0.055, 0.043, 0.035, 1.0);
const STONE: [Color; 3] = [
    Color::new(0.290, 0.267, 0.243, 1.0),
    Color::new(0.259, 0.235, 0.216, 1.0),
    Color::new(0.227, 0.208, 0.188, 1.0),
];
const STONE_DARK: Color = Color::new(0.169, 0.153, 0.137, 1.0);
const MOSS: Color = Color::new(0.267, 0.318, 0.204, 1.0);
const DIRT: Color = Color::new(0.227, 0.180, 0.141, 1.0);
const IRON: Color = Color::new(0.184, 0.173, 0.161, 1.0);
const FLAME: Color = Color::new(0.851, 0.643, 0.255, 1.0);
const FLAME_CORE: Color = Color::new(0.957, 0.839, 0.541, 1.0);
const STEEL: Color = Color::new(0.659, 0.627, 0.592, 1.0);
const STEEL_DARK: Color = Color::new(0.431, 0.408, 0.380, 1.0);
const CLOAK: Color = Color::new(0.478, 0.227, 0.196, 1.0);
const VISOR: Color = Color::new(0.078, 0.067, 0.059, 1.0);
const GOLD: Color = Color::new(0.769, 0.627, 0.290, 1.0);
const TEXT_DIM: Color = Color::new(0.549, 0.510, 0.471, 1.0);
const TEXT_MAIN: Color = Color::new(0.847, 0.812, 0.769, 1.0);
const HEART_FILL: Color = Color::new(0.549, 0.161, 0.129, 1.0);
const HEART_SHINE: Color = Color::new(0.706, 0.294, 0.235, 1.0);
const HEART_EMPTY: Color = Color::new(0.278, 0.227, 0.208, 1.0);

/// Верхняя строка пола (мировые координаты) — линия горизонта для параллакса.
const HORIZON: f32 = 14.0 * TILE;

/// Точка возрождения.
fn spawn_point() -> Vec2 {
    Vec2::new(2.0 * TILE, HORIZON - crate::player::PLAYER_HEIGHT)
}

/// Запасной потолок: ниже — падение в пустоту и респавн.
const VOID_Y: f32 = 30.0 * TILE;

/// Основной цикл игры. Блокирует поток до выхода (на Android поток
/// запускается из `quad_main` и рендер-цикл miniquad работает параллельно).
pub async fn run() {
    let mut game = Game::new();
    let mut accumulator = 0.0f32;

    loop {
        let frame_dt = get_frame_time().min(0.25);
        game.sample_edges();

        accumulator += frame_dt;
        // Несколько физических шагов на «тяжёлый» кадр, ноль — на лёгкий:
        // симуляция всегда идёт с FIXED_DT независимо от частоты кадров.
        while accumulator >= FIXED_DT {
            game.fixed_update();
            accumulator -= FIXED_DT;
        }

        game.draw(frame_dt);
        next_frame().await;
    }
}

struct Game {
    world: World,
    player: Player,
    enemies: EnemyWorld,
    cam_pos: Vec2,
    time: f32,
    fps_smooth: f32,
    /// Захваченные edge-нажатия: живут до следующего фиксированного шага,
    /// чтобы пропущенные кадры не теряли нажатия.
    jump_edge: bool,
    dash_edge: bool,
}

impl Game {
    fn new() -> Self {
        let level = build_level();
        let mut enemies = EnemyWorld::new();
        for (kind, pos) in level.enemy_spawns {
            enemies.spawn(kind, pos);
        }
        Self {
            world: level.world,
            player: Player::new(level.spawn),
            enemies,
            cam_pos: spawn_point() + Vec2::new(0.0, -40.0),
            time: 0.0,
            fps_smooth: 60.0,
            jump_edge: false,
            dash_edge: false,
        }
    }

    /// Захват edge-нажатий с клавиатуры (вызывается каждый кадр отрисовки).
    fn sample_edges(&mut self) {
        if is_key_pressed(KeyCode::Space)
            || is_key_pressed(KeyCode::W)
            || is_key_pressed(KeyCode::Up)
        {
            self.jump_edge = true;
        }
        if is_key_pressed(KeyCode::LeftShift) || is_key_pressed(KeyCode::X) {
            self.dash_edge = true;
        }
    }

    /// Один фиксированный шаг симуляции (1/60 с).
    fn fixed_update(&mut self) {
        let mut move_x = 0.0;
        if is_key_down(KeyCode::A) || is_key_down(KeyCode::Left) {
            move_x -= 1.0;
        }
        if is_key_down(KeyCode::D) || is_key_down(KeyCode::Right) {
            move_x += 1.0;
        }
        let jump_held =
            is_key_down(KeyCode::Space) || is_key_down(KeyCode::W) || is_key_down(KeyCode::Up);

        let input = PlayerInput {
            move_x,
            jump_pressed: self.jump_edge,
            jump_held,
            dash_pressed: self.dash_edge,
        };
        // Edge тратится первым же фиксированным шагом.
        self.jump_edge = false;
        self.dash_edge = false;

        self.player.update(&input, &self.world, FIXED_DT);

        // Шаг ИИ врагов; возвращаются контакты, задевшие игроку.
        let hits = self.enemies.update(&self.world, &self.player, FIXED_DT);
        for hit in hits {
            let player_center = self.player.body().center();
            let knock_dir = if hit.center.x >= player_center.x {
                -1.0
            } else {
                1.0
            };
            self.player.take_damage(hit.damage, knock_dir);
        }

        self.time += FIXED_DT;

        // Падение в пустоту или смерть — возврат на чекпоинт с полным HP.
        if self.player.pos.y > VOID_Y || self.player.hp == 0 {
            self.player.respawn(spawn_point());
        }
    }

    fn draw(&mut self, frame_dt: f32) {
        clear_background(CLEAR_COLOR);

        // Камера догоняет игрока по реальному времени кадра — движение
        // остаётся плавным даже при нескольких физических шагах за кадр.
        let target = self.player.pos + self.player.size * 0.5;
        let follow = 1.0 - (-10.0 * frame_dt).exp();
        self.cam_pos += (target - self.cam_pos) * follow;

        let view = Vec2::new(screen_width() / PPU, screen_height() / PPU);
        // Камера с осью Y вниз (как у проекции по умолчанию): показывает ровно
        // `view` мировых единиц, центр — на игроке. zoom.y положителен,
        // потому что внутри matrix() он умножается на invert_y = -1.
        let camera = Camera2D {
            target: self.cam_pos,
            zoom: vec2(2.0 / view.x, 2.0 / view.y),
            offset: Vec2::ZERO,
            rotation: 0.0,
            ..Default::default()
        };
        set_camera(&camera);

        draw_parallax(&self.cam_pos, &view);
        draw_tiles(&self.world, &self.cam_pos, &view);
        draw_torches(self.time);
        draw_enemies(&self.enemies, self.time);
        draw_player(&self.player, self.time);

        set_default_camera();
        self.draw_hud(frame_dt);
    }

    fn draw_hud(&mut self, frame_dt: f32) {
        if frame_dt > f32::EPSILON {
            let fps = 1.0 / frame_dt;
            self.fps_smooth += (fps - self.fps_smooth) * 0.08;
        }

        draw_text(
            &format!("FPS {:.0}", self.fps_smooth),
            12.0,
            26.0,
            20.0,
            TEXT_DIM,
        );
        let state = match self.player.state {
            PlayerState::Idle => "IDLE",
            PlayerState::Run => "RUN",
            PlayerState::Jump => "JUMP",
            PlayerState::Fall => "FALL",
            PlayerState::WallSlide => "WALL SLIDE",
            PlayerState::Dash => "DASH",
            PlayerState::Hurt => "HURT",
        };
        draw_text(
            &format!("STATE: {state}"),
            12.0,
            50.0,
            20.0,
            TEXT_DIM,
        );

        // Здоровье: ряд «сердец»-квадратов (пиксельный стиль, ASCII-шрифт
        // не содержит глифов сердец).
        for i in 0..self.player.max_hp {
            let hx = 12.0 + i as f32 * 18.0;
            let hy = 70.0;
            if i < self.player.hp {
                draw_rectangle(hx, hy, 14.0, 14.0, HEART_FILL);
                draw_rectangle(hx + 2.0, hy + 2.0, 4.0, 4.0, HEART_SHINE);
            } else {
                draw_rectangle_lines(hx, hy, 14.0, 14.0, 1.0, HEART_EMPTY);
            }
        }
        // Шрифт по умолчанию в macroquad — только ASCII, поэтому HUD на латинице.
        draw_text(
            "A/D - move   SPACE - jump   SHIFT - dash",
            12.0,
            screen_height() - 26.0,
            20.0,
            TEXT_DIM,
        );
        draw_text(
            "VOXEL KNIGHT",
            12.0,
            screen_height() - 54.0,
            22.0,
            TEXT_MAIN,
        );
    }
}

// ---------------------------------------------------------------------------
// Уровень
// ---------------------------------------------------------------------------

/// Заполнить блок тайлов (один прямоугольник вместо сотни — быстрый broad-phase).
fn fill(world: &mut World, tx: i32, ty: i32, w: i32, h: i32) {
    world.add_solid(Aabb::new(
        tx as f32 * TILE,
        ty as f32 * TILE,
        w as f32 * TILE,
        h as f32 * TILE,
    ));
}

/// Готовый уровень: геометрия, точка спавна игрока и расстановка врагов.
struct Level {
    world: World,
    spawn: Vec2,
    enemy_spawns: Vec<(EnemyKind, Vec2)>,
}

/// Демо-уровень ядра: пол с ямой (Coyote Time), платформы, стена для
/// Wall Slide и первые враги (FSM). Генерация уровней (TMX/JSON)
/// подключается отдельным модулем.
fn build_level() -> Level {
    let mut world = World::new();

    // Пол слева от ямы и справа.
    fill(&mut world, -8, 14, 28, 2);
    fill(&mut world, 23, 14, 41, 2);
    // Дно ямы:2 ряда глубиной — из неё можно выпрыгнуть.
    fill(&mut world, 20, 16, 3, 2);

    // Платформы (подъём по 64 px — в пределах высоты прыжка ~85 px).
    fill(&mut world, 6, 12, 5, 1);
    fill(&mut world, 13, 10, 4, 1);
    fill(&mut world, 40, 12, 6, 1);

    // Высокая стена для Wall Slide / Wall Jump.
    fill(&mut world, 28, 7, 1, 7);

    // Границы уровня.
    fill(&mut world, -8, 6, 1, 8);
    fill(&mut world, 60, 6, 4, 8);

    // Расстановка врагов (левый верхний угол хитбокса):
    // два слизня на полу, скелет у правой стены, летучая мышь над ямой.
    let floor_top = 14.0 * TILE;
    let enemy_spawns = vec![
        (EnemyKind::Slime, Vec2::new(10.0 * TILE, floor_top - 20.0)),
        (EnemyKind::Slime, Vec2::new(36.0 * TILE, floor_top - 20.0)),
        (EnemyKind::Skeleton, Vec2::new(44.0 * TILE, floor_top - 38.0)),
        (EnemyKind::Bat, Vec2::new(21.5 * TILE, 9.0 * TILE)),
    ];

    Level {
        world,
        spawn: spawn_point(),
        enemy_spawns,
    }
}

// ---------------------------------------------------------------------------
// Отрисовка
// ---------------------------------------------------------------------------

/// Детерминированный «шум» для вариаций оттенков: [-1, 1] по индексу.
fn hash_noise(i: i32) -> f32 {
    let v = (i as f32 * 12.9898).sin() * 43758.5453;
    (v.fract() * 2.0) - 1.0
}

/// Три слоя параллакса: дальние руины, средние скалы, ближние контуры.
fn draw_parallax(cam: &Vec2, view: &Vec2) {
    struct Layer {
        factor: f32,
        color: Color,
        span: f32,
        min_h: f32,
        max_h: f32,
        seed: i32,
    }
    let layers = [
        Layer {
            factor: 0.2,
            color: Color::new(0.118, 0.098, 0.082, 1.0),
            span: 240.0,
            min_h: 90.0,
            max_h: 170.0,
            seed: 11,
        },
        Layer {
            factor: 0.45,
            color: Color::new(0.145, 0.125, 0.106, 1.0),
            span: 180.0,
            min_h: 70.0,
            max_h: 130.0,
            seed: 47,
        },
        Layer {
            factor: 0.7,
            color: Color::new(0.176, 0.153, 0.133, 1.0),
            span: 140.0,
            min_h: 50.0,
            max_h: 95.0,
            seed: 83,
        },
    ];

    for layer in &layers {
        // Слой движется медленнее камеры: сдвигаем рисунок в мировых координатах.
        let shift = cam.x * (1.0 - layer.factor);
        let left = cam.x - view.x * 0.5;
        let right = cam.x + view.x * 0.5;
        let k_start = (((left + shift) / layer.span).floor() as i32) - 1;
        let k_end = (((right + shift) / layer.span).ceil() as i32) + 1;

        for k in k_start..=k_end {
            let x = k as f32 * layer.span - shift;
            let n = hash_noise(k + layer.seed * 1013);
            let h = layer.min_h + (n + 1.0) * 0.5 * (layer.max_h - layer.min_h);
            // Силуэт из двух ступеней — очертания руин/скал.
            let w_top = layer.span * 0.30;
            let w_base = layer.span * 0.62;
            draw_rectangle(x, HORIZON - h, w_base, h, layer.color);
            let h_top = h * 0.45;
            draw_rectangle(
                x + (w_base - w_top) * 0.5,
                HORIZON - h - h_top,
                w_top,
                h_top + 1.0,
                layer.color,
            );
        }
    }
}

/// Тайловая отрисовка: камень с вариацией оттенков, мох на открытых верхах,
/// тёмные швы между тайлами.
fn draw_tiles(world: &World, cam: &Vec2, view: &Vec2) {
    let visible = Aabb::new(
        cam.x - view.x * 0.5 - TILE,
        cam.y - view.y * 0.5 - TILE,
        view.x + TILE * 2.0,
        view.y + TILE * 2.0,
    );

    for solid in world.query(visible) {
        let tx0 = (solid.left() / TILE).floor() as i32;
        let tx1 = (solid.right() / TILE).ceil() as i32;
        let ty0 = (solid.top() / TILE).floor() as i32;
        let ty1 = (solid.bottom() / TILE).ceil() as i32;

        for ty in ty0..ty1 {
            for tx in tx0..tx1 {
                let x = tx as f32 * TILE;
                let y = ty as f32 * TILE;

                let shade = ((hash_noise(tx * 73 + ty * 131) + 1.0) * 0.5 * 3.0) as usize;
                draw_rectangle(x, y, TILE, TILE, STONE[shade.min(2)]);

                // Верхний край открыт для воздуха — мох и трава.
                let above = Aabb::new(x, y - 1.0, TILE, 1.0);
                if !world.overlaps_any(above) {
                    draw_rectangle(x, y, TILE, 7.0, MOSS);
                    draw_rectangle(x, y + 7.0, TILE, 4.0, DIRT);
                }

                // Тёмный шов по периметру тайла — чёткая пиксельная сетка.
                draw_rectangle_lines(x, y, TILE, TILE, 1.0, STONE_DARK);
            }
        }
    }
}

/// Факелы: железный держатель, живой огонь и мягкий тёплый свет.
fn draw_torches(time: f32) {
    // Каждый факел висит на вертикальной грани блока (см. build_level).
    const TORCHES: [(f32, f32); 6] = [
        (28.0 * TILE - 6.0, 7.5 * TILE),  // стена wall-slide, верх
        (28.0 * TILE - 6.0, 12.0 * TILE), // стена wall-slide, низ
        (60.0 * TILE - 6.0, 8.0 * TILE),  // правая граница
        (-7.0 * TILE, 7.0 * TILE),        // левая граница
        (6.0 * TILE - 6.0, 12.5 * TILE),  // платформа
        (20.0 * TILE, 14.75 * TILE),      // стенка ямы
    ];

    for (i, (x, y)) in TORCHES.into_iter().enumerate() {
        let flicker = hash_noise(i as i32 * 17 + (time * 8.0) as i32);
        let pulse = 1.0 + flicker * 0.12;

        // Мягкое свечение — два концентрических круга с низкой альфой.
        draw_circle(x + 3.0, y, 46.0 * pulse, Color::new(0.851, 0.643, 0.255, 0.055));
        draw_circle(x + 3.0, y, 26.0 * pulse, Color::new(0.851, 0.643, 0.255, 0.085));

        // Держатель.
        draw_rectangle(x, y - 2.0, 6.0, 16.0, IRON);
        draw_rectangle(x - 2.0, y + 12.0, 10.0, 4.0, IRON);

        // Пламя.
        let flame_h = 12.0 * pulse;
        draw_rectangle(x + 1.0, y - flame_h, 4.0, flame_h, FLAME);
        draw_circle(x + 3.0, y - flame_h + 2.0, 3.0, FLAME_CORE);
    }
}

/// Рыцарь из прямоугольников: поза зависит от состояния, взгляд — от `facing`.
fn draw_player(p: &Player, time: f32) {
    // Мигание при i-frames после получения урона.
    if p.is_hurt_flashing() && ((time * 20.0) as i32) % 2 == 0 {
        return;
    }

    let x = p.pos.x;
    let y = p.pos.y;
    let w = p.size.x;
    let h = p.size.y;
    let facing_right = p.facing >= 0.0;

    // Зеркалирование: локальные координаты (0..w) → мировые.
    let fx = |lx: f32, ew: f32| -> f32 {
        if facing_right {
            x + lx
        } else {
            x + w - lx - ew
        }
    };

    // Тень на земле.
    if p.on_ground {
        draw_rectangle(x + 2.0, y + h - 1.0, w - 4.0, 3.0, Color::new(0.0, 0.0, 0.0, 0.35));
    }

    // Шлейф рывка.
    if p.state == PlayerState::Dash {
        let dir = p.facing;
        for i in 0..3 {
            let alpha = 110.0 - i as f32 * 35.0;
            let gx = x - dir * (8.0 + i as f32 * 9.0);
            draw_rectangle(
                gx + 4.0,
                y + 12.0,
                w - 8.0,
                h - 22.0,
                Color::new(0.659, 0.627, 0.592, alpha / 255.0),
            );
        }
    }

    let mut bob = 0.0f32;
    let mut lean = 0.0f32;
    let mut crouch = 0.0f32;
    let mut leg_phase = 0.0f32;
    match p.state {
        PlayerState::Idle => {
            bob = (time * 2.4).sin() * 0.8;
        }
        PlayerState::Run => {
            lean = 1.5 * p.facing;
            leg_phase = (p.state_time * 14.0).sin();
        }
        PlayerState::Jump => {
            bob = -1.0;
            leg_phase = 0.5;
        }
        PlayerState::Fall => {
            leg_phase = -0.4;
        }
        PlayerState::WallSlide => {
            // Прижатие к стене: корпус чуть ниже, ноги подобраны.
            crouch = 3.0;
            leg_phase = 0.7;
        }
        PlayerState::Hurt => {
            // Отброс: тело напряжено, ноги в напряжённой стойке.
            crouch = 2.0;
            leg_phase = 0.6;
        }
        PlayerState::Dash => {
            crouch = 6.0;
            lean = 3.0 * p.facing;
            leg_phase = 1.0;
        }
    }

    let top = y + bob + crouch;

    // --- плащ (за корпусом) ---
    let sway = (time * 6.0).sin() * 1.5 - p.vel.x * 0.012;
    let cloak_w = 9.0;
    let cloak_lx = if facing_right { 2.0 + sway } else { w - 11.0 - sway };
    draw_rectangle(
        fx(cloak_lx, cloak_w),
        top + 13.0,
        cloak_w,
        18.0 - crouch * 0.5,
        CLOAK,
    );

    // --- ноги ---
    let leg_y = top + h - 12.0;
    let swing = leg_phase * 4.5;
    let (leg1, leg2) = if p.state == PlayerState::Run {
        (4.0 + swing, 12.0 - swing)
    } else if p.state == PlayerState::WallSlide {
        // Ноги направлены в сторону стены.
        if facing_right {
            (6.0, 13.0)
        } else {
            (2.0, 9.0)
        }
    } else if p.state == PlayerState::Jump {
        (3.0, 11.0)
    } else if p.state == PlayerState::Fall {
        (2.0, 13.0)
    } else {
        (4.0, 12.0)
    };
    let leg_lift = (leg_phase.abs()) * 2.0;
    draw_rectangle(fx(leg1, 6.0), leg_y + if p.state == PlayerState::Run { leg_lift } else { 0.0 }, 6.0, 12.0, STEEL_DARK);
    draw_rectangle(fx(leg2, 6.0), leg_y - if p.state == PlayerState::Run { leg_lift * 0.5 } else { 0.0 }, 6.0, 12.0, STEEL_DARK);

    // --- корпус ---
    let torso_lx = 4.0 + lean;
    draw_rectangle(fx(torso_lx, 16.0), top + 12.0, 16.0, 14.0, STEEL);
    draw_rectangle(fx(torso_lx, 16.0), top + 22.0, 16.0, 4.0, STEEL_DARK);
    // Пояс — тусклое золото.
    draw_rectangle(fx(torso_lx + 2.0, 12.0), top + 21.0, 12.0, 2.0, GOLD);

    // --- шлем ---
    let helm_lx = 5.0 + lean;
    draw_rectangle(fx(helm_lx, 14.0), top + 1.0, 14.0, 12.0, STEEL);
    draw_rectangle(fx(helm_lx, 14.0), top + 1.0, 14.0, 3.0, STEEL_DARK);
    // Глазница смотрит в сторону движения.
    draw_rectangle(fx(helm_lx + 6.0, 7.0), top + 6.0, 7.0, 3.0, VISOR);
    // Гребень — ржаво-красный.
    draw_rectangle(fx(helm_lx + 4.0, 4.0), top - 1.0, 4.0, 3.0, CLOAK);
}
