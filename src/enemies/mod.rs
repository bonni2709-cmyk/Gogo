//! ИИ врагов поверх лёгкого ECS ([`crate::ecs`]).
//!
//! Каждый враг — сущность с компонентами [`Transform`], [`Velocity`],
//! [`Health`] и [`EnemyBrain`] (конечный автомат конкретного вида).
//! [`EnemyWorld`] владеет хранилищами и отвечает за общую обвязку:
//!
//! * физику (гравитация для наземных, интеграция для летающих);
//! * агро-логику: дистанция + линия взгляда, «липкое» преследование;
//! * контактный урон и уборку погибших (`hp == 0` → despawn).
//!
//! Поведение видов изолировано в модулях [`slime`], [`bat`], [`skeleton`],
//! [`cultist`] (метатель ножей) и [`deathknight`] (босс-некрорыцарь).
//! Боевое оружие игрока подключается позже через [`EnemyWorld::damage`].

pub mod bat;
pub mod cultist;
pub mod deathknight;
pub mod skeleton;
pub mod slime;

use macroquad::color::Color;
use macroquad::math::Vec2;

use crate::ecs::{ComponentStore, Entity, EntityAllocator, Health, Transform, Velocity};
use crate::physics::{apply_gravity, Aabb, World, GRAVITY, MAX_FALL_SPEED};
use crate::player::Player;

// ---------------------------------------------------------------------------
// Виды и состояния
// ---------------------------------------------------------------------------

/// Вид врага — определяет параметры, FSM и отрисовку.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnemyKind {
    Slime,
    Bat,
    Skeleton,
    /// Культист: держит дистанцию и метает ножи.
    Cultist,
    /// Некрорыцарь-босс: тяжёлые выпады и ярость ниже половины HP.
    DeathKnight,
}

/// Состояния конечных автоматов врагов (общий набор для всех видов).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnemyState {
    /// Ходит взад-вперёд, разворачивается у стен и обрывов.
    Patrol,
    /// Преследует игрока.
    Chase,
    /// Парит у точки привязки (летучая мышь).
    Rest,
    /// Летит назад к точке привязки (летучая мышь).
    Return,
    /// Прыжок-выпад (слизень, в воздухе).
    Hop,
    /// Замах с телеграфом (скелет).
    Windup,
    /// Активная фаза выпада (скелет).
    Strike,
    /// Откат после выпада (скелет).
    Recover,
    /// Оглушение от полученного урона.
    Hurt,
}

/// Боевые параметры вида (пиксели, секунды).
#[derive(Clone, Copy, Debug)]
pub struct EnemyParams {
    pub size: Vec2,
    pub hp: u32,
    /// Урон при физическом контакте с игроком.
    pub contact_damage: u32,
    pub patrol_speed: f32,
    pub chase_speed: f32,
    /// Дистанция обнаружения игрока (при наличии линии взгляда).
    pub aggro_range: f32,
    /// Летает ли вид (нет гравитации).
    pub flying: bool,
}

/// Параметры вида.
pub fn params(kind: EnemyKind) -> EnemyParams {
    match kind {
        EnemyKind::Slime => EnemyParams {
            size: Vec2::new(26.0, 20.0),
            hp: 3,
            contact_damage: 1,
            patrol_speed: 55.0,
            chase_speed: 105.0,
            aggro_range: 125.0,
            flying: false,
        },
        EnemyKind::Bat => EnemyParams {
            size: Vec2::new(22.0, 14.0),
            hp: 2,
            contact_damage: 1,
            patrol_speed: 70.0,
            chase_speed: 128.0,
            aggro_range: 150.0,
            flying: true,
        },
        EnemyKind::Skeleton => EnemyParams {
            size: Vec2::new(26.0, 38.0),
            hp: 5,
            contact_damage: 1,
            patrol_speed: 60.0,
            chase_speed: 112.0,
            aggro_range: 145.0,
            flying: false,
        },
        EnemyKind::Cultist => EnemyParams {
            size: Vec2::new(24.0, 34.0),
            hp: 3,
            contact_damage: 1,
            patrol_speed: 50.0,
            chase_speed: 95.0,
            aggro_range: 210.0,
            flying: false,
        },
        EnemyKind::DeathKnight => EnemyParams {
            size: Vec2::new(40.0, 56.0),
            hp: 24,
            contact_damage: 2,
            patrol_speed: 40.0,
            chase_speed: 90.0,
            aggro_range: 260.0,
            flying: false,
        },
    }
}

/// Длительности фаз FSM (секунды).
pub(crate) mod timing {
    /// Оглушение после урона.
    pub const HURT: f32 = 0.3;
    /// Замах скелета (телеграф перед ударом).
    pub const WINDUP: f32 = 0.45;
    /// Активная фаза выпада.
    pub const STRIKE: f32 = 0.14;
    /// Откат после выпада.
    pub const RECOVER: f32 = 0.55;
    /// Кулдаун следующего выпада.
    pub const ATTACK_COOLDOWN: f32 = 0.9;
    /// Кулдаун прыжка-выпада слизня.
    pub const HOP_COOLDOWN: f32 = 1.1;
    /// Сколько секунд без видимости враг продолжает преследование.
    pub const FORGET: f32 = 2.4;
    /// Замах культиста перед броском ножа.
    pub const THROW_WINDUP: f32 = 0.40;
    /// Кадр броска.
    pub const THROW_STRIKE: f32 = 0.10;
    /// Откат после броска.
    pub const THROW_RECOVER: f32 = 0.45;
    /// Кулдаун следующего броска.
    pub const THROW_COOLDOWN: f32 = 1.15;
    /// Замах некрорыцаря (длиннее обычного скелета — босс читается).
    pub const BOSS_WINDUP: f32 = 0.63;
    /// Откат после выпада босса.
    pub const BOSS_RECOVER: f32 = 0.66;
    /// Кулдаун выпада босса.
    pub const BOSS_COOLDOWN: f32 = 0.95;
}

/// Мозг врага: конечный автомат + «память» преследования.
#[derive(Clone, Copy, Debug)]
pub struct EnemyBrain {
    pub kind: EnemyKind,
    pub state: EnemyState,
    /// Время в текущем состоянии, с (сбрасывается при переходе).
    pub state_timer: f32,
    /// Направление патруля/движения: `-1.0` или `1.0`.
    pub dir: f32,
    /// Включено ли преследование (липкое: держится [`timing::FORGET`].
    pub aggro: bool,
    /// Сколько секунд осталось помнить цель без видимости.
    pub forget_timer: f32,
    /// Точка привязки (для летучей мыши — место парения).
    pub anchor: Vec2,
    /// Фаза волны полёта (для анимации и маневров).
    pub wobble: f32,
    /// Стоит ли на земле (обновляется физикой).
    pub on_ground: bool,
    /// Общий кулдаун способности (прыжок слизня, выпад скелета).
    pub cooldown: f32,
    /// Запрошен ли бросок ножа (культист ставит флаг, [`EnemyWorld`] создаёт снаряд).
    pub want_throw: bool,
    /// Ярость босса (ниже половины HP): ускоренные фазы и более злые глаза.
    pub enraged: bool,
}

impl EnemyBrain {
    fn new(kind: EnemyKind, anchor: Vec2) -> Self {
        let resting = match kind {
            EnemyKind::Bat => EnemyState::Rest,
            _ => EnemyState::Patrol,
        };
        Self {
            kind,
            state: resting,
            state_timer: 0.0,
            dir: -1.0,
            aggro: false,
            forget_timer: 0.0,
            anchor,
            wobble: 0.0,
            on_ground: false,
            cooldown: 0.0,
            want_throw: false,
            enraged: false,
        }
    }

    /// Перейти в состояние, сбросив таймер фазы.
    pub(crate) fn transition(&mut self, next: EnemyState) {
        self.state = next;
        self.state_timer = 0.0;
    }
}

/// Снимок цели для FSM: позиция игрока, дистанция и видимость.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TargetInfo {
    pub center: Vec2,
    pub dist: f32,
    pub los: bool,
}

/// Контактное попадание по игроку за текущий шаг.
#[derive(Clone, Copy, Debug)]
pub struct ContactHit {
    pub entity: Entity,
    pub damage: u32,
    /// Центр врага — для направления отбрасывания.
    pub center: Vec2,
}

/// Летящий нож культиста (обычный снарядный объект без гравитации).
#[derive(Clone, Copy, Debug)]
pub struct Knife {
    /// Верхний левый угол хитбокса 6×4 в мировых единицах.
    pub pos: Vec2,
    pub vel: Vec2,
    /// Остаток жизни, с.
    pub life: f32,
    /// Владелец (для attribution контакта).
    pub owner: Entity,
    pub damage: u32,
}

impl Knife {
    /// Хитбокс снаряда.
    pub fn body(&self) -> Aabb {
        Aabb::new(self.pos.x, self.pos.y, 6.0, 4.0)
    }
}

/// Результат одного шага симуляции врагов.
#[derive(Default)]
pub struct EnemyUpdate {
    /// Контакты, задевшие игроку (учитывает i-frames сам игрок).
    pub contacts: Vec<ContactHit>,
    /// Погибшие на этом шаге: центр взрыва и вид (для частиц).
    pub deaths: Vec<(Vec2, EnemyKind)>,
}

// ---------------------------------------------------------------------------
// Хранилище врагов
// ---------------------------------------------------------------------------

/// Мир врагов: ECS-хранилища + общая обвязка обновления.
pub struct EnemyWorld {
    alloc: EntityAllocator,
    pub(crate) transforms: ComponentStore<Transform>,
    pub(crate) velocities: ComponentStore<Velocity>,
    pub(crate) healths: ComponentStore<Health>,
    pub(crate) brains: ComponentStore<EnemyBrain>,
    /// Летящие ножи культистов (обычный Vec — снарядов мало, пул избыточен).
    knives: Vec<Knife>,
}

impl EnemyWorld {
    pub fn new() -> Self {
        Self {
            alloc: EntityAllocator::new(),
            transforms: ComponentStore::new(),
            velocities: ComponentStore::new(),
            healths: ComponentStore::new(),
            brains: ComponentStore::new(),
            knives: Vec::new(),
        }
    }

    /// Заспавить врага вида `kind` с левым верхним углом `pos`.
    pub fn spawn(&mut self, kind: EnemyKind, pos: Vec2) -> Entity {
        let p = params(kind);
        let entity = self.alloc.spawn();
        let _ = self.transforms.insert(
            entity,
            Transform {
                pos,
                size: p.size,
                facing: -1.0,
            },
        );
        let _ = self.velocities.insert(entity, Velocity(Vec2::ZERO));
        let _ = self.healths.insert(entity, Health::new(p.hp));
        let _ = self
            .brains
            .insert(entity, EnemyBrain::new(kind, pos + p.size * 0.5));
        entity
    }

    /// Убрать врага из всех хранилищ.
    pub fn despawn(&mut self, entity: Entity) {
        let _ = self.transforms.remove(entity);
        let _ = self.velocities.remove(entity);
        let _ = self.healths.remove(entity);
        let _ = self.brains.remove(entity);
        self.alloc.despawn(entity);
    }

    pub fn is_alive(&self, entity: Entity) -> bool {
        self.alloc.is_alive(entity) && self.brains.contains(entity)
    }

    pub fn health(&self, entity: Entity) -> Option<&Health> {
        self.healths.get(entity)
    }

    /// Нанести урон врагу. `knock_dir` — куда отталкивать (`-1`/`1`).
    /// Возвращает `true`, если урон применён. Смерть (`hp == 0`)
    /// оформляется при следующем [`EnemyWorld::update`] (despawn).
    pub fn damage(&mut self, entity: Entity, amount: u32, knock_dir: f32) -> bool {
        if !self.is_alive(entity) {
            return false;
        }
        let Some(health) = self.healths.get_mut(entity) else {
            return false;
        };
        if health.hp == 0 {
            return false;
        }
        health.hp = health.hp.saturating_sub(amount);
        health.hit_flash = 0.15;

        let Some(brain) = self.brains.get_mut(entity) else {
            return false;
        };
        brain.transition(EnemyState::Hurt);
        brain.aggro = true;
        brain.forget_timer = timing::FORGET;

        let p = params(brain.kind);
        if let Some(vel) = self.velocities.get_mut(entity) {
            vel.0.x = knock_dir * if p.flying { 240.0 } else { 190.0 };
            if !p.flying {
                vel.0.y = -230.0;
            }
        }
        true
    }

    /// Шаг симуляции всех врагов (фиксированный `dt`).
    ///
    /// Возвращает контакты с игроком (i-frames решает сам игрок) и
    /// список погибших — для пиксельного взрыва частицами.
    pub fn update(&mut self, world: &World, player: &Player, dt: f32) -> EnemyUpdate {
        let p_body = player.body();

        let entities: Vec<Entity> = self.brains.entities().collect();
        let mut hits = Vec::new();
        let mut deaths = Vec::new();
        let mut dead = Vec::new();

        for entity in entities {
            let (Some(brain), Some(transform), Some(vel), Some(health)) = (
                self.brains.get_mut(entity),
                self.transforms.get_mut(entity),
                self.velocities.get_mut(entity),
                self.healths.get_mut(entity),
            ) else {
                continue;
            };

            // --- общие таймеры ---
            brain.state_timer += dt;
            brain.wobble += dt;
            brain.forget_timer -= dt;
            brain.cooldown -= dt;
            health.hit_flash = (health.hit_flash - dt).max(0.0);

            // --- агро: дистанция + линия взгляда, «липкое» запоминание ---
            let center = transform.center();
            let dist = center.distance(p_body.center());
            let p = params(brain.kind);
            let los = line_of_sight(world, center, p_body.center());
            if dist <= p.aggro_range && los {
                if !brain.aggro {
                    brain.aggro = true;
                }
                brain.forget_timer = timing::FORGET;
            } else if brain.forget_timer <= 0.0 {
                brain.aggro = false;
            }

            let info = TargetInfo {
                center: p_body.center(),
                dist,
                los,
            };

            // --- ярость босса: ниже половины максимума HP ---
            if brain.kind == EnemyKind::DeathKnight
                && !brain.enraged
                && health.hp * 2 <= p.hp
            {
                brain.enraged = true;
            }

            // --- FSM конкретного вида ---
            match brain.kind {
                EnemyKind::Slime => slime::update(brain, transform, vel, world, &info, dt),
                EnemyKind::Bat => bat::update(brain, transform, vel, world, &info, dt),
                EnemyKind::Skeleton => skeleton::update(brain, transform, vel, world, &info, dt),
                EnemyKind::Cultist => cultist::update(brain, transform, vel, world, &info, dt),
                EnemyKind::DeathKnight => {
                    deathknight::update(brain, transform, vel, world, &info, dt)
                }
            }

            // --- бросок культиста: флаг FSM → снаряд в мире ---
            if brain.want_throw {
                brain.want_throw = false;
                let dir = if transform.facing != 0.0 {
                    transform.facing
                } else {
                    brain.dir
                };
                let c = transform.center();
                self.knives.push(Knife {
                    pos: Vec2::new(
                        c.x + dir * (transform.size.x * 0.5 + 3.0),
                        c.y - 6.0,
                    ),
                    vel: Vec2::new(dir * 265.0, -12.0),
                    life: 2.4,
                    owner: entity,
                    damage: 1,
                });
            }

            // --- физика ---
            if p.flying {
                // Летающие не падают, но упираются в стены и потолок.
                let (pos, new_vel, hit) = world.move_body(transform.pos, transform.size, vel.0, dt);
                transform.pos = pos;
                vel.0 = new_vel;
                if hit.wall_dir != 0.0 {
                    vel.0.x = 0.0;
                }
                if hit.hit_ceiling {
                    vel.0.y = 0.0;
                }
                // Лица в сторону полёта.
                if vel.0.x.abs() > 8.0 {
                    transform.facing = vel.0.x.signum();
                }
            } else {
                apply_gravity(&mut vel.0, dt, GRAVITY, MAX_FALL_SPEED);
                let (pos, new_vel, hit) = world.move_body(transform.pos, transform.size, vel.0, dt);
                transform.pos = pos;
                vel.0 = new_vel;
                brain.on_ground = hit.on_ground;
                // Упёрлись в стену при ходьбе — разворот (патруль/погоня).
                if hit.wall_dir != 0.0
                    && matches!(brain.state, EnemyState::Patrol | EnemyState::Chase)
                {
                    brain.dir = -hit.wall_dir;
                    vel.0.x = 0.0;
                }
                transform.facing = brain.dir;
            }

            // --- смерть и контактный урон ---
            if health.hp == 0 {
                deaths.push((transform.center(), brain.kind));
                dead.push(entity);
                continue;
            }
            if transform.body().overlaps(&p_body) {
                hits.push(ContactHit {
                    entity,
                    damage: p.contact_damage,
                    center: transform.center(),
                });
            }
        }

        // --- полёт ножей: столкновение со стеной и игроком ---
        let mut i = 0;
        while i < self.knives.len() {
            let knife = self.knives[i];
            let mut remove = knife.life <= 0.0;
            if !remove {
                let next = knife.pos + knife.vel * dt;
                let swept = Aabb::new(
                    knife.pos.x.min(next.x) - 1.0,
                    knife.pos.y.min(next.y) - 1.0,
                    (next.x - knife.pos.x).abs() + 8.0,
                    (next.y - knife.pos.y).abs() + 6.0,
                );
                if world.overlaps_any(swept) {
                    remove = true;
                } else {
                    self.knives[i].pos = next;
                    self.knives[i].life -= dt;
                    if self.knives[i].body().overlaps(&p_body) {
                        hits.push(ContactHit {
                            entity: knife.owner,
                            damage: knife.damage,
                            center: self.knives[i].pos,
                        });
                        remove = true;
                    }
                }
            }
            if remove {
                self.knives.swap_remove(i);
            } else {
                i += 1;
            }
        }

        for entity in dead {
            self.despawn(entity);
        }
        EnemyUpdate {
            contacts: hits,
            deaths,
        }
    }

    /// Летящие ножи (для отрисовки).
    pub fn knives(&self) -> &[Knife] {
        &self.knives
    }

    /// Обход живых врагов (для отрисовки и будущей боевой системы).
    pub fn iter(&self) -> impl Iterator<Item = (Entity, &Transform, &EnemyBrain, &Health)> + '_ {
        self.brains.iter().filter_map(move |(entity, brain)| {
            let transform = self.transforms.get(entity)?;
            let health = self.healths.get(entity)?;
            Some((entity, transform, brain, health))
        })
    }

    pub fn len(&self) -> usize {
        self.brains.len()
    }

    pub fn is_empty(&self) -> bool {
        self.brains.is_empty()
    }
}

impl Default for EnemyWorld {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Общие хелперы ИИ
// ---------------------------------------------------------------------------

/// Прямая видимость: луч от `from` к `to` не пересекает стены раньше цели.
pub fn line_of_sight(world: &World, from: Vec2, to: Vec2) -> bool {
    let delta = to - from;
    let dist = delta.length();
    if dist < 1.0 {
        return true;
    }
    match world.raycast(from, delta) {
        None => true,
        // Цель ближе или на самом луче — преграды нет.
        Some(hit) => hit.dist >= dist - 4.0,
    }
}

/// Есть ли обрыв перед передним краем тела (нет опоры под носом).
pub fn ledge_ahead(world: &World, t: &Transform, dir: f32) -> bool {
    let foot_x = if dir > 0.0 {
        t.pos.x + t.size.x + 2.0
    } else {
        t.pos.x - 2.0
    };
    let origin = Vec2::new(foot_x, t.pos.y + t.size.y - 2.0);
    world.raycast(origin, Vec2::new(0.0, 12.0)).is_none()
}

/// Есть ли стена перед телом (короткий горизонтальный пробник).
pub fn wall_ahead(world: &World, t: &Transform, dir: f32) -> bool {
    let probe = if dir > 0.0 {
        Aabb {
            pos: Vec2::new(t.pos.x + t.size.x, t.pos.y + 6.0),
            size: Vec2::new(4.0, t.size.y - 12.0),
        }
    } else {
        Aabb {
            pos: Vec2::new(t.pos.x - 4.0, t.pos.y + 6.0),
            size: Vec2::new(4.0, t.size.y - 12.0),
        }
    };
    world.overlaps_any(probe)
}

/// Смешать цвет с белым на `k ∈ [0..=1]` (вспышка при уроне).
pub(crate) fn flash_color(base: Color, hit_flash: f32) -> Color {
    if hit_flash <= 0.0 {
        return base;
    }
    let k = (hit_flash / 0.15).clamp(0.0, 1.0) * 0.85;
    Color::new(
        base.r + (1.0 - base.r) * k,
        base.g + (1.0 - base.g) * k,
        base.b + (1.0 - base.b) * k,
        base.a,
    )
}

/// Палитра врагов: землистые оттенки классического тёмного фэнтези.
pub(crate) mod palette {
    use macroquad::color::Color;

    /// Слизь — моховато-зелёный.
    pub const SLIME: Color = Color::new(0.302, 0.373, 0.196, 1.0);
    pub const SLIME_DARK: Color = Color::new(0.184, 0.235, 0.118, 1.0);
    pub const SLIME_BELLY: Color = Color::new(0.420, 0.478, 0.263, 1.0);
    /// Влажный блик сверху.
    pub const SLIME_GLOSS: Color = Color::new(0.573, 0.639, 0.396, 1.0);
    /// Кожа летучей мыши (подсветка меха).
    pub const BAT_FUR: Color = Color::new(0.278, 0.216, 0.176, 1.0);
    /// Рваная ткань/трапеция на костяных врагах.
    pub const TATTER: Color = Color::new(0.353, 0.235, 0.157, 1.0);
    /// Летучая мышь — тёмно-коричневый.
    pub const BAT: Color = Color::new(0.161, 0.125, 0.106, 1.0);
    pub const BAT_WING: Color = Color::new(0.231, 0.180, 0.149, 1.0);
    pub const BAT_EYE: Color = Color::new(0.627, 0.435, 0.235, 1.0);
    /// Кость и сталь скелета.
    pub const BONE: Color = Color::new(0.780, 0.741, 0.639, 1.0);
    pub const BONE_DARK: Color = Color::new(0.549, 0.510, 0.427, 1.0);
    pub const SOCKET: Color = Color::new(0.086, 0.075, 0.063, 1.0);
    pub const RUST: Color = Color::new(0.478, 0.294, 0.184, 1.0);
    pub const STEEL: Color = Color::new(0.659, 0.627, 0.592, 1.0);
    pub const STEEL_DARK: Color = Color::new(0.431, 0.408, 0.380, 1.0);
    pub const SHADOW: Color = Color::new(0.0, 0.0, 0.0, 0.35);
    /// Обводка силуэтов (почти чёрный тёплый).
    pub const OUTLINE: Color = Color::new(0.055, 0.047, 0.039, 1.0);
    /// Роба культиста — тёмный марун (выцветшая ткань культа).
    pub const CULTIST_ROBE: Color = Color::new(0.361, 0.173, 0.145, 1.0);
    pub const CULTIST_DARK: Color = Color::new(0.208, 0.098, 0.086, 1.0);
    /// Костяная маска под капюшоном.
    pub const CULTIST_MASK: Color = Color::new(0.722, 0.678, 0.573, 1.0);
    /// Пластины некрорыцаря — холодная притушенная сталь.
    pub const BOSS_PLATE: Color = Color::new(0.318, 0.306, 0.337, 1.0);
    pub const BOSS_PLATE_DARK: Color = Color::new(0.192, 0.184, 0.208, 1.0);
    /// Выцветший тёмно-красный плащ босса.
    pub const BOSS_CAPE: Color = Color::new(0.322, 0.145, 0.118, 1.0);
    /// Тлеющие глаза-угольки.
    pub const BOSS_EYE: Color = Color::new(0.851, 0.420, 0.176, 1.0);
    /// Тусклое золото (пряжки, заклёпки).
    pub const GOLD: Color = Color::new(0.710, 0.553, 0.263, 1.0);
}

/// Цвета «пиксельного взрыва» при смерти вида.
pub fn death_palette(kind: EnemyKind) -> [Color; 4] {
    use palette::*;
    match kind {
        EnemyKind::Slime => [SLIME, SLIME_BELLY, SLIME_DARK, SLIME_GLOSS],
        EnemyKind::Bat => [BAT, BAT_WING, BAT_FUR, TATTER],
        EnemyKind::Skeleton => [BONE, BONE_DARK, STEEL, RUST],
        EnemyKind::Cultist => [CULTIST_ROBE, CULTIST_DARK, CULTIST_MASK, STEEL],
        EnemyKind::DeathKnight => [BOSS_PLATE, BOSS_CAPE, BONE, BOSS_EYE],
    }
}

/// Отрисовать всех живых врагов (в детерминированном порядке).
pub fn draw_enemies(enemies: &EnemyWorld, time: f32) {
    // Ножи — под телами (снаряд летит по земле/на уровне пояса).
    for knife in enemies.knives() {
        let c = knife.body().center();
        let dir = knife.vel.x.signum();
        let tip_x = c.x + dir * 4.0;
        let tail_x = c.x - dir * 4.0;
        // Обводка и сталь клинка.
        macroquad::shapes::draw_line(tail_x, c.y, tip_x, c.y, 4.0, palette::OUTLINE);
        macroquad::shapes::draw_line(tail_x, c.y, tip_x, c.y, 2.0, palette::STEEL);
        // Рукоять сзади.
        macroquad::shapes::draw_rectangle(
            tail_x - dir * 2.0 - 1.0,
            c.y - 2.0,
            3.0,
            4.0,
            palette::TATTER,
        );
        // Блик острия.
        macroquad::shapes::draw_rectangle(tip_x - 1.0, c.y - 1.0, 2.0, 1.0, palette::STEEL);
    }

    for (_entity, transform, brain, health) in enemies.iter() {
        match brain.kind {
            EnemyKind::Slime => slime::draw(transform, brain, health, time),
            EnemyKind::Bat => bat::draw(transform, brain, health, time),
            EnemyKind::Skeleton => skeleton::draw(transform, brain, health, time),
            EnemyKind::Cultist => cultist::draw(transform, brain, health, time),
            EnemyKind::DeathKnight => deathknight::draw(transform, brain, health, time),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::player::Player;

    fn flat_world(width: f32) -> (World, Vec2) {
        let mut world = World::new();
        world.add_solid(Aabb::new(-width, 300.0, width * 2.0, 64.0));
        (world, Vec2::new(0.0, 300.0 - 40.0))
    }

    fn grounded_player(pos: Vec2) -> Player {
        Player::new(pos)
    }

    #[test]
    fn line_of_sight_blocked_by_wall() {
        let mut world = World::new();
        world.add_solid(Aabb::new(100.0, 0.0, 32.0, 400.0));
        assert!(!line_of_sight(&world, Vec2::new(0.0, 200.0), Vec2::new(300.0, 200.0)));
        assert!(line_of_sight(&world, Vec2::new(0.0, 50.0), Vec2::new(60.0, 50.0)));
    }

    #[test]
    fn ledge_probe_detects_empty_space() {
        // Пол заканчивается на x = 100.
        let mut world = World::new();
        world.add_solid(Aabb::new(-100.0, 300.0, 200.0, 64.0));
        let on_floor = Transform {
            pos: Vec2::new(0.0, 280.0),
            size: Vec2::new(26.0, 20.0),
            facing: 1.0,
        };
        assert!(!ledge_ahead(&world, &on_floor, 1.0), "под носом пол есть");

        let at_edge = Transform {
            pos: Vec2::new(80.0, 280.0),
            size: Vec2::new(26.0, 20.0),
            facing: 1.0,
        };
        assert!(ledge_ahead(&world, &at_edge, 1.0), "впереди обрыв");
    }

    #[test]
    fn slime_aggro_switches_to_chase() {
        let (world, spawn) = flat_world(400.0);
        let mut enemies = EnemyWorld::new();
        let slime = enemies.spawn(EnemyKind::Slime, Vec2::new(0.0, 280.0));
        let mut player = grounded_player(spawn);

        // Далеко — патруль.
        player.pos = Vec2::new(500.0, 260.0);
        for _ in 0..10 {
            enemies.update(&world, &player, crate::physics::FIXED_DT);
        }
        let brain = enemies.brains.get(slime).unwrap();
        assert_eq!(brain.state, EnemyState::Patrol);
        assert!(!brain.aggro);

        // Игрок рядом на той же линии — агро и погоня.
        player.pos = Vec2::new(60.0, 260.0);
        for _ in 0..10 {
            enemies.update(&world, &player, crate::physics::FIXED_DT);
        }
        let brain = enemies.brains.get(slime).unwrap();
        assert!(brain.aggro, "слизень должен заметить игрока");
        // Сразу после агро — погоня; близко к игроку может уже прыгнуть.
        assert!(
            matches!(brain.state, EnemyState::Chase | EnemyState::Hop),
            "ожидалась погоня, состояние {:?}",
            brain.state
        );
    }

    #[test]
    fn bat_wakes_near_player_and_rests_again() {
        let (world, spawn) = flat_world(400.0);
        let mut enemies = EnemyWorld::new();
        let bat = enemies.spawn(EnemyKind::Bat, Vec2::new(0.0, 150.0));
        let mut player = grounded_player(spawn);
        player.pos = Vec2::new(600.0, 260.0);

        for _ in 0..10 {
            enemies.update(&world, &player, crate::physics::FIXED_DT);
        }
        assert_eq!(enemies.brains.get(bat).unwrap().state, EnemyState::Rest);

        // Игрок под летучей мышью — та просыпается.
        player.pos = Vec2::new(0.0, 260.0);
        for _ in 0..10 {
            enemies.update(&world, &player, crate::physics::FIXED_DT);
        }
        assert_eq!(enemies.brains.get(bat).unwrap().state, EnemyState::Chase);

        // Далеко и без видимости дольше FORGET — возврат к точке парения.
        player.pos = Vec2::new(900.0, 260.0);
        let steps = (timing::FORGET + 1.0) / crate::physics::FIXED_DT;
        for _ in 0..steps as u32 {
            enemies.update(&world, &player, crate::physics::FIXED_DT);
        }
        let brain = enemies.brains.get(bat).unwrap();
        assert!(!brain.aggro, "цель потеряна");
        assert!(matches!(brain.state, EnemyState::Return | EnemyState::Rest));
    }

    #[test]
    fn skeleton_attack_chain_windup_strike_recover() {
        let (world, spawn) = flat_world(400.0);
        let mut enemies = EnemyWorld::new();
        let sk = enemies.spawn(EnemyKind::Skeleton, Vec2::new(0.0, 262.0));
        let player = grounded_player(spawn);
        let dt = crate::physics::FIXED_DT;

        // Принудительно ставим замах — FSM должен пройти цепочку фаз.
        {
            let brain = enemies.brains.get_mut(sk).unwrap();
            brain.transition(EnemyState::Windup);
            brain.aggro = true;
            brain.forget_timer = 10.0;
        }
        let mut saw_strike = false;
        let mut saw_recover = false;
        for _ in 0..120 {
            enemies.update(&world, &player, dt);
            let state = enemies.brains.get(sk).unwrap().state;
            if state == EnemyState::Strike {
                saw_strike = true;
            }
            if saw_strike && state == EnemyState::Recover {
                saw_recover = true;
                break;
            }
        }
        assert!(saw_strike && saw_recover, "замах → удар → откат");
    }

    #[test]
    fn contact_hit_and_player_iframes() {
        let (world, spawn) = flat_world(400.0);
        let mut enemies = EnemyWorld::new();
        enemies.spawn(EnemyKind::Slime, Vec2::new(0.0, 280.0));
        let mut player = grounded_player(spawn);
        // Игрок стоит там же, где слизень (на полу y=260).
        player.pos = Vec2::new(2.0, 260.0);

        let hits1 = enemies.update(&world, &player, crate::physics::FIXED_DT);
        assert_eq!(hits1.contacts.len(), 1);
        assert!(player.take_damage(hits1.contacts[0].damage, -1.0));
        assert_eq!(player.hp, player.max_hp - 1);

        // Второй контакт в кадр i-frames не проходит.
        let hits2 = enemies.update(&world, &player, crate::physics::FIXED_DT);
        if let Some(hit) = hits2.contacts.first() {
            assert!(!player.take_damage(hit.damage, -1.0), "i-frames");
        }
        assert_eq!(player.hp, player.max_hp - 1);
    }

    #[test]
    fn zero_hp_enemy_is_despawned_on_update() {
        let (world, spawn) = flat_world(400.0);
        let mut enemies = EnemyWorld::new();
        let slime = enemies.spawn(EnemyKind::Slime, Vec2::new(0.0, 280.0));
        let player = grounded_player(spawn);

        assert!(enemies.damage(slime, 99, -1.0));
        assert_eq!(enemies.health(slime).unwrap().hp, 0);
        let step = enemies.update(&world, &player, crate::physics::FIXED_DT);
        assert_eq!(step.deaths.len(), 1, "смерть должна отдаваться наружу");
        assert_eq!(step.deaths[0].1, EnemyKind::Slime);
        assert!(!enemies.is_alive(slime));
        assert!(enemies.is_empty());
        assert!(!enemies.damage(slime, 1, -1.0), "мёртвому — не бьёт");
    }

    #[test]
    fn slime_patrol_turns_at_ledge_and_stays_alive() {
        let mut world = World::new();
        // Пол заканчивается на x = 160.
        world.add_solid(Aabb::new(-200.0, 300.0, 360.0, 64.0));
        let mut enemies = EnemyWorld::new();
        let slime = enemies.spawn(EnemyKind::Slime, Vec2::new(100.0, 280.0));
        let mut player = Player::new(Vec2::new(-150.0, 260.0));
        player.pos = Vec2::new(-150.0, 260.0); // далеко: агро не включается

        let start_x = enemies.transforms.get(slime).unwrap().pos.x;
        for _ in 0..600 {
            enemies.update(&world, &player, crate::physics::FIXED_DT);
        }
        let after = enemies.transforms.get(slime).unwrap().pos;
        assert!(
            after.y < 300.0,
            "слизень не должен упасть с обрыва (y = {})",
            after.y
        );
        assert!(after.x <= start_x + 40.0, "патруль держится у обрыва");
        assert!(enemies.is_alive(slime));
    }

    /// Культист в полосе броска: агро → замах → ножлетит в игрока.
    #[test]
    fn cultist_throws_knife_at_range() {
        let (world, spawn) = flat_world(400.0);
        let mut enemies = EnemyWorld::new();
        let cultist = enemies.spawn(EnemyKind::Cultist, Vec2::new(0.0, spawn.y - 6.0));
        let player = Player::new(Vec2::new(160.0, spawn.y));

        let mut knife_seen = false;
        let mut knife_hit = false;
        for _ in 0..360 {
            let step = enemies.update(&world, &player, crate::physics::FIXED_DT);
            if !enemies.knives().is_empty() {
                knife_seen = true;
            }
            if !step.contacts.is_empty() {
                knife_hit = true;
                break;
            }
        }
        assert!(knife_seen, "культист должен выбросить нож");
        assert!(knife_hit, "нож должен долететь до игрока");
        assert!(enemies.is_alive(cultist), "бросок не должен вредить самому себе");
    }

    /// Вблизи игрока культист отступает, а не лезет в упор.
    #[test]
    fn cultist_retreats_when_player_close() {
        let (world, spawn) = flat_world(400.0);
        let mut enemies = EnemyWorld::new();
        let start_x = -50.0;
        let cultist = enemies.spawn(EnemyKind::Cultist, Vec2::new(start_x, spawn.y - 6.0));
        let player = Player::new(Vec2::new(30.0, spawn.y));

        for _ in 0..60 {
            enemies.update(&world, &player, crate::physics::FIXED_DT);
        }
        let t = enemies.transforms.get(cultist).unwrap();
        assert!(
            t.pos.x < start_x - 10.0,
            "культист обязан отступать от близкого игрока (x={})",
            t.pos.x
        );
    }

    /// Босс впадает в ярость ниже половины HP.
    #[test]
    fn deathknight_enrages_below_half_hp() {
        let (world, spawn) = flat_world(400.0);
        let mut enemies = EnemyWorld::new();
        let boss = enemies.spawn(EnemyKind::DeathKnight, Vec2::new(0.0, spawn.y - 16.0));
        let player = Player::new(Vec2::new(300.0, spawn.y));

        let p = params(EnemyKind::DeathKnight);
        assert_eq!(p.hp, 24, "у босса24 HP");
        assert_eq!(p.contact_damage, 2, "контакт босса бьёт вдвойне");

        // 24 → 11 HP: ниже половины.
        assert!(enemies.damage(boss, 13, 1.0));
        enemies.update(&world, &player, crate::physics::FIXED_DT);
        let brain = enemies.brains.get(boss).unwrap();
        assert!(brain.enraged, "ниже половины HP должна включиться ярость");
    }
}
