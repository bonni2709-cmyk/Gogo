//! Ядро физики VOXEL KNIGHT.
//!
//! * AABB-коллизии с **разделением осей**: сначала X (и разрешение по X),
//!   затем Y — игрок не застревает в углах платформ.
//! * Подшаги: перемещение разбивается на куски ≤ [`MAX_STEP_DISTANCE`],
//!   поэтому рывок и падение не «проскакивают» сквозь стены.
//! * Broad-phase на равномерной сетке 64×64 — тайловые уровни обходятся
//!   без перебора всех прямоугольников.
//! * Рейкасты (slab-метод) для будущего ИИ врагов: обнаружение обрывов,
//!   стен и линии взгляда.

use macroquad::math::Vec2;
use std::collections::HashMap;

/// Фиксированный шаг симуляции — 60 Гц.
pub const FIXED_DT: f32 = 1.0 / 60.0;

/// Базовая гравитация, px/с².
pub const GRAVITY: f32 = 2400.0;

/// Предельная скорость падения, px/с.
pub const MAX_FALL_SPEED: f32 = 900.0;

/// Максимальное перемещение тела за один подшаг (px).
/// Меньше размера тайла (32 px), поэтому тело физически не может
/// пересечь стену целиком за один подшаг.
pub const MAX_STEP_DISTANCE: f32 = 8.0;

/// Размер ячейки broad-phase сетки (px).
const CELL_SIZE: f32 = 64.0;

/// Прямоугольник в мировых координатах (Y вниз).
/// `pos` — левый верхний угол.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aabb {
    pub pos: Vec2,
    pub size: Vec2,
}

impl Aabb {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self {
            pos: Vec2::new(x, y),
            size: Vec2::new(w, h),
        }
    }

    pub fn from_center(center: Vec2, size: Vec2) -> Self {
        Self {
            pos: center - size * 0.5,
            size,
        }
    }

    #[inline]
    pub fn left(&self) -> f32 {
        self.pos.x
    }

    #[inline]
    pub fn right(&self) -> f32 {
        self.pos.x + self.size.x
    }

    #[inline]
    pub fn top(&self) -> f32 {
        self.pos.y
    }

    #[inline]
    pub fn bottom(&self) -> f32 {
        self.pos.y + self.size.y
    }

    #[inline]
    pub fn center(&self) -> Vec2 {
        self.pos + self.size * 0.5
    }

    /// Строгое пересечение: касание гранями столкновением не считается.
    #[inline]
    pub fn overlaps(&self, other: &Aabb) -> bool {
        self.left() < other.right()
            && self.right() > other.left()
            && self.top() < other.bottom()
            && self.bottom() > other.top()
    }

    pub fn expanded(&self, margin: f32) -> Aabb {
        Aabb {
            pos: self.pos - Vec2::splat(margin),
            size: self.size + Vec2::splat(margin * 2.0),
        }
    }

    pub fn translated(&self, delta: Vec2) -> Aabb {
        Aabb {
            pos: self.pos + delta,
            size: self.size,
        }
    }

    #[inline]
    pub fn contains(&self, point: Vec2) -> bool {
        point.x >= self.left()
            && point.x <= self.right()
            && point.y >= self.top()
            && point.y <= self.bottom()
    }
}

/// Результат одного шага [`World::move_body`].
#[derive(Clone, Copy, Debug, Default)]
pub struct CollisionHit {
    /// Тело опирается на поверхность.
    pub on_ground: bool,
    /// Тело ударилось о потолок.
    pub hit_ceiling: bool,
    /// Направление задетой стены: `-1` — слева, `1` — справа, `0` — нет контакта.
    pub wall_dir: f32,
}

/// Попадание луча в твёрдое тело.
#[derive(Clone, Copy, Debug)]
pub struct RayHit {
    /// Точка удара.
    pub point: Vec2,
    /// Нормаль поверхности (противоположна направлению луча).
    pub normal: Vec2,
    /// Параметр вдоль переданного отрезка: `0.0 ..= 1.0`.
    pub t: f32,
    /// Расстояние от `origin` до точки удара.
    pub dist: f32,
}

/// Игровой мир: набор неподвижных твёрдых тел (тайлы, платформы, стены)
/// с broad-phase на равномерной сетке.
#[derive(Default)]
pub struct World {
    solids: Vec<Aabb>,
    cells: HashMap<(i32, i32), Vec<u32>>,
}

impl World {
    pub fn new() -> Self {
        Self::default()
    }

    /// Добавить неподвижное тело (тайл/объединённый блок тайлов).
    pub fn add_solid(&mut self, solid: Aabb) {
        let index = self.solids.len() as u32;
        for cell in cells_overlapping(solid) {
            self.cells.entry(cell).or_default().push(index);
        }
        self.solids.push(solid);
    }

    pub fn solid_count(&self) -> usize {
        self.solids.len()
    }

    /// Все тела, пересекающие `region`.
    /// Порядок — по индексу вставки (детерминированно).
    pub fn query(&self, region: Aabb) -> Vec<&Aabb> {
        let mut indices: Vec<u32> = Vec::new();
        for cell in cells_overlapping(region) {
            if let Some(list) = self.cells.get(&cell) {
                indices.extend_from_slice(list);
            }
        }
        indices.sort_unstable();
        indices.dedup();
        indices
            .into_iter()
            .map(|i| &self.solids[i as usize])
            .filter(|s| s.overlaps(&region))
            .collect()
    }

    /// Есть ли хотя бы одно тело, пересекающее `region` (без аллокаций результата).
    pub fn overlaps_any(&self, region: Aabb) -> bool {
        for cell in cells_overlapping(region) {
            if let Some(list) = self.cells.get(&cell) {
                for &i in list {
                    if self.solids[i as usize].overlaps(&region) {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Интеграция движения тела с коллизиями и разделением осей:
    /// 1. перемещение по X → выталкивание из всех пересечений по X;
    /// 2. перемещение по Y → выталкивание по Y, фиксация пола/потолка.
    ///
    /// Возвращает новую позицию, новые скорости и контакты.
    pub fn move_body(&self, pos: Vec2, size: Vec2, vel: Vec2, dt: f32) -> (Vec2, Vec2, CollisionHit) {
        let mut pos = pos;
        let mut vel = vel;
        let mut hit = CollisionHit::default();

        // Подшаги против туннелирования на высоких скоростях (dash, падение).
        let delta = vel * dt;
        let farthest = delta.x.abs().max(delta.y.abs());
        let steps = ((farthest / MAX_STEP_DISTANCE).ceil().max(1.0) as u32).min(64);
        let step_dt = dt / steps as f32;

        for _ in 0..steps {
            // ---------- ось X ----------
            let step_x = vel.x * step_dt;
            if step_x != 0.0 {
                pos.x += step_x;
                let mut body = Aabb { pos, size };
                for &solid in &self.query(body.expanded(1.0)) {
                    if !body.overlaps(solid) {
                        continue;
                    }
                    if vel.x > 0.0 {
                        pos.x = solid.left() - size.x;
                        hit.wall_dir = 1.0;
                    } else if vel.x < 0.0 {
                        pos.x = solid.right();
                        hit.wall_dir = -1.0;
                    }
                    vel.x = 0.0;
                    body = Aabb { pos, size };
                }
            }

            // ---------- ось Y ----------
            let step_y = vel.y * step_dt;
            if step_y != 0.0 {
                pos.y += step_y;
                let mut body = Aabb { pos, size };
                for &solid in &self.query(body.expanded(1.0)) {
                    if !body.overlaps(solid) {
                        continue;
                    }
                    if vel.y > 0.0 {
                        pos.y = solid.top() - size.y;
                        vel.y = 0.0;
                        hit.on_ground = true;
                    } else if vel.y < 0.0 {
                        pos.y = solid.bottom();
                        vel.y = 0.0;
                        hit.hit_ceiling = true;
                    }
                    body = Aabb { pos, size };
                }
            }
        }

        // Страховка для тел с нулевой вертикальной скоростью (дэш на земле,
        // покоящиеся объекты): опора под ногами есть — фиксируем контакт.
        if !hit.on_ground && vel.y >= 0.0 {
            let feet = Aabb {
                pos: pos + Vec2::new(2.0, size.y),
                size: Vec2::new((size.x - 4.0).max(1.0), 1.0),
            };
            if self.overlaps_any(feet) {
                hit.on_ground = true;
            }
        }

        (pos, vel, hit)
    }

    /// Рейкаст против твёрдых тел (slab-метод).
    /// `delta` — полный вектор отрезка: луч идёт от `origin` до `origin + delta`.
    pub fn raycast(&self, origin: Vec2, delta: Vec2) -> Option<RayHit> {
        if delta.x.abs() < f32::EPSILON && delta.y.abs() < f32::EPSILON {
            return None;
        }

        let mut region = Aabb {
            pos: Vec2::new(
                origin.x.min(origin.x + delta.x),
                origin.y.min(origin.y + delta.y),
            ),
            size: Vec2::new(delta.x.abs(), delta.y.abs()),
        };
        region = region.expanded(0.5);

        let mut best: Option<(f32, Vec2)> = None;
        for solid in self.query(region) {
            if let Some((t, normal)) = ray_aabb(origin, delta, solid) {
                match best {
                    None => best = Some((t, normal)),
                    // Оставляем ближайшее попадание (наименьший параметр t).
                    Some((best_t, _)) if t < best_t => best = Some((t, normal)),
                    _ => {}
                }
            }
        }

        best.map(|(t, normal)| {
            let point = origin + delta * t;
            RayHit {
                point,
                normal,
                t,
                dist: delta.length() * t,
            }
        })
    }
}

/// Пересечение отрезка `origin → origin + delta` с прямоугольником.
/// Возвращает параметр входа `t ∈ [0, 1]` и нормаль ударившейся грани.
fn ray_aabb(origin: Vec2, delta: Vec2, b: &Aabb) -> Option<(f32, Vec2)> {
    let mut tmin = 0.0f32;
    let mut tmax = 1.0f32;
    let mut normal = Vec2::ZERO;

    // --- ось X ---
    if delta.x.abs() < f32::EPSILON {
        if origin.x < b.left() || origin.x > b.right() {
            return None;
        }
    } else {
        let inv = 1.0 / delta.x;
        let t1 = (b.left() - origin.x) * inv;
        let t2 = (b.right() - origin.x) * inv;
        let (entry, exit, entry_normal) = if t1 <= t2 {
            (t1, t2, Vec2::new(-delta.x.signum(), 0.0))
        } else {
            (t2, t1, Vec2::new(delta.x.signum(), 0.0))
        };
        if entry > tmin {
            tmin = entry;
            normal = entry_normal;
        }
        if exit < tmax {
            tmax = exit;
        }
        if tmin > tmax {
            return None;
        }
    }

    // --- ось Y ---
    if delta.y.abs() < f32::EPSILON {
        if origin.y < b.top() || origin.y > b.bottom() {
            return None;
        }
    } else {
        let inv = 1.0 / delta.y;
        let t1 = (b.top() - origin.y) * inv;
        let t2 = (b.bottom() - origin.y) * inv;
        let (entry, exit, entry_normal) = if t1 <= t2 {
            (t1, t2, Vec2::new(0.0, -delta.y.signum()))
        } else {
            (t2, t1, Vec2::new(0.0, delta.y.signum()))
        };
        if entry > tmin {
            tmin = entry;
            normal = entry_normal;
        }
        if exit < tmax {
            tmax = exit;
        }
        if tmin > tmax {
            return None;
        }
    }

    if tmin > 1.0 || tmin < 0.0 {
        return None;
    }
    // Начало внутри тела — нормали нет, для игровых проб такой случай
    // не нужен (пробы стартуют вне твёрдых тел).
    if tmin == 0.0 && normal == Vec2::ZERO {
        return None;
    }
    Some((tmin, normal))
}

/// Ячейки сетки, пересекающие прямоугольник.
fn cells_overlapping(region: Aabb) -> impl Iterator<Item = (i32, i32)> {
    let x0 = (region.left() / CELL_SIZE).floor() as i32;
    let x1 = (region.right() / CELL_SIZE).floor() as i32;
    let y0 = (region.top() / CELL_SIZE).floor() as i32;
    let y1 = (region.bottom() / CELL_SIZE).floor() as i32;
    (x0..=x1).flat_map(move |x| (y0..=y1).map(move |y| (x, y)))
}

/// Добавить к скорости гравитацию с ограничением максимального падения.
#[inline]
pub fn apply_gravity(vel: &mut Vec2, dt: f32, gravity: f32, max_fall: f32) {
    vel.y = (vel.y + gravity * dt).min(max_fall);
}

/// Движение к целевому значению со скоростью не более `max_delta` за шаг.
/// Даёт ощущение инерции: плавный разгон и плавное торможение.
#[inline]
pub fn move_toward(current: f32, target: f32, max_delta: f32) -> f32 {
    let diff = target - current;
    if diff.abs() <= max_delta {
        target
    } else {
        current + diff.signum() * max_delta
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ground_world() -> (World, Aabb) {
        let mut world = World::new();
        let ground = Aabb::new(0.0, 100.0, 320.0, 32.0);
        world.add_solid(ground);
        (world, ground)
    }

    #[test]
    fn aabb_overlap_strict() {
        let a = Aabb::new(0.0, 0.0, 10.0, 10.0);
        let touching = Aabb::new(10.0, 0.0, 10.0, 10.0);
        let overlapping = Aabb::new(5.0, 5.0, 10.0, 10.0);
        assert!(!a.overlaps(&touching));
        assert!(a.overlaps(&overlapping));
    }

    #[test]
    fn falls_onto_ground_without_tunneling() {
        let (world, ground) = ground_world();
        let size = Vec2::new(24.0, 40.0);
        let mut pos = Vec2::new(50.0, 0.0);
        let mut vel = Vec2::new(0.0, 0.0);
        // 2 секунды падения с гравитацией — тело обязано встать на землю.
        for _ in 0..120 {
            apply_gravity(&mut vel, FIXED_DT, GRAVITY, MAX_FALL_SPEED);
            let (p, v, hit) = world.move_body(pos, size, vel, FIXED_DT);
            pos = p;
            vel = v;
            assert!(pos.y + size.y <= ground.top() + 0.5, "тело провалилось под землю");
            if hit.on_ground {
                assert!(vel.y == 0.0);
            }
        }
        assert!((pos.y + size.y - ground.top()).abs() < 0.01, "не встал на землю");
    }

    #[test]
    fn axis_separation_does_not_stick_in_corner() {
        let mut world = World::new();
        // Пол и вертикальная стена — классический угол.
        world.add_solid(Aabb::new(0.0, 100.0, 320.0, 32.0));
        world.add_solid(Aabb::new(64.0, 36.0, 32.0, 64.0));
        let size = Vec2::new(24.0, 40.0);
        // Тело скользит вправо и падает: должно скользить вдоль стены вниз,
        // а не застрять в углу.
        let mut pos = Vec2::new(30.0, 40.0);
        let mut vel = Vec2::new(200.0, 0.0);
        for _ in 0..120 {
            apply_gravity(&mut vel, FIXED_DT, GRAVITY, MAX_FALL_SPEED);
            vel.x = 200.0;
            let (p, v, _) = world.move_body(pos, size, vel, FIXED_DT);
            pos = p;
            vel = v;
        }
        assert!(
            (pos.y + size.y - 100.0).abs() < 0.5,
            "угол не должен блокировать падение, pos={:?}",
            pos
        );
        assert!(pos.x + size.x <= 64.0 + 0.5, "тело должно скользить вдоль стены");
    }

    #[test]
    fn raycast_hits_ground_and_reports_normal() {
        let (world, _) = ground_world();
        // По горизонтали на высоте 50 ничего нет — промах.
        assert!(world.raycast(Vec2::new(10.0, 50.0), Vec2::new(300.0, 0.0)).is_none());
        // Луч вниз — удар в землю с нормалью вверх.
        let down = world
            .raycast(Vec2::new(10.0, 50.0), Vec2::new(0.0, 200.0))
            .expect("луч вниз должен достичь земли");
        assert!(
            (down.normal.y + 1.0).abs() < f32::EPSILON,
            "нормаль должна смотреть вверх"
        );
        assert!((down.point.y - 100.0).abs() < 0.5);
    }

    #[test]
    fn move_toward_reaches_target() {
        assert_eq!(move_toward(0.0, 10.0, 3.0), 3.0);
        assert_eq!(move_toward(9.0, 10.0, 3.0), 10.0);
        assert_eq!(move_toward(10.0, 0.0, 4.0), 6.0);
    }
}
