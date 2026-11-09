//! Загрузка уровней из Tiled-JSON (формат редактора Tiled, экспорт .json).
//!
//! Карта = тайловые слои `solid` (GID > 0 — твёрдое) и `decor` (GID 10..14 —
//! декор: лианы, цепи, черепа, знамёна, щебень) плюс слой объектов
//! `entities`: игрок, враги, движущиеся платформы, шипы, чекпоинты,
//! монеты и факелы. Уровень лежит в `assets/level1.json` и вкомпилирован
//! в бинарник через `include_str!` — внешние файлы не нужны.

use macroquad::math::Vec2;
use serde::Deserialize;

use crate::enemies::EnemyKind;
use crate::physics::{Aabb, World};

/// Декомпрессированный уровень: геометрия мира и все сущности.
pub struct LevelData {
    /// Твёрдая геометрия (заполненные ряды-прогоны, не отдельные тайлы).
    pub world: World,
    /// Точка спавна игрока (левый верхний угол хитбокса).
    pub spawn: Vec2,
    /// Враги: вид + левый верхний угол хитбокса.
    pub enemy_spawns: Vec<(EnemyKind, Vec2)>,
    /// Движущиеся платформы.
    pub movers: Vec<MoverDef>,
    /// Прямоугольники шипов.
    pub spikes: Vec<Aabb>,
    /// Чекпоинты (точка основания).
    pub checkpoints: Vec<Vec2>,
    /// Монеты (центр).
    pub coins: Vec<Vec2>,
    /// Факелы (базовая точка держателя).
    pub torches: Vec<(f32, f32)>,
    /// Декор-тайлы: (tx, ty, вид).
    pub decor: Vec<(i32, i32, Decor)>,
    /// Дверь-выход (основание): касание → следующий уровень.
    pub exit: Option<Vec2>,
}

/// Вид декор-тайла (GID в слое `decor`).
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Decor {
    /// Свисающая лиана.
    Vine,
    /// Висячая цепь.
    Chain,
    /// Череп на земле.
    Skull,
    /// Знамя на стене.
    Banner,
    /// Щебень и кости.
    Rubble,
}

impl Decor {
    fn from_gid(gid: i32) -> Option<Decor> {
        match gid {
            10 => Some(Decor::Vine),
            11 => Some(Decor::Chain),
            12 => Some(Decor::Skull),
            13 => Some(Decor::Banner),
            14 => Some(Decor::Rubble),
            _ => None,
        }
    }
}

/// Описание движущейся платформы из объекта `mover`.
pub struct MoverDef {
    /// Базовый прямоугольник (px, левый верхний угол).
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// Ось качания: true — горизонталь, false — вертикаль.
    pub axis_x: bool,
    /// Амплитуда (px) от базовой позиции.
    pub range: f32,
    /// Угловая скорость синуса (рад/с).
    pub speed: f32,
    /// Начальная фаза.
    pub phase: f32,
    /// Крушитель: висит наверху, обрушивается вниз, медленно поднимается.
    pub crusher: bool,
}

// --- структуры десериализации Tiled-JSON (только нужные поля) ---

#[derive(Deserialize)]
struct TiledMap {
    width: i32,
    height: i32,
    tilewidth: i32,
    tileheight: i32,
    layers: Vec<TiledLayer>,
}

#[derive(Deserialize)]
#[serde(tag = "type")]
enum TiledLayer {
    #[serde(rename = "tilelayer")]
    Tiles(TileLayer),
    #[serde(rename = "objectgroup")]
    Objects(ObjectGroup),
    #[serde(other)]
    Other,
}

#[derive(Deserialize)]
struct TileLayer {
    name: String,
    data: Vec<i32>,
}

#[derive(Deserialize)]
struct ObjectGroup {
    #[allow(dead_code)]
    name: String,
    #[serde(default)]
    objects: Vec<TiledObject>,
}

#[derive(Deserialize)]
struct TiledObject {
    name: String,
    x: f32,
    y: f32,
    #[serde(default)]
    width: f32,
    #[serde(default)]
    height: f32,
    #[serde(default)]
    properties: Vec<TiledProp>,
}

#[derive(Deserialize)]
struct TiledProp {
    name: String,
    value: serde_json::Value,
}

impl TiledProp {
    fn num(&self) -> Option<f32> {
        self.value.as_f64().map(|v| v as f32)
    }
    fn text(&self) -> Option<&str> {
        self.value.as_str()
    }
    fn flag(&self) -> bool {
        self.value.as_bool().unwrap_or(false)
    }
}

/// Распарсить Tiled-JSON в готовый [`LevelData`].
///
/// # Panics
/// Паникует при битой структуре уровня (слой без данных, неизвестный
/// тайлсайз) — уровень вкомпилирован в бинарник и проверен тестами.
pub fn parse(json: &str) -> LevelData {
    let map: TiledMap =
        serde_json::from_str(json).expect("level: битый Tiled-JSON");
    assert_eq!(map.tilewidth, 32, "level: тайл должен быть 32px");
    assert_eq!(map.tileheight, 32, "level: тайл должен быть 32px");

    let tw = map.tilewidth as f32;
    let th = map.tileheight as f32;

    let mut world = World::new();
    let mut decor = Vec::new();
    let mut spawn = Vec2::ZERO;
    let mut enemy_spawns = Vec::new();
    let mut movers = Vec::new();
    let mut spikes = Vec::new();
    let mut checkpoints = Vec::new();
    let mut coins = Vec::new();
    let mut torches = Vec::new();
    let mut exit = None;

    for layer in &map.layers {
        match layer {
            TiledLayer::Tiles(tiles) => {
                if tiles.name == "solid" {
                    // Прогоны по строке: один AABB на ряд одинаковых тайлов.
                    for ty in 0..map.height {
                        let mut run_start: Option<i32> = None;
                        for tx in 0..=map.width {
                            let gid = if tx < map.width {
                                tiles.data[(ty * map.width + tx) as usize]
                            } else {
                                0 // терминатор — закрывает открытый прогон
                            };
                            if gid > 0 && run_start.is_none() {
                                run_start = Some(tx);
                            }
                            if gid == 0 {
                                if let Some(sx) = run_start.take() {
                                    world.add_solid(Aabb::new(
                                        sx as f32 * tw,
                                        ty as f32 * th,
                                        (tx - sx) as f32 * tw,
                                        th,
                                    ));
                                }
                            }
                        }
                    }
                } else if tiles.name == "decor" {
                    for ty in 0..map.height {
                        for tx in 0..map.width {
                            let gid = tiles.data[(ty * map.width + tx) as usize];
                            if let Some(kind) = Decor::from_gid(gid) {
                                decor.push((tx, ty, kind));
                            }
                        }
                    }
                }
            }
            TiledLayer::Objects(group) => {
                for o in &group.objects {
                    match o.name.as_str() {
                        "player" => spawn = Vec2::new(o.x, o.y),
                        "slime" => enemy_spawns.push((EnemyKind::Slime, Vec2::new(o.x, o.y))),
                        "bat" => enemy_spawns.push((EnemyKind::Bat, Vec2::new(o.x, o.y))),
                        "skeleton" => {
                            enemy_spawns.push((EnemyKind::Skeleton, Vec2::new(o.x, o.y)))
                        }
                        "cultist" => {
                            enemy_spawns.push((EnemyKind::Cultist, Vec2::new(o.x, o.y)))
                        }
                        "deathknight" => {
                            enemy_spawns.push((EnemyKind::DeathKnight, Vec2::new(o.x, o.y)))
                        }
                        "mover" => {
                            let mut axis_x = false;
                            let mut range = 48.0f32;
                            let mut speed = 1.0f32;
                            let mut phase = 0.0f32;
                            let mut crusher = false;
                            for p in &o.properties {
                                match p.name.as_str() {
                                    "axis" => axis_x = p.text() == Some("x"),
                                    "range" => range = p.num().unwrap_or(range),
                                    "speed" => speed = p.num().unwrap_or(speed),
                                    "phase" => phase = p.num().unwrap_or(phase),
                                    "crusher" => crusher = p.flag(),
                                    _ => {}
                                }
                            }
                            movers.push(MoverDef {
                                x: o.x,
                                y: o.y,
                                w: if o.width > 0.0 { o.width } else { 64.0 },
                                h: if o.height > 0.0 { o.height } else { 14.0 },
                                axis_x,
                                range,
                                speed,
                                phase,
                                crusher,
                            });
                        }
                        "spikes" => spikes.push(Aabb::new(
                            o.x,
                            o.y,
                            if o.width > 0.0 { o.width } else { 32.0 },
                            if o.height > 0.0 { o.height } else { 16.0 },
                        )),
                        "checkpoint" => checkpoints.push(Vec2::new(o.x, o.y)),
                        "coin" => coins.push(Vec2::new(
                            o.x + if o.width > 0.0 { o.width * 0.5 } else { 0.0 },
                            o.y + if o.height > 0.0 { o.height * 0.5 } else { 0.0 },
                        )),
                        "torch" => torches.push((o.x, o.y)),
                        "exit" => exit = Some(Vec2::new(o.x, o.y)),
                        _ => {}
                    }
                }
            }
            TiledLayer::Other => {}
        }
    }

    LevelData {
        world,
        spawn,
        enemy_spawns,
        movers,
        spikes,
        checkpoints,
        coins,
        torches,
        decor,
        exit,
    }
}

/// Все уровни игры, вкомпилированные в бинарник.
const LEVELS: [&str; 2] = [
    include_str!("../assets/level1.json"),
    include_str!("../assets/level2.json"),
];

/// Загрузить уровень по индексу (0 — «Тропа пепла», 1 — «Катакомбы»).
pub fn load_level(index: usize) -> LevelData {
    parse(
        LEVELS
            .get(index)
            .unwrap_or_else(|| panic!("level: нет уровня с индексом {index}")),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level1_parses_with_expected_content() {
        let lvl = load_level(0);
        assert_eq!(lvl.spawn.x, 96.0);
        assert_eq!(lvl.spawn.y, 410.0);
        assert_eq!(lvl.enemy_spawns.len(), 8, "8 врагов на уровне");
        assert!(lvl
            .enemy_spawns
            .iter()
            .any(|(k, _)| matches!(k, EnemyKind::DeathKnight)));
        assert!(lvl
            .enemy_spawns
            .iter()
            .any(|(k, _)| matches!(k, EnemyKind::Cultist)));
        assert_eq!(lvl.movers.len(), 4, "лифт, челнок и два крушителя");
        assert!(lvl.movers[2].crusher, "третья платформа — крушитель");
        assert!(lvl.movers[3].crusher, "четвёртая платформа — крушитель");
        assert!((lvl.movers[2].range - 154.0).abs() < 0.01, "ход крушителя 154 px");
        assert!((lvl.movers[3].phase - 0.5).abs() < 0.01, "крушители в противофазе");
        assert_eq!(lvl.spikes.len(), 2, "две зоны шипов");
        assert_eq!(lvl.checkpoints.len(), 2, "два чекпоинта");
        assert_eq!(lvl.coins.len(), 9, "девять монет");
        assert_eq!(lvl.torches.len(), 7, "семь факелов");
        assert!(lvl.decor.len() >= 16, "декор на месте");
        assert_eq!(lvl.exit, Some(Vec2::new(2976.0, 448.0)), "дверь у правой стены");
    }

    #[test]
    fn level2_catacombs_parse_with_expected_content() {
        let lvl = load_level(1);
        assert_eq!(lvl.spawn.x, 96.0);
        assert_eq!(lvl.enemy_spawns.len(), 9, "9 врагов во втором уровне");
        let bosses = lvl
            .enemy_spawns
            .iter()
            .filter(|(k, _)| matches!(k, EnemyKind::DeathKnight))
            .count();
        assert_eq!(bosses, 1, "финальный босс на арене");
        let crushers = lvl.movers.iter().filter(|m| m.crusher).count();
        assert_eq!(crushers, 2, "два крушителя в нижнем коридоре");
        assert!((lvl.movers[0].range - 90.0).abs() < 0.01);
        assert!((lvl.movers[1].phase - 0.5).abs() < 0.01, "крушители в противофазе");
        assert_eq!(lvl.spikes.len(), 2, "шипы в разломе и коридоре");
        assert_eq!(lvl.checkpoints.len(), 2, "зал и предарена");
        assert_eq!(lvl.coins.len(), 13, "13 монет, включая секретку");
        assert_eq!(lvl.torches.len(), 5);
        assert!(lvl.exit.is_none(), "финал второго уровня — победа, без двери");
        // Верхний ярус: пол на y = 8*TILE.
        assert!(lvl
            .world
            .overlaps_any(Aabb::new(50.0 * 32.0, 8.0 * 32.0 + 8.0, 4.0, 4.0)));
        // Дыра в пол над секретной полостью — воздух.
        assert!(!lvl
            .world
            .overlaps_any(Aabb::new(21.5 * 32.0, 14.0 * 32.0 + 8.0, 4.0, 4.0)));
    }

    #[test]
    fn level1_geometry_is_solid_where_expected() {
        let lvl = load_level(0);
        // Пол в стартовом зале — твёрдый.
        assert!(lvl
            .world
            .overlaps_any(Aabb::new(3.0 * 32.0, 14.0 * 32.0 + 8.0, 4.0, 4.0)));
        // Яма (x20..23) — пусто, падение в пустоту.
        assert!(!lvl
            .world
            .overlaps_any(Aabb::new(21.0 * 32.0 + 8.0, 14.0 * 32.0 + 8.0, 4.0, 4.0)));
        // Секретная полость — воздух, дно полости — твёрдое.
        assert!(!lvl
            .world
            .overlaps_any(Aabb::new(55.5 * 32.0, 16.0 * 32.0 + 8.0, 4.0, 4.0)));
        assert!(lvl
            .world
            .overlaps_any(Aabb::new(55.5 * 32.0, 18.0 * 32.0 + 8.0, 4.0, 4.0)));
        // Стена арены босса.
        assert!(lvl
            .world
            .overlaps_any(Aabb::new(90.0 * 32.0 + 8.0, 10.0 * 32.0, 4.0, 4.0)));
    }

    #[test]
    fn mover_defs_carry_tuned_properties() {
        let lvl = load_level(0);
        let shuttle = &lvl.movers[0];
        assert!(shuttle.axis_x, "челнок над ямой ходит по X");
        assert!((shuttle.range - 48.0).abs() < 0.01);
        let lift = &lvl.movers[1];
        assert!(!lift.axis_x, "лифт у стены ходит по Y");
        assert!((lift.range - 80.0).abs() < 0.01);
        assert!((lift.speed - 1.0).abs() < 0.01);
    }
}
