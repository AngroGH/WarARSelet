use warsel_core::defs::UNIT_WORKER;
use warsel_core::entity::Entity;
use warsel_core::{Command, GameMode, MatchConfig, PlayerId, TilePos, World};

#[test]
fn debug_gather() {
    let mut world = World::new(
        MatchConfig {
            seed: 2024,
            map_size: warsel_core::MapSize::Duel,
            mode: GameMode::Sandbox,
            ..MatchConfig::default()
        },
        vec![("Alice".into(), 0, false)],
    );
    let alice = PlayerId(1);
    let (node_i, node_pos, kind) = world
        .map
        .alive_nodes()
        .next()
        .map(|(i, n)| (i, n.pos, n.kind))
        .unwrap();
    println!("node {:?} at {:?}", kind, node_pos);
    let id = world
        .entities
        .iter()
        .find(|(_, e)| matches!(e, Entity::Unit(u) if u.owner == alice && u.def.0 == UNIT_WORKER))
        .map(|(id, _)| id)
        .unwrap();
    let r = world.apply_command(alice, Command::Gather { units: vec![id], node: node_i as u32 });
    println!("cmd {:?}", r);
    for t in 0..400 {
        world.step();
        if t % 40 == 0 {
            let u = world.unit(id).unwrap();
            println!(
                "t={} tile={:?} cargo={} state={:?} order={:?} food={}",
                t,
                TilePos::from_vec(u.pos),
                u.cargo.amount,
                u.state,
                u.order,
                world.player(alice).resources.food
            );
        }
    }
}
