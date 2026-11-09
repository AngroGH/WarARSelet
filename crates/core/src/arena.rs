//! Generational entity arena.
//!
//! Slots are recycled but their generation is bumped, so a stale `EntityId`
//! can never silently point at a new unit. This is the foundation of the
//! anti-cheat guarantee: the server validates every id it receives.

use serde::{Deserialize, Serialize};

use crate::ids::{EntityId, EntityIndex};

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Slot<T> {
    generation: u16,
    value: Option<T>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Arena<T> {
    slots: Vec<Slot<T>>,
    free: Vec<u32>,
    len: usize,
}

impl<T> Default for Arena<T> {
    fn default() -> Self {
        Arena {
            slots: Vec::new(),
            free: Vec::new(),
            len: 0,
        }
    }
}

impl<T> Arena<T> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_capacity(cap: usize) -> Self {
        Arena {
            slots: Vec::with_capacity(cap),
            free: Vec::new(),
            len: 0,
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    #[inline]
    pub fn capacity(&self) -> usize {
        self.slots.len()
    }

    pub fn insert(&mut self, value: T) -> EntityId {
        self.len += 1;
        if let Some(idx) = self.free.pop() {
            let slot = &mut self.slots[idx as usize];
            slot.value = Some(value);
            EntityId {
                index: EntityIndex(idx),
                generation: slot.generation,
            }
        } else {
            let idx = self.slots.len() as u32;
            self.slots.push(Slot {
                generation: 0,
                value: Some(value),
            });
            EntityId {
                index: EntityIndex(idx),
                generation: 0,
            }
        }
    }

    pub fn remove(&mut self, id: EntityId) -> Option<T> {
        let slot = self.slots.get_mut(id.index.0 as usize)?;
        if slot.generation != id.generation {
            return None;
        }
        let value = slot.value.take()?;
        slot.generation = slot.generation.wrapping_add(1);
        self.free.push(id.index.0);
        self.len -= 1;
        Some(value)
    }

    #[inline]
    pub fn get(&self, id: EntityId) -> Option<&T> {
        let slot = self.slots.get(id.index.0 as usize)?;
        if slot.generation != id.generation {
            return None;
        }
        slot.value.as_ref()
    }

    #[inline]
    pub fn get_mut(&mut self, id: EntityId) -> Option<&mut T> {
        let slot = self.slots.get_mut(id.index.0 as usize)?;
        if slot.generation != id.generation {
            return None;
        }
        slot.value.as_mut()
    }

    #[inline]
    pub fn contains(&self, id: EntityId) -> bool {
        self.get(id).is_some()
    }

    pub fn iter(&self) -> impl Iterator<Item = (EntityId, &T)> {
        self.slots.iter().enumerate().filter_map(|(i, slot)| {
            slot.value.as_ref().map(|v| {
                (
                    EntityId {
                        index: EntityIndex(i as u32),
                        generation: slot.generation,
                    },
                    v,
                )
            })
        })
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (EntityId, &mut T)> {
        self.slots.iter_mut().enumerate().filter_map(|(i, slot)| {
            let generation = slot.generation;
            slot.value.as_mut().map(|v| {
                (
                    EntityId {
                        index: EntityIndex(i as u32),
                        generation,
                    },
                    v,
                )
            })
        })
    }

    pub fn ids(&self) -> impl Iterator<Item = EntityId> + '_ {
        self.iter().map(|(id, _)| id)
    }

    pub fn values(&self) -> impl Iterator<Item = &T> {
        self.iter().map(|(_, v)| v)
    }

    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut T> {
        self.iter_mut().map(|(_, v)| v)
    }

    pub fn clear(&mut self) {
        self.slots.clear();
        self.free.clear();
        self.len = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_handle_is_rejected() {
        let mut arena: Arena<u32> = Arena::new();
        let a = arena.insert(10);
        let b = arena.insert(20);
        assert_eq!(arena.get(a), Some(&10));
        assert_eq!(arena.remove(a), Some(10));
        let c = arena.insert(30); // reuses slot of `a`
        assert_eq!(c.index, a.index);
        assert_ne!(c.generation, a.generation);
        assert_eq!(arena.get(a), None, "stale handle must not resolve");
        assert_eq!(arena.get(c), Some(&30));
        assert_eq!(arena.get(b), Some(&20));
        assert_eq!(arena.len(), 2);
    }

    #[test]
    fn iteration_matches_len() {
        let mut arena: Arena<u32> = Arena::new();
        for i in 0..100 {
            arena.insert(i);
        }
        for i in 0..50 {
            let id = EntityId {
                index: EntityIndex(i),
                generation: 0,
            };
            arena.remove(id);
        }
        assert_eq!(arena.len(), 50);
        assert_eq!(arena.iter().count(), 50);
        assert_eq!(arena.values().sum::<u32>(), (50..100).sum::<u32>());
    }
}
