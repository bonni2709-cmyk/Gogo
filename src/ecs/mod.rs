//! Лёгкий ECS: аллокатор сущностей + типизированные хранилища компонентов.
//!
//! Архитектура намеренно минимальна (без внешних зависимостей):
//!
//! * [`Entity`] — индекс слота + поколение. При уничтожении поколение
//!   слота растёт, поэтому «старые» ссылки перестают быть валидными,
//!   даже если слот уже переиспользован.
//! * [`ComponentStore`] — карта `Entity → компонент` на `BTreeMap`:
//!   детерминированный порядок обхода (важно для воспроизводимости тестов).
//! * [`Transform`], [`Velocity`], [`Health`] — общие компоненты сущностей.
//!
//! Игровые системы (враги) собирают сущности из хранилищ сами и владеют
//! своими компонентами мозга.

use macroquad::math::Vec2;
use std::collections::BTreeMap;

use crate::physics::Aabb;

/// Идентификатор сущности: слот + поколение.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Entity {
    pub index: u32,
    pub generation: u32,
}

/// Аллокатор сущностей с переиспользованием слотов и защитой поколения.
#[derive(Default)]
pub struct EntityAllocator {
    generations: Vec<u32>,
    alive: Vec<bool>,
    free: Vec<u32>,
}

impl EntityAllocator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Выделить новую сущность (при возможности — переиспользуя слот).
    pub fn spawn(&mut self) -> Entity {
        if let Some(index) = self.free.pop() {
            let generation = self.generations[index as usize];
            self.alive[index as usize] = true;
            Entity {
                index,
                generation,
            }
        } else {
            let index = self.generations.len() as u32;
            self.generations.push(0);
            self.alive.push(true);
            Entity {
                index,
                generation: 0,
            }
        }
    }

    /// Уничтожить сущность: слот уходит в свободный список, поколение
    /// растёт — все прежние `Entity` с этим индексом становятся мёртвыми.
    pub fn despawn(&mut self, entity: Entity) -> bool {
        if !self.is_alive(entity) {
            return false;
        }
        let i = entity.index as usize;
        self.alive[i] = false;
        self.generations[i] = self.generating(i);
        self.free.push(entity.index);
        true
    }

    fn generating(&self, i: usize) -> u32 {
        self.generations[i].saturating_add(1)
    }

    /// Жива ли сущность прямо сейчас.
    pub fn is_alive(&self, entity: Entity) -> bool {
        let i = entity.index as usize;
        i < self.alive.len() && self.alive[i] && self.generations[i] == entity.generation
    }

    /// Количество живых сущностей.
    pub fn alive_count(&self) -> usize {
        self.alive.iter().filter(|a| **a).count()
    }
}

/// Хранилище компонентов одного типа: `Entity → T`.
///
/// `BTreeMap` даёт стабильный (сортированный по `Entity`) порядок обхода.
#[derive(Default)]
pub struct ComponentStore<T> {
    data: BTreeMap<Entity, T>,
}

impl<T> ComponentStore<T> {
    pub fn new() -> Self {
        Self {
            data: BTreeMap::new(),
        }
    }

    /// Поставить компонент, возвращая прежнее значение (если было).
    pub fn insert(&mut self, entity: Entity, value: T) -> Option<T> {
        self.data.insert(entity, value)
    }

    pub fn remove(&mut self, entity: Entity) -> Option<T> {
        self.data.remove(&entity)
    }

    pub fn get(&self, entity: Entity) -> Option<&T> {
        self.data.get(&entity)
    }

    pub fn get_mut(&mut self, entity: Entity) -> Option<&mut T> {
        self.data.get_mut(&entity)
    }

    pub fn contains(&self, entity: Entity) -> bool {
        self.data.contains_key(&entity)
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// Обход всех сущностей хранилища в детерминированном порядке.
    pub fn entities(&self) -> impl Iterator<Item = Entity> + '_ {
        self.data.keys().copied()
    }

    pub fn iter(&self) -> impl Iterator<Item = (Entity, &T)> + '_ {
        self.data.iter().map(|(e, v)| (*e, v))
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (Entity, &mut T)> + '_ {
        self.data.iter_mut().map(|(e, v)| (*e, v))
    }
}

// ---------------------------------------------------------------------------
// Общие компоненты
// ---------------------------------------------------------------------------

/// Позиция и размер тела: левый верхний угол — как у игрока.
#[derive(Clone, Copy, Debug)]
pub struct Transform {
    pub pos: Vec2,
    pub size: Vec2,
    /// Куда смотрит сущность: `-1.0` или `1.0`.
    pub facing: f32,
}

impl Transform {
    pub fn body(&self) -> Aabb {
        Aabb {
            pos: self.pos,
            size: self.size,
        }
    }

    pub fn center(&self) -> Vec2 {
        self.pos + self.size * 0.5
    }
}

/// Скорость тела (пиксели в секунду).
#[derive(Clone, Copy, Debug)]
pub struct Velocity(pub Vec2);

/// Здоровье: очки жизни и таймер вспышки при получении урона.
#[derive(Clone, Copy, Debug)]
pub struct Health {
    pub hp: u32,
    /// Остаток времени вспышки, с (0 — не мигает).
    pub hit_flash: f32,
}

impl Health {
    pub fn new(hp: u32) -> Self {
        Self { hp, hit_flash: 0.0 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn despawn_invalidates_old_entity() {
        let mut alloc = EntityAllocator::new();
        let first = alloc.spawn();
        assert!(alloc.is_alive(first));

        assert!(alloc.despawn(first));
        assert!(!alloc.is_alive(first));
        assert!(!alloc.despawn(first), "повторный despawn — no-op");

        // Слот переиспользуется, но поколение другое — старая ссылка мертва.
        let second = alloc.spawn();
        assert_eq!(second.index, first.index);
        assert_ne!(second.generation, first.generation);
        assert!(alloc.is_alive(second));
        assert!(!alloc.is_alive(first));
        assert_eq!(alloc.alive_count(), 1);
    }

    #[test]
    fn store_insert_get_remove() {
        let mut alloc = EntityAllocator::new();
        let e = alloc.spawn();
        let mut store = ComponentStore::new();

        assert!(store.insert(e, 42u32).is_none());
        assert_eq!(store.get(e), Some(&42));
        *store.get_mut(e).unwrap() += 1;
        assert_eq!(store.get(e), Some(&43));
        assert_eq!(store.remove(e), Some(43));
        assert!(store.get(e).is_none());
        assert!(store.is_empty());
    }

    #[test]
    fn store_iterates_in_entity_order() {
        let mut alloc = EntityAllocator::new();
        let a = alloc.spawn();
        let b = alloc.spawn();
        let c = alloc.spawn();
        let mut store = ComponentStore::new();
        // Вставляем в «перепутанном» порядке — обход отсортирован.
        store.insert(c, "c");
        store.insert(a, "a");
        store.insert(b, "b");

        let keys: Vec<Entity> = store.entities().collect();
        assert_eq!(keys, vec![a, b, c]);
        let values: Vec<&str> = store.iter().map(|(_, v)| *v).collect();
        assert_eq!(values, vec!["a", "b", "c"]);
    }
}
