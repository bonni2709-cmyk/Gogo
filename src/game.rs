//! Главный игровой цикл: фиксированный шаг 60 Гц, сборка демо-уровня,
//! камера, многослойный параллакс, факелы и HUD.
//!
//! Модули врагов, генерации уровней, частиц и тач-интерфейса подключаются
//! следующими этапами и расширяют эту сцену, не меняя её контракт:
//! [`run`] владеет циклом, физика считается только на [`FIXED_DT`].

use macroquad::prelude::*;

use crate::enemies::{death_palette, draw_enemies, EnemyKind, EnemyWorld};
use crate::particles::ParticlePool;
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

// --- расширенная палитра детализации (землистая, тёмное фэнтези) ---
/// Светлый скол камня (подсветка рёбер).
const STONE_HI: Color = Color::new(0.420, 0.400, 0.369, 1.0);
/// Глубокая тень камня (швы, изнанка).
const STONE_LO: Color = Color::new(0.125, 0.114, 0.102, 1.0);
/// Светлый мох.
const MOSS_HI: Color = Color::new(0.373, 0.435, 0.267, 1.0);
/// Тёмный мох (подлёток, тень).
const MOSS_LO: Color = Color::new(0.165, 0.208, 0.118, 1.0);
/// Тёмная земля под мхом.
const DIRT_LO: Color = Color::new(0.145, 0.114, 0.090, 1.0);
/// Глубокое пламя (внешний язык).
const FLAME_DEEP: Color = Color::new(0.706, 0.353, 0.129, 1.0);
/// Уголь/уголёк в воздухе.
const EMBER: Color = Color::new(0.902, 0.549, 0.204, 1.0);
/// Обводка силуэтов (почти чёрный тёплый).
const OUTLINE: Color = Color::new(0.055, 0.047, 0.039, 1.0);
/// Кожа ремня.
const LEATHER: Color = Color::new(0.310, 0.220, 0.149, 1.0);
/// Светлая сталь (края доспеха).
const STEEL_HI: Color = Color::new(0.804, 0.773, 0.722, 1.0);
/// Тёмная кожа/подкладка плаща.
const CLOAK_DARK: Color = Color::new(0.302, 0.137, 0.118, 1.0);
/// Туман на горизонте.
const FOG: Color = Color::new(0.360, 0.333, 0.302, 1.0);

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
    /// Захваченный edge атаки (клавиша или мышь) до фиксированного шага.
    attack_edge: bool,
    /// Предыдущее состояние мыши — для edge-нажатия атаки.
    attack_mouse_prev: bool,
    // --- боёвка: частицы, hitstop, учёт попаданий свинга ---
    particles: ParticlePool,
    /// Заморозка мира на ударе (частицы продолжают лететь), с.
    hitstop: f32,
    /// Кому уже навредил текущий свинг.
    swing_hits: Vec<crate::ecs::Entity>,
    /// Номер свинга, обработанный в `swing_hits`.
    last_swing: u32,
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
            attack_edge: false,
            attack_mouse_prev: false,
            particles: ParticlePool::new(),
            hitstop: 0.0,
            swing_hits: Vec::new(),
            last_swing: 0,
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
        // Атака: J / Z с клавиатуры либо edge левой кнопки мыши.
        if is_key_pressed(KeyCode::Z) || is_key_pressed(KeyCode::J) {
            self.attack_edge = true;
        }
        let mouse_atk = is_mouse_button_down(MouseButton::Left);
        if mouse_atk && !self.attack_mouse_prev {
            self.attack_edge = true;
        }
        self.attack_mouse_prev = mouse_atk;
    }

    /// Один фиксированный шаг симуляции (1/60 с).
    fn fixed_update(&mut self) {
        // HITSTOP: мир замирает на ударе, частицы продолжают лететь.
        // Edge-нажатия при этом копятся и тратятся после разморозки.
        if self.hitstop > 0.0 {
            self.hitstop -= FIXED_DT;
            self.particles.update(FIXED_DT);
            self.time += FIXED_DT;
            return;
        }

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
            attack_pressed: self.attack_edge,
        };
        // Edge тратится первым же фиксированным шагом.
        self.jump_edge = false;
        self.dash_edge = false;
        self.attack_edge = false;

        self.player.update(&input, &self.world, FIXED_DT);

        // ---------- УДАРЫ МЕЧОМ: хитбокс активных кадров ----------
        if self.player.is_attacking() && self.player.is_attack_active() {
            if self.player.swing_id() != self.last_swing {
                self.last_swing = self.player.swing_id();
                self.swing_hits.clear();
            }
            if let Some(hitbox) = self.player.attack_hitbox() {
                // Сначала собираем цели (entity + центр) — иначе borrow
                // врагов мешает нанесению урона.
                let mut targets = Vec::new();
                for (entity, transform, _, _) in self.enemies.iter() {
                    if !self.swing_hits.contains(&entity)
                        && transform.body().overlaps(&hitbox)
                    {
                        targets.push((entity, transform.center()));
                    }
                }
                let damage = self.player.attack_damage();
                for (entity, center) in targets {
                    self.swing_hits.push(entity);
                    // Отброс врага — от игрока.
                    let dir = if center.x >= self.player.body().center().x {
                        1.0
                    } else {
                        -1.0
                    };
                    if self.enemies.damage(entity, damage, dir) {
                        self.particles.hit_spark(center, dir);
                        // Контрольный hitstop: тяжёлый свинг замирает дольше.
                        self.hitstop = if damage >= 2 { 0.08 } else { 0.05 };
                    }
                }
            }
        }

        // ---------- ШАГ ВРАГОВ: смерть взрывается пикселями ----------
        let step = self.enemies.update(&self.world, &self.player, FIXED_DT);
        for (pos, kind) in step.deaths {
            self.particles.death_burst(pos, &death_palette(kind));
            self.hitstop = self.hitstop.max(0.05);
        }

        // Контактные попадания врагов в игрока (у игрока свои i-frames).
        for hit in step.contacts {
            let player_center = self.player.body().center();
            let knock_dir = if hit.center.x >= player_center.x {
                -1.0
            } else {
                1.0
            };
            self.player.take_damage(hit.damage, knock_dir);
        }

        self.particles.update(FIXED_DT);
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

        draw_parallax(&self.cam_pos, &view, self.time);
        draw_tiles(&self.world, &self.cam_pos, &view);
        draw_torches(self.time);
        draw_motes(&self.cam_pos, &view, self.time);
        draw_enemies(&self.enemies, self.time);
        draw_player(&self.player, self.time);
        self.particles.draw();

        set_default_camera();
        self.draw_hud(frame_dt);
    }

    fn draw_hud(&mut self, frame_dt: f32) {
        if frame_dt > f32::EPSILON {
            let fps = 1.0 / frame_dt;
            self.fps_smooth += (fps - self.fps_smooth) * 0.08;
        }

        // Панель статуса: тёмное поле с двойной рамкой (за текстом и сердцами).
        draw_rectangle(6.0, 6.0, 320.0, 96.0, Color::new(0.02, 0.016, 0.014, 0.62));
        draw_rectangle_lines(6.0, 6.0, 320.0, 96.0, 1.0, Color::new(0.30, 0.27, 0.24, 0.85));
        draw_rectangle_lines(9.0, 9.0, 314.0, 90.0, 1.0, Color::new(0.16, 0.14, 0.12, 0.9));
        // Угловые заклёпки рамки.
        for (cx, cy) in [(10.0, 10.0), (318.0, 10.0), (10.0, 94.0), (318.0, 94.0)] {
            draw_rectangle(cx, cy, 4.0, 4.0, GOLD);
        }

        draw_text(
            &format!("FPS {:.0}", self.fps_smooth),
            16.0,
            24.0,
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
            PlayerState::Attack => "ATTACK",
        };
        draw_text(
            &format!("STATE: {state}"),
            16.0,
            48.0,
            20.0,
            TEXT_DIM,
        );

        // Здоровье: пиксельные сердца (паттерн 5×5, шрифт без глифов ♥).
        for i in 0..self.player.max_hp {
            draw_heart(16.0 + i as f32 * 20.0, 68.0, i < self.player.hp);
        }
        // Шрифт по умолчанию в macroquad — только ASCII, поэтому HUD на латинице.
        draw_text(
            "A/D - move   SPACE - jump   SHIFT - dash   J/Z - attack",
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
    // два слизня на полу, летучая мышь над ямой, культист на платформе,
    // скелет и некрорыцарь-босс у правой стены.
    let floor_top = 14.0 * TILE;
    let enemy_spawns = vec![
        (EnemyKind::Slime, Vec2::new(10.0 * TILE, floor_top - 20.0)),
        (EnemyKind::Slime, Vec2::new(36.0 * TILE, floor_top - 20.0)),
        (EnemyKind::Skeleton, Vec2::new(44.0 * TILE, floor_top - 38.0)),
        (EnemyKind::Bat, Vec2::new(21.5 * TILE, 9.0 * TILE)),
        // Культист держит среднюю дистанцию на верхней платформе (y = 10*TILE).
        (EnemyKind::Cultist, Vec2::new(14.0 * TILE, 10.0 * TILE - 34.0)),
        // Босс — «босс-арена» у правой границы уровня.
        (EnemyKind::DeathKnight, Vec2::new(52.0 * TILE, floor_top - 56.0)),
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

/// Фон: градиент неба, луна, звёзды, четыре слоя силуэтов (руины,
/// мёртвые деревья, скалы, ближний лес) и полосы тумана у горизонта.
fn draw_parallax(cam: &Vec2, view: &Vec2, time: f32) {
    let left = cam.x - view.x * 0.5;
    let top = cam.y - view.y * 0.5;

    // --- небо: вертикальный градиент тёмного (вверху) к тёплому у горизонта ---
    let sky_bands = 8;
    let sky_h = (HORIZON - top).max(view.y) / sky_bands as f32 + 1.0;
    for i in 0..sky_bands {
        let t = i as f32 / (sky_bands - 1) as f32;
        let c = Color::new(
            0.031 + t * 0.062,
            0.027 + t * 0.043,
            0.031 + t * 0.024,
            1.0,
        );
        draw_rectangle(left, top + i as f32 * sky_h, view.x, sky_h, c);
    }

    // --- звёзды: редкие тёплые точки с мерцанием ---
    let star_cell = 34.0;
    let star_k0 = ((left * 0.94 / star_cell).floor()) as i32;
    let star_k1 = (((left + view.x) * 0.94 / star_cell).ceil()) as i32;
    let star_bottom = (HORIZON - 70.0).max(top + 20.0);
    for k in star_k0..=star_k1 {
        for row in 0..9 {
            let n1 = hash_noise(k * 71 + row * 37 + 5);
            let n2 = hash_noise(k * 131 + row * 17 + 9);
            if n1 < 0.25 {
                continue; // не в каждой ячейке есть звезда
            }
            let sx = left
                + ((n1 + 1.0) * 0.5 * view.x + cam.x * 0.04).rem_euclid(view.x);
            let sy = top + 14.0 + row as f32 * ((star_bottom - top - 20.0) / 9.0)
                + (n2 + 1.0) * 6.0;
            if sy > star_bottom {
                continue;
            }
            let tw = 0.55 + 0.45 * (time * 1.7 + n2 * 9.0).sin();
            draw_rectangle(
                sx,
                sy,
                1.5,
                1.5,
                Color::new(0.85, 0.82, 0.74, 0.20 + 0.35 * tw),
            );
        }
    }

    // --- луна: тусклый диск с кратерами и тёплым ореолом ---
    let moon_x = cam.x * 0.94 + 420.0;
    let moon_y = top + 52.0;
    draw_circle(moon_x, moon_y, 30.0, Color::new(0.85, 0.75, 0.55, 0.035));
    draw_circle(moon_x, moon_y, 17.0, Color::new(0.85, 0.78, 0.60, 0.06));
    draw_circle(moon_x, moon_y, 10.5, Color::new(0.78, 0.75, 0.66, 0.85));
    draw_circle(moon_x - 3.0, moon_y - 2.0, 2.6, Color::new(0.62, 0.60, 0.54, 0.55));
    draw_circle(moon_x + 3.5, moon_y + 3.0, 1.8, Color::new(0.64, 0.62, 0.56, 0.5));
    draw_circle(moon_x + 1.5, moon_y - 4.5, 1.2, Color::new(0.63, 0.61, 0.55, 0.45));

    // --- дальний слой: горный хребет с руинами замка ---
    {
        let speed = 0.12; // доля движения камеры: дальше — медленнее
        let color = Color::new(0.098, 0.082, 0.071, 1.0);
        let span = 260.0;
        let shift = cam.x * (1.0 - speed);
        let k_start = (((left - shift) / span).floor()) as i32 - 1;
        let k_end = (((left + view.x - shift) / span).ceil()) as i32 + 1;
        for k in k_start..=k_end {
            let x = k as f32 * span + shift;
            // Хребет: ступенчатые колонны.
            let col_w = 16.0;
            let cols = (span / col_w) as i32 + 1;
            for c in 0..cols {
                let n = hash_noise(k * 977 + c * 53 + 3);
                let h = 60.0 + (n + 1.0) * 0.5 * 90.0;
                // Плавный силуэт: средняя колонна выше.
                let center_bias = 1.0 - ((c as f32 / cols as f32) - 0.5).abs();
                let h = h * (0.75 + center_bias * 0.45);
                draw_rectangle(x + c as f32 * col_w, HORIZON - h, col_w + 1.0, h, color);
            }
            // Руина замка на некоторых сегментах.
            let m = hash_noise(k * 613 + 11);
            if m > 0.35 {
                let tower_x = x + span * 0.3 + m * 40.0;
                let tower_h = 120.0 + (m + 1.0) * 35.0;
                draw_rectangle(tower_x, HORIZON - tower_h, 30.0, tower_h, color);
                // Зубцы.
                for ci in 0..3 {
                    draw_rectangle(
                        tower_x + ci as f32 * 11.0,
                        HORIZON - tower_h - 7.0,
                        7.0,
                        7.0,
                        color,
                    );
                }
                // Пролом стены.
                draw_rectangle(tower_x + 34.0, HORIZON - tower_h * 0.55, 44.0, tower_h * 0.55, color);
                draw_rectangle(
                    tower_x + 74.0,
                    HORIZON - tower_h * 0.40,
                    24.0,
                    tower_h * 0.40,
                    color,
                );
                // Тусклые окна-бойницы.
                draw_rectangle(
                    tower_x + 12.0,
                    HORIZON - tower_h + 24.0,
                    5.0,
                    11.0,
                    Color::new(0.72, 0.48, 0.20, 0.5),
                );
                if m > 0.6 {
                    draw_rectangle(
                        tower_x + 12.0,
                        HORIZON - tower_h + 52.0,
                        5.0,
                        11.0,
                        Color::new(0.66, 0.42, 0.18, 0.4),
                    );
                }
            }
        }
    }

    // --- второй слой: холмы и мёртвые деревья ---
    {
        let speed = 0.3;
        let color = Color::new(0.129, 0.106, 0.090, 1.0);
        let tree_color = Color::new(0.106, 0.086, 0.071, 1.0);
        let span = 190.0;
        let shift = cam.x * (1.0 - speed);
        let k_start = (((left - shift) / span).floor()) as i32 - 1;
        let k_end = (((left + view.x - shift) / span).ceil()) as i32 + 1;
        for k in k_start..=k_end {
            let x = k as f32 * span + shift;
            let n = hash_noise(k * 449 + 29);
            let h = 46.0 + (n + 1.0) * 0.5 * 55.0;
            draw_rectangle(x, HORIZON - h, span * 0.72, h, color);
            draw_rectangle(x + span * 0.5, HORIZON - h - 14.0, span * 0.4, 16.0, color);

            // Мёртвое дерево-скелет.
            let t = hash_noise(k * 271 + 7);
            if t > 0.2 {
                let tx = x + span * 0.15 + (t + 1.0) * 30.0;
                let th = 44.0 + (t + 1.0) * 16.0;
                draw_rectangle(tx, HORIZON - th, 4.0, th, tree_color);
                draw_rectangle(tx - 10.0, HORIZON - th + 6.0, 11.0, 4.0, tree_color);
                draw_rectangle(tx - 13.0, HORIZON - th + 1.0, 4.0, 7.0, tree_color);
                draw_rectangle(tx + 3.0, HORIZON - th + 13.0, 10.0, 4.0, tree_color);
                draw_rectangle(tx + 11.0, HORIZON - th + 8.0, 4.0, 7.0, tree_color);
            }
        }
    }

    // --- третий слой: скальные шпили и стены руин ---
    {
        let speed = 0.55;
        let color = Color::new(0.153, 0.130, 0.110, 1.0);
        let span = 150.0;
        let shift = cam.x * (1.0 - speed);
        let k_start = (((left - shift) / span).floor()) as i32 - 1;
        let k_end = (((left + view.x - shift) / span).ceil()) as i32 + 1;
        for k in k_start..=k_end {
            let x = k as f32 * span + shift;
            let n = hash_noise(k * 811 + 41);
            if n > -0.15 {
                // Шпиль: ступенчатый треугольник.
                let sh = 70.0 + (n + 1.0) * 0.5 * 60.0;
                let steps = 5;
                for s in 0..steps {
                    let sw = 46.0 * (1.0 - s as f32 / steps as f32) + 8.0;
                    draw_rectangle(
                        x + (46.0 - sw) * 0.5,
                        HORIZON - sh + s as f32 * (sh / steps as f32),
                        sw,
                        sh / steps as f32 + 1.0,
                        color,
                    );
                }
            } else {
                // Обломок стены с бойницей.
                let wh = 52.0 + (n + 1.0) * 30.0;
                draw_rectangle(x + 20.0, HORIZON - wh, 64.0, wh, color);
                draw_rectangle(x + 34.0, HORIZON - wh + 14.0, 7.0, 16.0, Color::new(0.05, 0.04, 0.035, 1.0));
                draw_rectangle(x + 58.0, HORIZON - wh + 14.0, 7.0, 16.0, Color::new(0.05, 0.04, 0.035, 1.0));
                draw_rectangle(x + 20.0, HORIZON - wh - 6.0, 14.0, 6.0, color);
                draw_rectangle(x + 66.0, HORIZON - wh - 6.0, 14.0, 6.0, color);
            }
        }
    }

    // --- ближний слой: тёмный кряж у самого горизонта ---
    {
        let speed = 0.8;
        let color = Color::new(0.184, 0.157, 0.133, 1.0);
        let span = 120.0;
        let shift = cam.x * (1.0 - speed);
        let k_start = (((left - shift) / span).floor()) as i32 - 1;
        let k_end = (((left + view.x - shift) / span).ceil()) as i32 + 1;
        for k in k_start..=k_end {
            let x = k as f32 * span + shift;
            let n = hash_noise(k * 197 + 61);
            let h = 26.0 + (n + 1.0) * 0.5 * 34.0;
            draw_rectangle(x, HORIZON - h, span + 1.0, h, color);
            // Верхушки елей.
            let m = hash_noise(k * 353 + 13);
            if m > 0.0 {
                let px = x + (m + 1.0) * 0.5 * span * 0.7;
                let ph = h + 26.0 + (m + 1.0) * 10.0;
                for s in 0..4 {
                    let sw = 26.0 - s as f32 * 6.0;
                    draw_rectangle(
                        px - sw * 0.5 + 6.0,
                        HORIZON - ph + s as f32 * 7.0,
                        sw,
                        8.0,
                        color,
                    );
                }
            }
        }
    }

    // --- туман у горизонта: мягкие полосы с лёгким дрейфом ---
    for i in 0..4 {
        let drift = (time * (6.0 + i as f32 * 3.0)).sin() * 18.0;
        draw_rectangle(
            left - 20.0 + drift,
            HORIZON - 14.0 + i as f32 * 5.0,
            view.x + 40.0,
            5.0,
            Color::new(FOG.r, FOG.g, FOG.b, 0.045 + i as f32 * 0.02),
        );
    }
}

/// Тайловая отрисовка: каменная кладка (ряды со швами и забутовкой),
/// сколы и трещины, мох на открытых верхах, боковая подсветка рёбер.
fn draw_tiles(world: &World, cam: &Vec2, view: &Vec2) {
    let visible = Aabb::new(
        cam.x - view.x * 0.5 - TILE,
        cam.y - view.y * 0.5 - TILE,
        view.x + TILE * 2.0,
        view.y + TILE * 2.0,
    );
    let view_rect = Aabb::new(
        cam.x - view.x * 0.5,
        cam.y - view.y * 0.5,
        view.x,
        view.y,
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

                // Отсечение невидимых тайлов — экономия отрисовки.
                if !view_rect.overlaps(&Aabb::new(x, y, TILE, TILE)) {
                    continue;
                }

                let seed = tx * 73 + ty * 131;
                let shade = ((hash_noise(seed) + 1.0) * 0.5 * 3.0) as usize;
                let base = STONE[shade.min(2)];
                draw_rectangle(x, y, TILE, TILE, base);

                // --- кладка: три ряда кирпичей со смещёнными стыками ---
                let rows = [11.0, 10.0, 11.0];
                let mut ry = y;
                for (ri, rh) in rows.iter().enumerate() {
                    if ri > 0 {
                        // Горизонтальный шов + тень под ним.
                        draw_rectangle(x, ry - 1.0, TILE, 1.0, STONE_DARK);
                        draw_rectangle(x, ry, TILE, 1.0, STONE_LO);
                    }
                    // Вертикальные стыки (кладка вразбежку): второй
                    // элемент — NaN-заглушка, когда стык один.
                    let off =
                        ((hash_noise(seed + ri as i32 * 17) + 1.0) * 0.5 * 6.0) - 3.0;
                    let joint: [f32; 2] = match ri {
                        0 => [16.0 + off, f32::NAN],
                        1 => [8.0, 24.0],
                        _ => [16.0 - off, f32::NAN],
                    };
                    for jx in joint {
                        if jx.is_nan() {
                            continue;
                        }
                        draw_rectangle(x + jx, ry + 1.0, 1.0, *rh - 2.0, STONE_DARK);
                        // Подсветка края стыка (свет из факелов/сверху).
                        draw_rectangle(x + jx + 1.0, ry + 1.0, 1.0, *rh - 2.0, STONE_HI);
                    }
                    // Лёгкая верхняя грань ряда.
                    draw_rectangle(x + 1.0, ry + 1.0, TILE - 2.0, 1.0, STONE_HI);
                    ry += *rh;
                }

                // --- края: боковые грани и тени ---
                let open_above = !world.overlaps_any(Aabb::new(x, y - 1.0, TILE, 1.0));
                let open_left = !world.overlaps_any(Aabb::new(x - 1.0, y, 1.0, TILE));
                let open_right = !world.overlaps_any(Aabb::new(x + TILE, y, 1.0, TILE));
                let open_below = !world.overlaps_any(Aabb::new(x, y + TILE, TILE, 1.0));

                if open_left {
                    draw_rectangle(x, y, 3.0, TILE, STONE_LO);
                    draw_rectangle(x + 3.0, y, 1.0, TILE, STONE_HI);
                }
                if open_right {
                    draw_rectangle(x + TILE - 3.0, y, 3.0, TILE, STONE_LO);
                }
                if open_below {
                    draw_rectangle(x, y + TILE - 4.0, TILE, 4.0, STONE_LO);
                    draw_rectangle(x, y + TILE - 1.0, TILE, 1.0, OUTLINE);
                }

                // --- открытый верх: мох, трава, земля ---
                if open_above {
                    let moss_h = 5.0 + ((hash_noise(seed + 5) + 1.0) * 0.5) * 5.0;
                    draw_rectangle(x, y, TILE, moss_h, MOSS);
                    draw_rectangle(x, y, TILE, 2.0, MOSS_HI);
                    // Кусочки мха свисают вниз по краям.
                    for i in 0..3 {
                        let dx = ((hash_noise(seed + 40 + i) + 1.0) * 0.5) * (TILE - 4.0);
                        let dh = 2.0 + ((hash_noise(seed + 50 + i) + 1.0) * 0.5) * 4.0;
                        draw_rectangle(x + dx, y + moss_h - 1.0, 3.0, dh, MOSS_LO);
                    }
                    draw_rectangle(x, y + moss_h, TILE, 3.0, DIRT);
                    draw_rectangle(x, y + moss_h + 3.0, TILE, 1.0, DIRT_LO);
                    // Камешки у основания мха.
                    let n = ((hash_noise(seed + 9) + 1.0) * 1.5) as i32;
                    for i in 0..n {
                        let px = 3.0 + ((hash_noise(seed + 60 + i) + 1.0) * 0.5) * (TILE - 8.0);
                        draw_rectangle(x + px, y + moss_h + 4.0, 2.0, 2.0, STONE_HI);
                    }
                }

                // --- трещины и сколы ---
                let cracks = ((hash_noise(seed + 3) + 1.0) * 1.2) as i32;
                for c in 0..cracks {
                    let mut px = x + 5.0
                        + ((hash_noise(seed + 13 + c * 7) + 1.0) * 0.5) * (TILE - 12.0);
                    let mut py = y + 6.0
                        + ((hash_noise(seed + 23 + c * 7) + 1.0) * 0.5) * (TILE - 14.0);
                    for s in 0..4 {
                        draw_rectangle(px, py, 1.5, 1.5, STONE_LO);
                        px += ((hash_noise(seed + 31 + c * 11 + s * 5) + 1.0) * 0.5) * 4.0 - 1.5;
                        py += 2.0 + ((hash_noise(seed + 37 + c * 13 + s * 3) + 1.0) * 0.5) * 2.0;
                    }
                }
                // Скол-пятно в углу.
                if hash_noise(seed + 19) > 0.55 {
                    let corner = ((hash_noise(seed + 21) + 1.0) * 0.5) as i32;
                    let cx = if corner % 2 == 0 { x + 2.0 } else { x + TILE - 8.0 };
                    let cy = if corner > 1 { y + 2.0 } else { y + TILE - 8.0 };
                    draw_rectangle(cx, cy, 6.0, 6.0, STONE_LO);
                    draw_rectangle(cx + 1.0, cy + 1.0, 3.0, 3.0, STONE[0]);
                }

                // Чёткий шов по периметру — пиксельная сетка.
                draw_rectangle_lines(x, y, TILE, TILE, 1.0, STONE_DARK);
            }
        }
    }
}

/// Пиксельное сердце (паттерн 5×5 при масштабе 3): обводка, заливка,
/// блик. Пустое сердце — тёмная «пустая» версия того же узора.
fn draw_heart(x: f32, y: f32, filled: bool) {
    const S: f32 = 3.0;
    // 1 — заполненная ячейка узора.
    const PATTERN: [[u8; 5]; 5] = [
        [0, 1, 0, 1, 0],
        [1, 1, 1, 1, 1],
        [1, 1, 1, 1, 1],
        [0, 1, 1, 1, 0],
        [0, 0, 1, 0, 0],
    ];

    // Сначала обводка (расширяем узор на 1 единицу).
    for r in 0..5usize {
        for c in 0..5usize {
            if PATTERN[r][c] == 1 {
                draw_rectangle(
                    x + c as f32 * S - 1.0,
                    y + r as f32 * S - 1.0,
                    S + 2.0,
                    S + 2.0,
                    OUTLINE,
                );
            }
        }
    }
    // Заливка.
    let fill = if filled { HEART_FILL } else { HEART_EMPTY };
    for r in 0..5usize {
        for c in 0..5usize {
            if PATTERN[r][c] == 1 {
                draw_rectangle(x + c as f32 * S, y + r as f32 * S, S, S, fill);
            }
        }
    }
    if filled {
        // Блик на левой дольке + тень снизу.
        draw_rectangle(x + S + 1.0, y + S + 1.0, S - 1.0, S - 1.0, HEART_SHINE);
        draw_rectangle(x + 2.0 * S, y + 4.0 * S, S, 1.0, Color::new(0.35, 0.09, 0.08, 1.0));
    } else {
        // Пустое: тёмный контур внутри.
        draw_rectangle(x + S + 1.0, y + S + 1.0, S - 1.0, S - 1.0, Color::new(0.16, 0.13, 0.12, 1.0));
    }
}

/// Пылинки и угольки в воздухе: тёплые искры и пыль, медленно дрейфуют.
fn draw_motes(cam: &Vec2, view: &Vec2, time: f32) {
    for i in 0..26 {
        let h1 = hash_noise(i * 37 + 5);
        let h2 = hash_noise(i * 91 + 13);
        let speed = 5.0 + (h1 + 1.0) * 4.0;
        let span_x = view.x + 60.0;
        let span_y = view.y + 60.0;
        let px = cam.x - view.x * 0.5 - 30.0
            + (((h1 + 1.0) * 0.5 * span_x + time * speed * 0.35) % span_x);
        let py = cam.y - view.y * 0.5 - 30.0
            + (((h2 + 1.0) * 0.5 * span_y - time * speed).rem_euclid(span_y));
        let tw = 0.5 + 0.5 * (time * 2.6 + i as f32 * 1.7).sin();
        let warm = i % 3 == 0;
        let c = if warm {
            Color::new(EMBER.r, EMBER.g, EMBER.b, 0.12 + 0.22 * tw)
        } else {
            Color::new(0.62, 0.58, 0.50, 0.08 + 0.14 * tw)
        };
        let size = if warm { 2.0 } else { 1.5 };
        draw_rectangle(px, py, size, size, c);
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
        let flick2 = hash_noise(i as i32 * 31 + (time * 13.0 + 4.0) as i32);
        let pulse = 1.0 + flicker * 0.14;
        let lean = flick2 * 2.0; // язык пламя косит в сторону

        // --- мягкий тёплый свет: три кольца + пятно на полу ---
        draw_circle(x + 3.0, y - 6.0, 64.0 * pulse, Color::new(0.851, 0.643, 0.255, 0.030));
        draw_circle(x + 3.0, y - 6.0, 40.0 * pulse, Color::new(0.851, 0.643, 0.255, 0.055));
        draw_circle(x + 3.0, y - 4.0, 22.0 * pulse, Color::new(0.906, 0.706, 0.318, 0.095));
        // Световое пятно (горизонтальные слои — условный эллипс).
        let pool_w = 46.0 * pulse;
        draw_rectangle(
            x - pool_w * 0.5 + 3.0,
            y + 14.0,
            pool_w,
            5.0,
            Color::new(0.851, 0.643, 0.255, 0.05),
        );
        draw_rectangle(
            x - pool_w * 0.32 + 3.0,
            y + 14.0,
            pool_w * 0.64,
            3.0,
            Color::new(0.906, 0.706, 0.318, 0.07),
        );

        // --- кованый держатель: пластина, заклёпки, кронштейн, корзинка ---
        draw_rectangle(x - 1.0, y + 1.0, 8.0, 17.0, IRON);
        draw_rectangle(x - 1.0, y + 1.0, 8.0, 2.0, STEEL_DARK); // верх пластины
        draw_rectangle(x - 1.0, y + 16.0, 8.0, 2.0, OUTLINE); // низ пластины
        draw_rectangle(x + 0.0, y + 4.0, 2.0, 2.0, STEEL_HI); // заклёпка 1
        draw_rectangle(x + 5.0, y + 4.0, 2.0, 2.0, STEEL_HI); // заклёпка 2
        draw_rectangle(x + 5.0, y + 12.0, 2.0, 2.0, STEEL_HI); // заклёпка 3
        // Рукоять вверх и чаша.
        draw_rectangle(x + 1.0, y - 4.0, 4.0, 6.0, IRON);
        draw_rectangle(x - 2.0, y - 5.0, 10.0, 3.0, IRON);
        draw_rectangle(x - 2.0, y - 5.0, 10.0, 1.0, STEEL_DARK);
        // Рёбра корзинки.
        draw_rectangle(x - 3.0, y - 10.0, 2.0, 7.0, IRON);
        draw_rectangle(x + 7.0, y - 10.0, 2.0, 7.0, IRON);
        draw_rectangle(x - 3.0, y - 11.0, 12.0, 2.0, IRON);

        // --- многослойное пламя ---
        let fh = 17.0 * pulse;
        // Внешний тёмно-оранжевый язык (ступенчатый, с наклоном).
        draw_rectangle(x + 0.5 + lean * 0.4, y - fh * 0.72, 7.0, fh * 0.72, FLAME_DEEP);
        draw_rectangle(x + 1.5 + lean * 0.7, y - fh, 5.0, fh * 0.34, FLAME_DEEP);
        draw_rectangle(x + 2.5 + lean, y - fh - 3.0, 3.0, 4.0, FLAME_DEEP);
        // Средний оранжевый.
        draw_rectangle(x + 1.5 + lean * 0.5, y - fh * 0.60, 5.0, fh * 0.60, FLAME);
        draw_rectangle(x + 2.2 + lean * 0.8, y - fh * 0.86, 3.0, fh * 0.30, FLAME);
        // Ядро.
        draw_rectangle(x + 2.5 + lean * 0.3, y - fh * 0.34, 3.0, fh * 0.34, FLAME_CORE);
        draw_circle(x + 4.0, y - 2.0, 3.2, Color::new(0.98, 0.90, 0.65, 0.9));

        // --- угольки, поднимающиеся от пламени ---
        for e in 0..3 {
            let phase = (time * (1.3 + e as f32 * 0.4) + i as f32 * 0.7 + e as f32 * 2.1)
                .rem_euclid(1.0);
            let ex = x + 3.0 + ((time * (18.0 + e as f32 * 7.0) + i as f32 * 40.0)
                .sin()
                * (4.0 + e as f32 * 3.0))
                + lean;
            let ey = y - 6.0 - phase * 42.0;
            let ea = (1.0 - phase) * 0.75;
            let es = if phase < 0.4 { 2.0 } else { 1.5 };
            draw_rectangle(ex, ey, es, es, Color::new(EMBER.r, EMBER.g, EMBER.b, ea));
        }
    }
}

/// Рыцарь: послойная пиксельная отрисовка — обводка силуэта, бевели
/// (свет сверху-слева), детали доспеха, плащ со складками, шлем с плюмажем.
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

    // --- поза ---
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
            crouch = 3.0;
            leg_phase = 0.7;
        }
        PlayerState::Hurt => {
            crouch = 2.0;
            leg_phase = 0.6;
        }
        PlayerState::Dash => {
            crouch = 6.0;
            lean = 3.0 * p.facing;
            leg_phase = 1.0;
        }
        PlayerState::Attack => {
            // Свинг: присед-выпад корпуса вперёд.
            crouch = 3.0;
            lean = 2.5 * p.facing;
            leg_phase = 0.35;
        }
    }
    let top = y + bob + crouch;

    // --- тень на земле (слоями — мягкий край) ---
    if p.on_ground {
        draw_rectangle(x + 1.0, y + h - 1.0, w - 2.0, 4.0, Color::new(0.0, 0.0, 0.0, 0.30));
        draw_rectangle(x + 4.0, y + h - 1.0, w - 8.0, 3.0, Color::new(0.0, 0.0, 0.0, 0.22));
    }

    // --- шлейф рывка: силуэты-призраки позади ---
    if p.state == PlayerState::Dash {
        let dir = p.facing;
        for i in 0..3 {
            let a = 0.40 - i as f32 * 0.11;
            let gx = x - dir * (7.0 + i as f32 * 8.0);
            let ghost = Color::new(0.66, 0.63, 0.59, a);
            draw_rectangle(gx + 5.0, top + 10.0, w - 10.0, 17.0, ghost);
            draw_rectangle(gx + 6.0, top + 1.0, 12.0, 10.0, ghost);
            draw_rectangle(gx + 7.0, top + 25.0, 4.0, 14.0, ghost);
        }
    }

    // --- примитивы: обводка + заливка, бевель свет/тень ---
    let part = |lx: f32, ly: f32, pw: f32, ph: f32, fill: Color| {
        draw_rectangle(fx(lx, pw) - 1.0, top + ly - 1.0, pw + 2.0, ph + 2.0, OUTLINE);
        draw_rectangle(fx(lx, pw), top + ly, pw, ph, fill);
    };
    let bevel = |lx: f32, ly: f32, pw: f32, ph: f32, base: Color, hi: Color, lo: Color| {
        part(lx, ly, pw, ph, base);
        draw_rectangle(fx(lx, pw), top + ly, pw, 1.0, hi);
        draw_rectangle(fx(lx, pw), top + ly + ph - 1.0, pw, 1.0, lo);
        draw_rectangle(fx(lx, pw), top + ly, 1.0, ph, hi);
        draw_rectangle(fx(lx, pw) + pw - 1.0, top + ly, 1.0, ph, lo);
    };

    // --- плащ: три полосы-складки разной длины, колышется ---
    let sway = (time * 6.0).sin() * 1.5 - p.vel.x * 0.012;
    let cloak_h = 23.0 - crouch * 0.5;
    part(2.0 + sway, 12.0, 4.0, cloak_h, CLOAK);
    part(6.0 + sway * 0.7, 12.0, 4.0, cloak_h - 3.0, CLOAK_DARK);
    part(10.0 + sway * 0.4, 12.0, 3.0, cloak_h - 1.0, CLOAK);
    // Открытый край ткани.
    draw_rectangle(fx(2.0 + sway, 4.0), top + 12.0, 1.0, cloak_h, Color::new(0.62, 0.32, 0.26, 1.0));

    // --- ножны меча за спиной (набалдашник над плечом) ---
    part(18.0, 3.0, 4.0, 24.0, LEATHER);
    draw_rectangle(fx(18.0, 4.0), top + 3.0, 4.0, 2.0, STEEL_DARK);
    draw_rectangle(fx(19.0, 2.0), top - 1.0, 2.0, 4.0, GOLD);
    draw_rectangle(fx(18.0, 4.0), top + 25.0, 4.0, 3.0, STEEL_DARK);

    // --- ноги: бедро, наколенник, голень, сапог с подошвой ---
    let leg_y = top + h - 16.0;
    let swing = leg_phase * 4.5;
    let (leg1, leg2) = if p.state == PlayerState::Run {
        (4.0 + swing, 12.0 - swing)
    } else if p.state == PlayerState::WallSlide {
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
    let leg_lift = leg_phase.abs() * 2.0;
    let leg = |lx: f32, lift: f32| {
        let ly = leg_y - lift;
        part(lx, ly, 6.0, 5.0, STEEL_DARK);
        draw_rectangle(fx(lx + 1.0, 4.0), ly + 4.0, 4.0, 3.0, STEEL);
        part(lx, ly + 7.0, 6.0, 4.0, STEEL_DARK);
        part(lx, ly + 10.0, 7.0, 5.0, LEATHER);
        draw_rectangle(fx(lx, 7.0), ly + 14.0, 7.0, 1.0, OUTLINE);
        // Заклёпка на сапоге.
        draw_rectangle(fx(lx + 1.0, 2.0), ly + 11.0, 2.0, 2.0, GOLD);
    };
    leg(leg1, if p.state == PlayerState::Run { leg_lift } else { 0.0 });
    leg(leg2, if p.state == PlayerState::Run { leg_lift * 0.5 } else { 0.0 });

    // --- корпус: нагрудник с гребнем и сегментами ---
    let tl = 4.0 + lean;
    bevel(tl, 12.0, 16.0, 13.0, STEEL, STEEL_HI, STEEL_DARK);
    draw_rectangle(fx(tl + 7.0, 2.0), top + 13.0, 2.0, 9.0, STEEL_HI);
    draw_rectangle(fx(tl + 1.0, 14.0), top + 18.0, 14.0, 1.0, STEEL_DARK);
    draw_rectangle(fx(tl + 1.0, 14.0), top + 21.0, 14.0, 1.0, STEEL_DARK);
    draw_rectangle(fx(tl + 2.0, 2.0), top + 14.0, 2.0, 2.0, GOLD);
    draw_rectangle(fx(tl + 12.0, 2.0), top + 14.0, 2.0, 2.0, GOLD);

    // --- пояс с пряжкой ---
    part(tl, 23.0, 16.0, 3.0, LEATHER);
    draw_rectangle(fx(tl + 6.0, 4.0), top + 23.0, 4.0, 3.0, GOLD);
    draw_rectangle(fx(tl + 7.0, 2.0), top + 24.0, 2.0, 1.0, CLOAK_DARK);

    // --- наплечники (два яруса) ---
    bevel(tl - 2.0, 11.0, 7.0, 5.0, STEEL, STEEL_HI, STEEL_DARK);
    bevel(tl + 11.0, 11.0, 7.0, 5.0, STEEL, STEEL_HI, STEEL_DARK);
    part(tl - 1.0, 15.0, 5.0, 3.0, STEEL_DARK);
    part(tl + 12.0, 15.0, 5.0, 3.0, STEEL_DARK);

    // --- руки: задняя видна слева, передняя с наружи ---
    part(tl - 1.0, 16.0, 4.0, 7.0, STEEL_DARK);
    part(tl + 13.0, 15.0, 5.0, 7.0, STEEL_DARK);
    part(tl + 13.0, 21.0, 5.0, 4.0, STEEL);
    draw_rectangle(fx(tl + 13.0, 5.0), top + 21.0, 5.0, 1.0, STEEL_HI);

    // --- шлем: плюмаж, бровь, глазница, подбородок ---
    let hl = 5.0 + lean;
    bevel(hl, 1.0, 14.0, 12.0, STEEL, STEEL_HI, STEEL_DARK);
    // Плюмаж из трёх сегментов (колышется).
    let plume_sway = (time * 5.0).sin() * 1.2;
    draw_rectangle(fx(hl + 4.0, 5.0), top - 3.0, 5.0, 4.0, CLOAK);
    draw_rectangle(fx(hl + 3.0 + plume_sway, 5.0), top - 6.0, 5.0, 4.0, CLOAK);
    draw_rectangle(fx(hl + 2.0 + plume_sway * 1.6, 4.0), top - 8.0, 4.0, 3.0, CLOAK_DARK);
    draw_rectangle(fx(hl + 4.0, 6.0), top - 1.0, 6.0, 2.0, GOLD);
    // Глазница смотрит в сторону движения.
    draw_rectangle(fx(hl + 6.0, 7.0), top + 5.0, 7.0, 3.0, VISOR);
    draw_rectangle(fx(hl + 5.0, 9.0), top + 4.0, 9.0, 1.0, STEEL_HI);
    // Тусклая янтарная искра в щели.
    draw_rectangle(fx(hl + 11.0, 2.0), top + 6.0, 2.0, 1.0, EMBER);
    // Подбородочный щит и дыхательные прорези.
    draw_rectangle(fx(hl + 2.0, 10.0), top + 8.0, 10.0, 4.0, STEEL_DARK);
    draw_rectangle(fx(hl + 8.0, 2.0), top + 9.0, 2.0, 2.0, OUTLINE);
    draw_rectangle(fx(hl + 11.0, 2.0), top + 9.0, 2.0, 2.0, OUTLINE);
    // Заклёпки по краю шлема.
    draw_rectangle(fx(hl + 2.0, 2.0), top + 2.0, 2.0, 2.0, GOLD);
    draw_rectangle(fx(hl + 10.0, 2.0), top + 2.0, 2.0, 2.0, GOLD);
    // Шейный стояк.
    draw_rectangle(fx(hl + 3.0, 8.0), top + 13.0, 8.0, 2.0, STEEL_DARK);

    // --- меч в передней руке + дуга свинга ---
    let attacking = p.state == PlayerState::Attack;
    let hand = Vec2::new(
        fx(tl + 16.0, 2.0) + 1.0,
        top + if attacking { 13.0 } else { 17.0 },
    );
    // Угол клинка (y-down: положительный — «вниз»). Свинг: из-за головы
    // через горизонталь вниз; покой — клинок опущен вперёд.
    let theta = if attacking {
        let t = p.attack_progress();
        let ease = t * t * (3.0 - 2.0 * t); // smoothstep
        -2.55 + ease * 3.25 // -146° → +40°
    } else {
        0.87 // ~50° вниз-вперёд
    };
    let theta = if facing_right { theta } else { std::f32::consts::PI - theta };
    let blade_len = if attacking { 30.0 } else { 26.0 };
    let dirv = Vec2::new(theta.cos(), theta.sin());
    let perp = Vec2::new(-dirv.y, dirv.x);

    // Шлейф взмаха — три полупрозрачных следа позади клинка.
    if p.is_attack_active() {
        for i in 1..=3 {
            let ta = theta - 0.26 * i as f32 * if facing_right { 1.0 } else { -1.0 };
            let td = Vec2::new(ta.cos(), ta.sin());
            let a = 0.34 - i as f32 * 0.09;
            draw_line(
                hand.x + td.x * 4.0,
                hand.y + td.y * 4.0,
                hand.x + td.x * blade_len,
                hand.y + td.y * blade_len,
                5.0,
                Color::new(0.88, 0.90, 0.94, a),
            );
        }
    }

    // Крестовина (перпендикуляр клинку).
    let guard_c = hand + dirv * 3.0;
    draw_line(
        guard_c.x - perp.x * 6.0,
        guard_c.y - perp.y * 6.0,
        guard_c.x + perp.x * 6.0,
        guard_c.y + perp.y * 6.0,
        3.0,
        OUTLINE,
    );
    draw_line(
        guard_c.x - perp.x * 5.0,
        guard_c.y - perp.y * 5.0,
        guard_c.x + perp.x * 5.0,
        guard_c.y + perp.y * 5.0,
        1.0,
        GOLD,
    );
    // Рукоять и набалдашник.
    draw_line(
        hand.x - dirv.x * 5.0,
        hand.y - dirv.y * 5.0,
        hand.x + dirv.x * 2.0,
        hand.y + dirv.y * 2.0,
        4.0,
        OUTLINE,
    );
    draw_line(
        hand.x - dirv.x * 4.0,
        hand.y - dirv.y * 4.0,
        hand.x + dirv.x * 1.0,
        hand.y + dirv.y * 1.0,
        2.0,
        LEATHER,
    );
    draw_rectangle(hand.x - dirv.x * 6.0 - 1.5, hand.y - dirv.y * 6.0 - 1.5, 3.0, 3.0, GOLD);
    // Клинок: обводка, сталь, режущий край.
    let tip = hand + dirv * blade_len;
    let base = hand + dirv * 3.0;
    draw_line(base.x, base.y, tip.x, tip.y, 6.0, OUTLINE);
    draw_line(base.x, base.y, tip.x, tip.y, 3.0, STEEL);
    draw_line(
        base.x + perp.x * 1.0,
        base.y + perp.y * 1.0,
        tip.x + perp.x * 1.0,
        tip.y + perp.y * 1.0,
        1.0,
        STEEL_HI,
    );
    // Искра на острие в активных кадрах.
    if p.is_attack_active() {
        draw_rectangle(tip.x - 1.5, tip.y - 1.5, 3.0, 3.0, Color::new(1.0, 0.95, 0.80, 0.95));
    }
}
