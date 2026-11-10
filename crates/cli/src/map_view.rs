//! ASCII map preview.

use warsel_core::map::{Map, Terrain};
use warsel_core::ResourceKind;

use crate::args::Options;

pub fn print(opts: &Options) {
    let size = opts.map_size();
    let map = Map::generate(opts.seed, size, opts.players);

    // Fit the preview into a normal terminal width.
    let target_columns = 76;
    let step = ((map.width + target_columns - 1) / target_columns).max(1);
    let columns = (map.width + step - 1) / step;
    let rows = (map.height + step - 1) / step;

    println!(
        "WarARSelet map - seed {}, {} players, {:?} ({}x{})",
        opts.seed, opts.players, size, map.width, map.height
    );
    println!("{}x{} tiles per character\n", step, step);

    for gy in 0..rows {
        let mut line = String::with_capacity(columns as usize);
        for gx in 0..columns {
            let x0 = gx * step;
            let y0 = gy * step;
            let mut ch = None;
            for (i, start) in map.starts.iter().enumerate() {
                if in_block(*start, x0, y0, step) {
                    ch = Some(if i < 9 { (b'1' + i as u8) as char } else { '*' });
                    break;
                }
            }
            if ch.is_none() {
                for node in &map.nodes {
                    if node.amount > 0 && in_block(node.pos, x0, y0, step) {
                        ch = Some(node_char(node.kind));
                        break;
                    }
                }
            }
            let ch = ch.unwrap_or_else(|| match map.terrain(x0, y0) {
                Terrain::Water => '~',
                Terrain::Rock => '^',
                Terrain::Grass => '.',
            });
            line.push(ch);
        }
        println!("{line}");
    }

    println!();
    println!("Legend: 1-9=start *=start>9 f=food T=wood s=stone c=copper i=iron g=gold ~=water ^=rock .=grass");
    print!("Nodes:");
    for kind in ResourceKind::ALL {
        let nodes = map
            .nodes
            .iter()
            .filter(|node| node.kind == kind && node.amount > 0)
            .count();
        print!(" {}={}", kind.name(), nodes);
    }
    println!();
    let starts = map
        .starts
        .iter()
        .map(|start| format!("({},{})", start.x, start.y))
        .collect::<Vec<_>>()
        .join(" ");
    println!("Starts: {starts}");
}

fn in_block(pos: warsel_core::TilePos, x0: i32, y0: i32, step: i32) -> bool {
    pos.x >= x0 && pos.x < x0 + step && pos.y >= y0 && pos.y < y0 + step
}

fn node_char(kind: ResourceKind) -> char {
    match kind {
        ResourceKind::Food => 'f',
        ResourceKind::Wood => 'T',
        ResourceKind::Stone => 's',
        ResourceKind::Copper => 'c',
        ResourceKind::Iron => 'i',
        ResourceKind::Gold => 'g',
    }
}
