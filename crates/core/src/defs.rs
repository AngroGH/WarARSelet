//! Unit and building definitions (static game data).
//!
//! The MVP ships a small, hand-tuned data set that already demonstrates the
//! classic RTS damage triangle and the epoch power curve. Adding 200 unit
//! types later is a pure data exercise: push more entries into `UnitCatalog`.

use serde::{Deserialize, Serialize};

use crate::epoch::Epoch;
use crate::ids::{BuildingDefId, UnitDefId};
use crate::resource::{Cost, ResourceKind};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum ArmorType {
    /// Light clothes / unarmoured: melted by archers.
    Light,
    /// Shields & mail: spears and cavalry work well.
    Medium,
    /// Plate / fortification: needs siege or gunpowder.
    Heavy,
    /// Buildings and structures.
    Structure,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum DamageType {
    /// Melee cut/blunt: strong vs Light, weak vs Heavy.
    Melee,
    /// Arrows: strong vs Light and Medium, weak vs Structure.
    Piercing,
    /// Siege/powder: strong vs Structure and Heavy, weak vs Light.
    Siege,
}

impl DamageType {
    /// Percentage multiplier against an armour class (1000 == neutral).
    pub fn effectiveness(self, armor: ArmorType) -> i32 {
        use ArmorType::*;
        use DamageType::*;
        match (self, armor) {
            (Melee, Light) => 1250,
            (Melee, Medium) => 1000,
            (Melee, Heavy) => 600,
            (Melee, Structure) => 400,
            (Piercing, Light) => 1300,
            (Piercing, Medium) => 1050,
            (Piercing, Heavy) => 500,
            (Piercing, Structure) => 300,
            (Siege, Light) => 700,
            (Siege, Medium) => 900,
            (Siege, Heavy) => 1200,
            (Siege, Structure) => 1600,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum UnitRole {
    Worker,
    /// Anti-cavalry melee.
    Spear,
    /// Generic melee line infantry.
    Infantry,
    /// Anti-infantry ranged.
    Archer,
    /// Anti-archer fast melee.
    Cavalry,
    /// Ranged cavalry, mobile harassment.
    HorseArcher,
    /// Anti-building siege engine.
    Siege,
    /// Late-game ranged line.
    Gunpowder,
    /// Heavy late game shock unit.
    Tank,
}

impl UnitRole {
    pub fn is_worker(self) -> bool {
        self == UnitRole::Worker
    }

    pub fn is_ranged(self) -> bool {
        matches!(
            self,
            UnitRole::Archer | UnitRole::HorseArcher | UnitRole::Gunpowder | UnitRole::Tank
        )
    }

    pub fn is_military(self) -> bool {
        !self.is_worker()
    }
}

/// Fixed-capacity list of unit ids, `Copy` and const-constructible so the
/// catalog stays a compile-time table (and survives serialization).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct UnitList {
    len: u8,
    items: [u16; 8],
}

impl UnitList {
    pub const EMPTY: UnitList = UnitList {
        len: 0,
        items: [0; 8],
    };

    pub const fn from_slice(ids: &[u16]) -> UnitList {
        let mut items = [0u16; 8];
        let mut i = 0;
        while i < ids.len() && i < 8 {
            items[i] = ids[i];
            i += 1;
        }
        UnitList {
            len: i as u8,
            items,
        }
    }

    pub fn len(&self) -> usize {
        self.len as usize
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn contains(&self, id: UnitDefId) -> bool {
        self.items[..self.len()].contains(&id.0)
    }

    pub fn iter(&self) -> impl Iterator<Item = UnitDefId> + '_ {
        self.items[..self.len()].iter().copied().map(UnitDefId)
    }
}

#[derive(Clone, Debug)]
pub struct UnitDef {
    pub id: UnitDefId,
    pub key: &'static str,
    pub name: &'static str,
    pub epoch: Epoch,
    pub role: UnitRole,
    pub max_hp: i32,
    pub armor: ArmorType,
    pub attack: i32,
    pub damage_type: DamageType,
    /// Ticks between two attacks (20 ticks == 1 second).
    pub attack_period: u16,
    pub attack_range: i32,
    /// Movement speed in tiles per second (milli).
    pub speed_milli: i32,
    /// How far the unit can see, in tiles (milli).
    pub sight_milli: i32,
    pub cost: Cost,
    pub pop: i32,
    /// Ticks needed to train.
    pub build_ticks: u16,
    /// Ticks of construction work provided per tick.
    pub build_work: u16,
    /// Resources gathered per gathering cycle.
    pub gather_rate: i32,
    /// Max cargo a worker can carry.
    pub capacity: i32,
}

impl UnitDef {
    /// Cost adjusted for the epoch power curve.
    pub fn effective_cost(&self) -> Cost {
        let g = self.epoch.growth_milli();
        let mut c = self.cost;
        for k in ResourceKind::ALL {
            let v = c.get(k);
            if v > 0 {
                c.set(k, v * g as i64 / 1000);
            }
        }
        c
    }

    pub fn damage_per_second_milli(&self) -> i32 {
        self.attack * 1000 / self.attack_period.max(1) as i32
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum BuildingRole {
    /// Town centre: trains workers, loses -> player is in trouble.
    TownCenter,
    House,
    /// Drop-off point for gathered resources.
    Storage,
    Barracks,
    Stable,
    Range,
    SiegeWorkshop,
    /// Wonder: alternative victory condition.
    Wonder,
    Wall,
    Tower,
    /// Economic building: slowly generates resources.
    Farm,
    LumberCamp,
    Quarry,
}

impl BuildingRole {
    pub fn is_production(self) -> bool {
        matches!(
            self,
            BuildingRole::TownCenter
                | BuildingRole::Barracks
                | BuildingRole::Stable
                | BuildingRole::Range
                | BuildingRole::SiegeWorkshop
        )
    }

    pub fn is_defensive(self) -> bool {
        matches!(self, BuildingRole::Wall | BuildingRole::Tower)
    }
}

#[derive(Clone, Debug)]
pub struct BuildingDef {
    pub id: BuildingDefId,
    pub key: &'static str,
    pub name: &'static str,
    pub epoch: Epoch,
    pub role: BuildingRole,
    pub max_hp: i32,
    pub armor: ArmorType,
    /// Attack of the building itself (towers/walls), 0 for economy.
    pub attack: i32,
    pub damage_type: DamageType,
    pub attack_period: u16,
    pub attack_range: i32,
    pub sight: i32,
    pub footprint: i32,
    pub cost: Cost,
    pub build_ticks: u16,
    /// Population provided by the building.
    pub pop_provided: i32,
    /// Passive income per second (economic buildings).
    pub income: Cost,
    /// Which units this building can train.
    pub trains: UnitList,
    /// Epoch required to place it.
    pub required_epoch: Epoch,
    /// Wonder hold time in ticks (only for `Wonder`).
    pub wonder_hold_ticks: u32,
}

/// Static catalog of all MVP units and buildings.
///
/// Not serialized with the world: it is compiled-in data, and `Default`
/// returns the canonical MVP catalog, so a deserialized world always gets it
/// back.
#[derive(Clone, Debug)]
pub struct Catalog {
    pub units: Vec<UnitDef>,
    pub buildings: Vec<BuildingDef>,
}

impl Default for Catalog {
    fn default() -> Self {
        default_catalog()
    }
}

impl Catalog {
    pub fn unit(&self, id: UnitDefId) -> &UnitDef {
        &self.units[id.0 as usize]
    }

    /// Panicking lookups are only used after validation; use `try_unit` for
    /// untrusted ids coming from the network.
    pub fn try_unit(&self, id: UnitDefId) -> Option<&UnitDef> {
        self.units.get(id.0 as usize)
    }

    pub fn building(&self, id: BuildingDefId) -> &BuildingDef {
        &self.buildings[id.0 as usize]
    }

    pub fn try_building(&self, id: BuildingDefId) -> Option<&BuildingDef> {
        self.buildings.get(id.0 as usize)
    }

    pub fn find_unit(&self, key: &str) -> Option<&UnitDef> {
        self.units.iter().find(|u| u.key == key)
    }

    pub fn find_building(&self, key: &str) -> Option<&BuildingDef> {
        self.buildings.iter().find(|b| b.key == key)
    }

    /// Units trainable in the given epoch and unlocked by the epoch.
    pub fn units_for_epoch(&self, epoch: Epoch) -> impl Iterator<Item = &UnitDef> {
        self.units.iter().filter(move |u| u.epoch == epoch)
    }

    pub fn buildings_for_epoch(&self, epoch: Epoch) -> impl Iterator<Item = &BuildingDef> {
        self.buildings.iter().filter(move |b| b.epoch == epoch)
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        for (i, u) in self.units.iter().enumerate() {
            if u.id.0 as usize != i {
                return Err("unit catalog ids must match their index");
            }
        }
        for (i, b) in self.buildings.iter().enumerate() {
            if b.id.0 as usize != i {
                return Err("building catalog ids must match their index");
            }
            for t in b.trains.iter() {
                if (t.0 as usize) >= self.units.len() {
                    return Err("building trains a non-existent unit");
                }
            }
        }
        Ok(())
    }
}

// Helpers to keep the catalog table compact.
#[allow(clippy::too_many_arguments)]
const fn u(
    id: u16,
    key: &'static str,
    name: &'static str,
    epoch: Epoch,
    role: UnitRole,
    max_hp: i32,
    armor: ArmorType,
    attack: i32,
    damage_type: DamageType,
    attack_period: u16,
    attack_range: i32,
    speed_milli: i32,
    sight_milli: i32,
    cost: Cost,
    pop: i32,
    build_ticks: u16,
    gather_rate: i32,
    capacity: i32,
) -> UnitDef {
    UnitDef {
        id: UnitDefId(id),
        key,
        name,
        epoch,
        role,
        max_hp,
        armor,
        attack,
        damage_type,
        attack_period,
        attack_range,
        speed_milli,
        sight_milli,
        cost,
        pop,
        build_ticks,
        build_work: build_ticks,
        gather_rate,
        capacity,
    }
}

#[allow(clippy::too_many_arguments)]
const fn b(
    id: u16,
    key: &'static str,
    name: &'static str,
    epoch: Epoch,
    role: BuildingRole,
    max_hp: i32,
    cost: Cost,
    build_ticks: u16,
    sight: i32,
    footprint: i32,
    pop_provided: i32,
    attack: i32,
    attack_range: i32,
    trains: UnitList,
) -> BuildingDef {
    BuildingDef {
        id: BuildingDefId(id),
        key,
        name,
        epoch,
        role,
        max_hp,
        armor: ArmorType::Structure,
        attack,
        damage_type: if attack > 0 {
            DamageType::Piercing
        } else {
            DamageType::Siege
        },
        attack_period: 30,
        attack_range,
        sight,
        footprint,
        cost,
        build_ticks,
        pop_provided,
        income: Cost::ZERO,
        trains,
        required_epoch: epoch,
        wonder_hold_ticks: 0,
    }
}

// --- unit ids -------------------------------------------------------------
pub const UNIT_WORKER: u16 = 0;
pub const UNIT_CLUBMAN: u16 = 1;
pub const UNIT_SLINGER: u16 = 2;
pub const UNIT_SPEARMAN: u16 = 3;
pub const UNIT_BRONZE_SWORDSMAN: u16 = 4;
pub const UNIT_ARCHER: u16 = 5;
pub const UNIT_IRON_LEGIONARY: u16 = 6;
pub const UNIT_IRON_CAVALRY: u16 = 7;
pub const UNIT_KNIGHT: u16 = 8;
pub const UNIT_CROSSBOWMAN: u16 = 9;
pub const UNIT_HORSE_ARCHER: u16 = 10;
pub const UNIT_TREBUCHET: u16 = 11;
pub const UNIT_MUSKETEER: u16 = 12;
pub const UNIT_CAVALRY: u16 = 13;
pub const UNIT_LIGHT_TANK: u16 = 14;
pub const UNIT_HEAVY_TANK: u16 = 15;
pub const UNIT_ARTILLERY: u16 = 16;

// --- building ids ---------------------------------------------------------
pub const BUILDING_TOWN_CENTER: u16 = 0;
pub const BUILDING_HOUSE: u16 = 1;
pub const BUILDING_STORAGE: u16 = 2;
pub const BUILDING_BARRACKS: u16 = 3;
pub const BUILDING_STABLE: u16 = 4;
pub const BUILDING_RANGE: u16 = 5;
pub const BUILDING_TOWER: u16 = 6;
pub const BUILDING_WALL: u16 = 7;
pub const BUILDING_SIEGE_WORKSHOP: u16 = 8;
pub const BUILDING_WONDER: u16 = 9;
pub const BUILDING_FARM: u16 = 10;

const TC_TRAINS: UnitList = UnitList::from_slice(&[UNIT_WORKER]);
const BARRACKS_TRAINS: UnitList = UnitList::from_slice(&[
    UNIT_CLUBMAN,
    UNIT_SPEARMAN,
    UNIT_BRONZE_SWORDSMAN,
    UNIT_IRON_LEGIONARY,
]);
const RANGE_TRAINS: UnitList = UnitList::from_slice(&[
    UNIT_SLINGER,
    UNIT_ARCHER,
    UNIT_CROSSBOWMAN,
    UNIT_MUSKETEER,
]);
const STABLE_TRAINS: UnitList = UnitList::from_slice(&[
    UNIT_IRON_CAVALRY,
    UNIT_KNIGHT,
    UNIT_HORSE_ARCHER,
]);
const SIEGE_TRAINS: UnitList = UnitList::from_slice(&[
    UNIT_TREBUCHET,
    UNIT_ARTILLERY,
    UNIT_LIGHT_TANK,
    UNIT_HEAVY_TANK,
]);

/// Build the MVP catalog: 17 units and 11 buildings across 7 epochs.
pub fn default_catalog() -> Catalog {
    let units = vec![
        u(UNIT_WORKER, "worker", "Worker", Epoch::Stone, UnitRole::Worker, 40, ArmorType::Light, 3, DamageType::Melee, 24, 1, 1200, 4000, Cost::new(50, 0, 0), 1, 60, 15, 10),
        u(UNIT_CLUBMAN, "clubman", "Clubman", Epoch::Stone, UnitRole::Infantry, 60, ArmorType::Light, 8, DamageType::Melee, 24, 1, 1100, 3500, Cost::new(40, 20, 0), 1, 70, 0, 0),
        u(UNIT_SLINGER, "slinger", "Slinger", Epoch::Stone, UnitRole::Archer, 40, ArmorType::Light, 7, DamageType::Piercing, 32, 5, 1050, 4200, Cost::new(30, 40, 0), 1, 80, 0, 0),
        u(UNIT_SPEARMAN, "spearman", "Spearman", Epoch::Bronze, UnitRole::Spear, 80, ArmorType::Medium, 12, DamageType::Melee, 24, 1, 1000, 3600, Cost::new(60, 40, 0).with_copper(10), 1, 90, 0, 0),
        u(UNIT_BRONZE_SWORDSMAN, "swordsman", "Swordsman", Epoch::Bronze, UnitRole::Infantry, 110, ArmorType::Medium, 17, DamageType::Melee, 24, 1, 950, 3600, Cost::new(70, 30, 0).with_copper(25), 1, 110, 0, 0),
        u(UNIT_ARCHER, "archer", "Archer", Epoch::Bronze, UnitRole::Archer, 55, ArmorType::Light, 14, DamageType::Piercing, 30, 6, 1000, 4600, Cost::new(40, 60, 0).with_copper(5), 1, 100, 0, 0),
        u(UNIT_IRON_LEGIONARY, "legionary", "Legionary", Epoch::Iron, UnitRole::Infantry, 160, ArmorType::Heavy, 26, DamageType::Melee, 24, 1, 950, 3600, Cost::new(80, 30, 0).with_iron(30), 1, 140, 0, 0),
        u(UNIT_IRON_CAVALRY, "horseman", "Horseman", Epoch::Iron, UnitRole::Cavalry, 130, ArmorType::Medium, 24, DamageType::Melee, 22, 1, 1750, 4000, Cost::new(90, 40, 0).with_iron(20), 2, 150, 0, 0),
        u(UNIT_KNIGHT, "knight", "Knight", Epoch::EarlyMedieval, UnitRole::Cavalry, 220, ArmorType::Heavy, 38, DamageType::Melee, 22, 1, 1700, 4000, Cost::new(110, 40, 0).with_iron(50), 2, 200, 0, 0),
        u(UNIT_CROSSBOWMAN, "crossbowman", "Crossbowman", Epoch::EarlyMedieval, UnitRole::Archer, 80, ArmorType::Medium, 30, DamageType::Piercing, 36, 7, 950, 4800, Cost::new(50, 70, 0).with_iron(15), 1, 180, 0, 0),
        u(UNIT_HORSE_ARCHER, "horse_archer", "Horse Archer", Epoch::EarlyMedieval, UnitRole::HorseArcher, 140, ArmorType::Light, 24, DamageType::Piercing, 30, 6, 1900, 4600, Cost::new(80, 60, 0).with_iron(20), 2, 190, 0, 0),
        u(UNIT_TREBUCHET, "trebuchet", "Trebuchet", Epoch::LateMedieval, UnitRole::Siege, 180, ArmorType::Heavy, 90, DamageType::Siege, 90, 10, 550, 5000, Cost::new(80, 160, 60).with_iron(40), 3, 320, 0, 0),
        u(UNIT_MUSKETEER, "musketeer", "Musketeer", Epoch::Industry1, UnitRole::Gunpowder, 150, ArmorType::Medium, 55, DamageType::Siege, 40, 8, 900, 5000, Cost::new(90, 60, 0).with_iron(50), 2, 260, 0, 0),
        u(UNIT_CAVALRY, "cavalry", "Cavalry", Epoch::Industry1, UnitRole::Cavalry, 300, ArmorType::Heavy, 70, DamageType::Melee, 24, 1, 1800, 4200, Cost::new(120, 40, 0).with_iron(60), 3, 280, 0, 0),
        u(UNIT_LIGHT_TANK, "light_tank", "Light Tank", Epoch::Industry2, UnitRole::Tank, 520, ArmorType::Heavy, 90, DamageType::Siege, 34, 8, 1400, 5200, Cost::new(140, 60, 0).with_iron(120), 4, 400, 0, 0),
        u(UNIT_HEAVY_TANK, "heavy_tank", "Heavy Tank", Epoch::Industry2, UnitRole::Tank, 900, ArmorType::Heavy, 150, DamageType::Siege, 40, 8, 1150, 5000, Cost::new(200, 80, 0).with_iron(180), 6, 520, 0, 0),
        u(UNIT_ARTILLERY, "artillery", "Artillery", Epoch::Industry2, UnitRole::Siege, 380, ArmorType::Medium, 220, DamageType::Siege, 100, 14, 500, 6000, Cost::new(160, 100, 0).with_iron(140), 5, 460, 0, 0),
    ];

    let buildings = vec![
        b(BUILDING_TOWN_CENTER, "town_center", "Town Center", Epoch::Stone, BuildingRole::TownCenter, 2500, Cost::new(400, 350, 0), 600, 8, 4, 10, 6, 8, TC_TRAINS),
        b(BUILDING_HOUSE, "house", "House", Epoch::Stone, BuildingRole::House, 350, Cost::new(0, 70, 0), 180, 3, 1, 5, 0, 0, UnitList::EMPTY),
        b(BUILDING_STORAGE, "storage", "Storage Pit", Epoch::Stone, BuildingRole::Storage, 500, Cost::new(0, 90, 0), 200, 4, 2, 0, 0, 0, UnitList::EMPTY),
        b(BUILDING_BARRACKS, "barracks", "Barracks", Epoch::Stone, BuildingRole::Barracks, 900, Cost::new(0, 150, 0), 300, 4, 3, 0, 0, 0, BARRACKS_TRAINS),
        b(BUILDING_STABLE, "stable", "Stable", Epoch::Iron, BuildingRole::Stable, 950, Cost::new(0, 150, 20), 330, 4, 3, 0, 0, 0, STABLE_TRAINS),
        b(BUILDING_RANGE, "range", "Archery Range", Epoch::Bronze, BuildingRole::Range, 800, Cost::new(0, 140, 0), 300, 4, 3, 0, 0, 0, RANGE_TRAINS),
        b(BUILDING_TOWER, "tower", "Watch Tower", Epoch::Bronze, BuildingRole::Tower, 700, Cost::new(0, 40, 100), 260, 7, 1, 0, 18, 9, UnitList::EMPTY),
        b(BUILDING_WALL, "wall", "Wall", Epoch::Iron, BuildingRole::Wall, 1200, Cost::new(0, 0, 60), 120, 2, 1, 0, 4, 1, UnitList::EMPTY),
        b(BUILDING_SIEGE_WORKSHOP, "siege_workshop", "Siege Workshop", Epoch::LateMedieval, BuildingRole::SiegeWorkshop, 1100, Cost::new(0, 200, 0).with_iron(50), 380, 4, 3, 0, 0, 0, SIEGE_TRAINS),
        b(BUILDING_WONDER, "wonder", "Wonder of the World", Epoch::LateMedieval, BuildingRole::Wonder, 4000, Cost::new(2000, 2500, 2000).with_gold(1500), 2400, 6, 5, 20, 0, 0, UnitList::EMPTY),
        b(BUILDING_FARM, "farm", "Farm", Epoch::Bronze, BuildingRole::Farm, 200, Cost::new(0, 60, 0), 150, 2, 2, 0, 0, 0, UnitList::EMPTY),
    ];

    let mut catalog = Catalog { units, buildings };
    // Wonder hold time: 10 minutes of game time.
    if let Some(w) = catalog
        .buildings
        .iter_mut()
        .find(|b| b.role == BuildingRole::Wonder)
    {
        w.wonder_hold_ticks = crate::time::TICKS_PER_MINUTE * 10;
    }
    catalog
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_is_valid() {
        let c = default_catalog();
        c.validate().unwrap();
        assert_eq!(c.units.len(), 17);
        assert_eq!(c.buildings.len(), 11);
    }

    #[test]
    fn damage_triangle() {
        // spears beat cavalry (medium armour), cavalry beats archers (light),
        // archers beat light infantry.
        assert!(DamageType::Melee.effectiveness(ArmorType::Light) > 1200);
        assert!(DamageType::Melee.effectiveness(ArmorType::Heavy) < 700);
        assert!(DamageType::Piercing.effectiveness(ArmorType::Light) > 1200);
        assert!(DamageType::Siege.effectiveness(ArmorType::Structure) > 1500);
    }

    #[test]
    fn epoch_power_curve() {
        let c = default_catalog();
        let club = c.unit(UnitDefId(UNIT_CLUBMAN));
        let tank = c.unit(UnitDefId(UNIT_HEAVY_TANK));
        assert!(tank.damage_per_second_milli() > club.damage_per_second_milli() * 5);
        assert!(tank.effective_cost().total() > club.effective_cost().total() * 5);
    }

    #[test]
    fn unit_lists_work() {
        assert!(TC_TRAINS.contains(UnitDefId(UNIT_WORKER)));
        assert!(!TC_TRAINS.contains(UnitDefId(UNIT_CLUBMAN)));
        assert_eq!(BARRACKS_TRAINS.len(), 4);
    }
}
