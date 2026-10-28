//! `cargo run --release -p libsms-present --example treasures -- [cache dir]`: bakes every Pikmin 2 treasure and
//! upgrade from the cache (`libsms::extract::p2_treasure`) and prints one CSV line each: name, in-game name,
//! carriers min/max, Pokos, config radius/height, baked size, loop frames.
use libsms_assets::treasure::Table;
use libsms_present::treasure::Treasure;

fn main() {
    let root = std::env::args().nth(1).unwrap_or_else(|| "assets/p2".into());
    let dir = std::path::Path::new(&root).join("treasure");
    let table = Table::load(&dir).expect("table (run p2_treasure_extract first)");
    println!("kind,name,title,min,max,money,radius,height,size_x,size_y,size_z,loop_start,loop_end,indirect");
    for c in table.all() {
        let t = Treasure::load(&dir, c).expect("bake");
        let s = t.size();
        let (ls, le) = t.anim.as_ref().map_or((0.0, 0.0), |a| (a.loop_start, a.loop_end));
        println!(
            "{:?},{},\"{}\",{},{},{},{},{},{:.1},{:.1},{:.1},{},{},{:?}",
            c.kind, c.name, table.display_name(c).unwrap_or_default(), c.min, c.max, c.money, c.radius, c.height,
            s.x, s.y, s.z, ls, le, c.indirect
        );
    }
}
