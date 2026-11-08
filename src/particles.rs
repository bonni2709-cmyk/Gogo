//! Частицы с объектным пулом: искры удара и «пиксельный взрыв» смерти.
//!
//! Пул фиксированного размера ([`MAX_PARTICLES`]) — после [`ParticlePool::new`]
//! в кадре не происходит аллокаций: свободные слоты переиспользуются
//! через стек (`free`). ГПСЧ — xorshift32 без внешних зависимостей.

use macroquad::color::Color;
use macroquad::math::Vec2;
use macroquad::shapes::draw_rectangle;

/// Ёмкость пула (пиксели искр и взрывов укладываются с запасом).
pub const MAX_PARTICLES: usize = 600;

#[derive(Clone, Copy)]
struct Particle {
    pos: Vec2,
    vel: Vec2,
    /// Остаток жизни, с.
    life: f32,
    max_life: f32,
    size: f32,
    color: Color,
    gravity: f32,
}

/// Объектный пул частиц.
pub struct ParticlePool {
    slots: Vec<Option<Particle>>,
    free: Vec<u16>,
    rng: u32,
}

impl ParticlePool {
    pub fn new() -> Self {
        Self {
            slots: (0..MAX_PARTICLES).map(|_| None).collect(),
            free: (0..MAX_PARTICLES as u16).rev().collect(),
            rng: 0x9E37_79B9,
        }
    }

    /// Детерминированный xorshift32 → `0.0 .. 1.0`.
    fn rand(&mut self) -> f32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        (x & 0x00FF_FFFF) as f32 / 0x0100_0000 as f32
    }

    fn spawn(&mut self, particle: Particle) -> bool {
        let Some(idx) = self.free.pop() else {
            return false; // пул исчерпан — молча пропускаем
        };
        self.slots[idx as usize] = Some(particle);
        true
    }

    /// Сколько частиц сейчас живо (для тестов и отладки).
    pub fn alive_count(&self) -> usize {
        self.slots.iter().filter(|s| s.is_some()).count()
    }

    /// Искры от удара клинком: веер в сторону удара, быстрое затухание.
    pub fn hit_spark(&mut self, center: Vec2, dir: f32) {
        for _ in 0..10 {
            let a = self.rand();
            let b = self.rand();
            let c = self.rand();
            let d = self.rand();
            // Угол веера: перед персонажем, ±70°.
            let spread = -1.2 + a * 2.4;
            let angle = if dir >= 0.0 {
                spread
            } else {
                std::f32::consts::PI - spread
            };
            let speed = 90.0 + b * 190.0;
            let color = if c > 0.45 {
                Color::new(0.95, 0.93, 0.86, 1.0) // белая искра стали
            } else {
                Color::new(0.94, 0.75, 0.35, 1.0) // тёплая искра
            };
            let life = 0.16 + d * 0.16;
            self.spawn(Particle {
                pos: center,
                vel: Vec2::new(angle.cos(), angle.sin()) * speed,
                life,
                max_life: life,
                size: 2.0 + b * 1.5,
                color,
                gravity: 420.0,
            });
        }
    }

    /// Пиксельный взрыв смерти: цвета — палитра вида врага.
    pub fn death_burst(&mut self, center: Vec2, colors: &[Color]) {
        if colors.is_empty() {
            return;
        }
        for _ in 0..22 {
            let a = self.rand();
            let b = self.rand();
            let c = self.rand();
            let d = self.rand();
            let e = self.rand();
            let angle = a * std::f32::consts::TAU;
            let speed = 60.0 + b * 210.0;
            let color = colors[((c * colors.len() as f32) as usize).min(colors.len() - 1)];
            let offset = Vec2::new((d - 0.5) * 12.0, (e - 0.5) * 12.0);
            let life = 0.35 + c * 0.35;
            self.spawn(Particle {
                pos: center + offset,
                vel: Vec2::new(angle.cos(), angle.sin()) * speed,
                life,
                max_life: life,
                size: 2.0 + d * 2.0,
                color,
                gravity: 760.0,
            });
        }
    }

    /// Шаг симуляции (взывается только вне хитстопа).
    pub fn update(&mut self, dt: f32) {
        for (i, slot) in self.slots.iter_mut().enumerate() {
            let Some(p) = slot else {
                continue;
            };
            p.life -= dt;
            if p.life <= 0.0 {
                *slot = None;
                self.free.push(i as u16);
                continue;
            }
            p.vel.y += p.gravity * dt;
            // Лёгкое сопротивление воздуха.
            p.vel *= 1.0 - 1.6 * dt;
            p.pos += p.vel * dt;
        }
    }

    /// Отрисовка: затухание альфой и поджатие размера к концу жизни.
    pub fn draw(&self) {
        for slot in &self.slots {
            let Some(p) = slot else {
                continue;
            };
            let fade = (p.life / p.max_life).clamp(0.0, 1.0);
            let size = p.size * (0.55 + 0.45 * fade);
            let color = Color::new(
                p.color.r,
                p.color.g,
                p.color.b,
                p.color.a * (0.35 + 0.65 * fade),
            );
            draw_rectangle(
                p.pos.x - size * 0.5,
                p.pos.y - size * 0.5,
                size,
                size,
                color,
            );
        }
    }
}

impl Default for ParticlePool {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn burst_spawns_and_expires() {
        let mut pool = ParticlePool::new();
        assert_eq!(pool.alive_count(), 0);

        pool.death_burst(Vec2::ZERO, &[Color::new(1.0, 1.0, 1.0, 1.0)]);
        assert_eq!(pool.alive_count(), 22);

        // Живут не дольше ~0.7 с.
        for _ in 0..60 {
            pool.update(FIXED_STEP);
        }
        assert_eq!(pool.alive_count(), 0, "частицы должны истечь");
        // Слоты вернулись в пул — повторный взрыв возможен.
        pool.hit_spark(Vec2::ZERO, 1.0);
        assert_eq!(pool.alive_count(), 10);
    }

    #[test]
    fn pool_never_exceeds_capacity() {
        let mut pool = ParticlePool::new();
        for _ in 0..50 {
            pool.death_burst(Vec2::ZERO, &[Color::new(0.5, 0.5, 0.5, 1.0)]);
        }
        assert!(pool.alive_count() <= MAX_PARTICLES);
    }

    /// Локальная копия фиксированного шага (1/60), чтобы тест не зависел
    /// от других модулей.
    const FIXED_STEP: f32 = 1.0 / 60.0;
}
