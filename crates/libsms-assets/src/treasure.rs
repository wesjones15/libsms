//! Pikmin 2's treasure data, parsed from the text files `libsms::extract::p2_treasure` copies out of the user's
//! disc: the pellet config lists (`otakara_config.txt`, `item_config.txt`), a treasure's `animmgr.txt` and
//! `collinfo.txt`, and the message file (`pikmin2.bmg`) that holds the names and descriptions. No game data is
//! in this file; it only reads what the cache holds.
//!
//! Spec: `PelletConfig::TParms` and `PelletConfigList::read` (plugProjectKandoU/pelletConfig.cpp) for the
//! config fields; `Pellet::getPelletConfigMin/Max` and `Pellet::onInit` (pelletMgr.cpp) for what `min`/`max`
//! mean (carriers needed to lift it, carry slots); `onyonMgr.cpp` for `money` (the Pokos a delivery pays);
//! `PelletList::Mgr::getOffsetFromDictionaryNo` (gamePelletList.cpp) and `TItemZukan` (zukan2D.cpp) for
//! which message is whose.

use std::collections::HashMap;

/// Which list a config came from. The game counts both in one collection (`getDictionaryNum`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    /// `otakara_config.txt`: the 188 treasures.
    Treasure,
    /// `item_config.txt`: the 13 upgrades (Onion-pod parts: Power Punch... ).
    Upgrade,
}

/// How a treasure's surface is drawn (`PelletConfig::mIndirectState`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Indirect {
    No,
    /// `yes`: the model has an `indirect_tex` material that refracts the framebuffer.
    Yes,
    /// `use`: draws with the framebuffer texture the `yes` model made.
    Use,
}

/// One `{ ... end }` block of a config list (`PelletConfig::TParms`).
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub kind: Kind,
    /// Position in its list (`mParams.mIndex`).
    pub index: usize,
    pub name: String,
    /// The `.szs` beside the lists that holds the model and the carry motion.
    pub archive: String,
    pub bmd: String,
    pub anim_mgr: String,
    pub coll_tree: String,
    /// Footprint radius (`getBottomRadius`) and the carriers' ring radius (`getStickRadius`).
    pub radius: f32,
    pub p_radius: f32,
    pub height: f32,
    pub inertia_scaling: f32,
    pub particle_type: String,
    pub num_particles: i32,
    pub particle_size: f32,
    pub friction: f32,
    /// Pikmin needed to lift it (`mMin`) and the most that can carry it (`mMax`, the slot count).
    pub min: u32,
    pub max: u32,
    pub piki_count_min: u32,
    pub piki_count_max: u32,
    /// `lod` or `never`.
    pub dynamics: String,
    /// Pokos paid on delivery (`mMoney`).
    pub money: u32,
    pub unique: bool,
    pub indirect: Indirect,
    pub num_pmotions: u32,
    pub depth: f32,
    pub depth_max: f32,
    pub depth_abcd: [f32; 4],
    /// Item descriptions' message id (`0009_00`), upgrades only.
    pub message: Option<String>,
    pub code: i32,
    /// 1-based collection number (`mDictionary`, 0 when absent).
    pub dictionary: u32,
}

impl Config {
    fn blank(kind: Kind, index: usize) -> Config {
        Config {
            kind,
            index,
            name: String::new(),
            archive: String::new(),
            bmd: String::new(),
            anim_mgr: String::new(),
            coll_tree: String::new(),
            radius: 0.0,
            p_radius: 0.0,
            height: 0.0,
            inertia_scaling: 0.0,
            particle_type: String::new(),
            num_particles: 0,
            particle_size: 1.0,
            friction: 0.0,
            min: 0,
            max: 0,
            piki_count_min: 0,
            piki_count_max: 0,
            dynamics: String::new(),
            money: 0,
            unique: false,
            indirect: Indirect::No,
            num_pmotions: 0,
            depth: 0.0,
            depth_max: 0.0,
            // PelletConfig::PelletConfig: 10, 20, 30, 40.
            depth_abcd: [10.0, 20.0, 30.0, 40.0],
            message: None,
            code: 0,
            dictionary: 0,
        }
    }

    /// Folder name of the model under the cache's `models/` (the archive without `.szs`).
    pub fn model_dir(&self) -> &str {
        self.archive.strip_suffix(".szs").unwrap_or(&self.archive)
    }
}

/// Parses a config list. `None` when the text has no leading count or a block is cut short.
pub fn parse_configs(text: &str, kind: Kind) -> Option<Vec<Config>> {
    let mut lines = text
        .lines()
        .map(|l| l.split('#').next().unwrap_or("").trim())
        .filter(|l| !l.is_empty());
    let count: usize = lines.next()?.split_whitespace().next()?.parse().ok()?;
    let mut out = Vec::with_capacity(count);
    let mut cur: Option<Config> = None;
    for line in lines {
        let mut w = line.split_whitespace();
        let key = w.next()?;
        match key {
            "{" => cur = Some(Config::blank(kind, out.len())),
            "}" => out.push(cur.take()?),
            "end" => {}
            _ => {
                let c = cur.as_mut()?;
                let v = w.next().unwrap_or("");
                let f = || v.parse::<f32>().unwrap_or(0.0);
                let n = || v.parse::<i64>().unwrap_or(0);
                match key {
                    "name" => c.name = v.to_owned(),
                    "archive" => c.archive = v.to_owned(),
                    "bmd" => c.bmd = v.to_owned(),
                    "animmgr" => c.anim_mgr = v.to_owned(),
                    "colltree" => c.coll_tree = v.to_owned(),
                    "radius" => c.radius = f(),
                    "p_radius" => c.p_radius = f(),
                    "height" => c.height = f(),
                    "inertiascaling" => c.inertia_scaling = f(),
                    "particletype" => c.particle_type = v.to_owned(),
                    "numparticles" => c.num_particles = n() as i32,
                    "particlesize" => c.particle_size = f(),
                    "friction" => c.friction = f(),
                    "min" => c.min = n().max(0) as u32,
                    "max" => c.max = n().max(0) as u32,
                    "pikicountmin" => c.piki_count_min = n().max(0) as u32,
                    "pikicountmax" => c.piki_count_max = n().max(0) as u32,
                    "dynamics" => c.dynamics = v.to_owned(),
                    "money" => c.money = n().max(0) as u32,
                    "unique" => c.unique = v == "yes",
                    "indirect" => {
                        c.indirect = match v {
                            "yes" => Indirect::Yes,
                            "use" => Indirect::Use,
                            _ => Indirect::No,
                        }
                    }
                    "num_pmotions" => c.num_pmotions = n().max(0) as u32,
                    "depth" => c.depth = f(),
                    "depth_max" => c.depth_max = f(),
                    "depth_a" => c.depth_abcd[0] = f(),
                    "depth_b" => c.depth_abcd[1] = f(),
                    "depth_c" => c.depth_abcd[2] = f(),
                    "depth_d" => c.depth_abcd[3] = f(),
                    "message" => c.message = Some(v.to_owned()),
                    "code" => c.code = n() as i32,
                    "dictionary" => c.dictionary = n().max(0) as u32,
                    _ => {}
                }
            }
        }
    }
    (cur.is_none() && out.len() == count).then_some(out)
}

/// A treasure's `animmgr.txt` (`PelletConfig::mAnimMgr`): one motion, `carry.bck`, looped between two frames.
#[derive(Debug, Clone, PartialEq)]
pub struct AnimMgr {
    pub file: String,
    pub loop_start: f32,
    pub loop_end: f32,
}

pub fn parse_anim_mgr(text: &str) -> Option<AnimMgr> {
    let toks: Vec<&str> = text
        .lines()
        .map(|l| l.split('#').next().unwrap_or(""))
        .flat_map(str::split_whitespace)
        .collect();
    let file = toks.iter().find(|t| t.ends_with(".bck") || t.ends_with(".bca"))?;
    let at = toks.iter().position(|t| t == file)?;
    let nums: Vec<f32> = toks[at + 1..].iter().map_while(|t| t.parse().ok()).collect();
    // (frame, key) pairs: key 0 starts the loop, 1 ends it, then -1.
    let mut start = 0.0;
    let mut end = 0.0;
    for p in nums.chunks(2) {
        match p {
            [frame, key] if *key == 0.0 => start = *frame,
            [frame, key] if *key == 1.0 => end = *frame,
            _ => {}
        }
    }
    Some(AnimMgr { file: (*file).to_owned(), loop_start: start, loop_end: end })
}

/// One sphere of a collision tree (`CollPart`, `CollTree::read`): an offset from its joint and a radius.
#[derive(Debug, Clone, PartialEq)]
pub struct CollSphere {
    pub radius: f32,
    pub offset: [f32; 3],
    pub joint: i32,
    pub children: Vec<CollSphere>,
}

impl CollSphere {
    /// Every sphere of the tree, depth first.
    pub fn flatten(&self) -> Vec<&CollSphere> {
        let mut v = vec![self];
        for c in &self.children {
            v.extend(c.flatten());
        }
        v
    }
}

/// Parses a `collinfo.txt`. Nodes are: child count, radius, `{id}`, `{code}`, offset x y z, joint, attribute,
/// then, when the count is not 0, a `{ ... }` holding the children.
pub fn parse_coll_tree(text: &str) -> Option<CollSphere> {
    let toks: Vec<&str> = text
        .lines()
        .map(|l| l.split('#').next().unwrap_or(""))
        .flat_map(str::split_whitespace)
        .collect();
    fn node(t: &[&str], i: &mut usize) -> Option<CollSphere> {
        let count: usize = t.get(*i)?.parse().ok()?;
        let radius: f32 = t.get(*i + 1)?.parse().ok()?;
        // t[i+2], t[i+3]: id and code in braces.
        let off = [t.get(*i + 4)?.parse().ok()?, t.get(*i + 5)?.parse().ok()?, t.get(*i + 6)?.parse().ok()?];
        let joint: i32 = t.get(*i + 7)?.parse().ok()?;
        // t[i+8]: attribute.
        *i += 9;
        let mut children = Vec::new();
        if count > 0 {
            if *t.get(*i)? != "{" {
                return None;
            }
            *i += 1;
            for _ in 0..count {
                children.push(node(t, i)?);
            }
            if *t.get(*i)? != "}" {
                return None;
            }
            *i += 1;
        }
        Some(CollSphere { radius, offset: off, joint, children })
    }
    let mut i = 0;
    node(&toks, &mut i)
}

/// `pikmin2.bmg` (JMessage `MESGbmg1`: `INF1` entries, `DAT1` text, `MID1` ids): message id to text.
#[derive(Debug, Default, Clone)]
pub struct Messages {
    by_id: HashMap<u32, String>,
}

impl Messages {
    pub fn parse(d: &[u8]) -> Option<Messages> {
        if d.get(0..8)? != b"MESGbmg1" {
            return None;
        }
        let be32 = |o: usize| d.get(o..o + 4).map(|b| u32::from_be_bytes(b.try_into().unwrap()) as usize);
        let be16 = |o: usize| d.get(o..o + 2).map(|b| u16::from_be_bytes(b.try_into().unwrap()) as usize);
        let (mut inf, mut dat, mut mid) = (None, None, None);
        let mut off = 0x20;
        while off + 8 <= d.len() {
            let size = be32(off + 4)?;
            match d.get(off..off + 4)? {
                b"INF1" => inf = Some(off),
                b"DAT1" => dat = Some(off),
                b"MID1" => mid = Some(off),
                _ => {}
            }
            if size < 8 {
                return None;
            }
            off += size;
        }
        let (inf, dat, mid) = (inf?, dat?, mid?);
        let n = be16(inf + 8)?;
        let entry = be16(inf + 10)?;
        if entry < 4 || be16(mid + 8)? != n {
            return None;
        }
        let mut by_id = HashMap::with_capacity(n);
        for i in 0..n {
            let text_off = be32(inf + 16 + i * entry)?;
            let id = be32(mid + 16 + i * 4)? as u32;
            let start = dat + 8 + text_off;
            let bytes = d.get(start..)?;
            // 1-byte text; 0x1A [total length, 1 byte] [payload] is a control code.
            let mut s = String::new();
            let mut p = 0;
            while let Some(&b) = bytes.get(p) {
                match b {
                    0 => break,
                    0x1A => p += (*bytes.get(p + 1)? as usize).max(2),
                    _ => {
                        s.push(b as char);
                        p += 1;
                    }
                }
            }
            by_id.insert(id, s);
        }
        Some(Messages { by_id })
    }

    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }

    /// Text of message `group` (the number in a `NNNN_00` id, as a plain number).
    pub fn group(&self, group: u32) -> Option<&str> {
        self.by_id.get(&(group << 8)).map(String::as_str)
    }
}

/// First message group of the names, of the "what is it" descriptions and of Louie's notes
/// (`TItemZukan`'s `0101_00`, `0401_00`, `0701_00`); entry `offset` is `base + offset`.
pub const NAME_GROUP: u32 = 101;
pub const DESC_GROUP: u32 = 401;
pub const NOTE_GROUP: u32 = 701;

/// A parsed set of treasure configs with their names.
#[derive(Debug, Clone, Default)]
pub struct Table {
    pub treasures: Vec<Config>,
    pub upgrades: Vec<Config>,
    messages: Messages,
}

impl Table {
    /// Reads `<dir>/config/*.txt` and `<dir>/message/pikmin2.bmg` (`dir` = the cache's `treasure/` folder).
    pub fn load(dir: &std::path::Path) -> Result<Table, String> {
        let read = |rel: &str| std::fs::read(dir.join(rel)).map_err(|e| format!("{rel}: {e}"));
        let text = |rel: &str| read(rel).map(|b| b.iter().map(|&c| c as char).collect::<String>());
        let treasures = parse_configs(&text("config/otakara_config.txt")?, Kind::Treasure).ok_or("otakara_config.txt")?;
        let upgrades = parse_configs(&text("config/item_config.txt")?, Kind::Upgrade).ok_or("item_config.txt")?;
        let messages = Messages::parse(&read("message/pikmin2.bmg")?).ok_or("pikmin2.bmg")?;
        Ok(Table { treasures, upgrades, messages })
    }

    pub fn all(&self) -> impl Iterator<Item = &Config> {
        self.treasures.iter().chain(&self.upgrades)
    }

    /// The zukan offset of `c` (`getOffsetFromDictionaryNo`): the index among the treasures, or the treasure
    /// count plus the index among the upgrades.
    pub fn offset(&self, c: &Config) -> usize {
        match c.kind {
            Kind::Treasure => c.index,
            Kind::Upgrade => self.treasures.len() + c.index,
        }
    }

    /// The in-game name with its line breaks as spaces ("Spouse Alert").
    pub fn display_name(&self, c: &Config) -> Option<String> {
        let t = self.messages.group(NAME_GROUP + self.offset(c) as u32)?;
        Some(t.split_whitespace().collect::<Vec<_>>().join(" "))
    }

    /// What the treasure is, in Olimar's words (the zukan's first description).
    pub fn description(&self, c: &Config) -> Option<&str> {
        self.messages.group(DESC_GROUP + self.offset(c) as u32)
    }

    pub fn find(&self, name: &str) -> Option<&Config> {
        self.all().find(|c| c.name == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "# c\n2\t# size\n{\n\tname\ta\n\tarchive\ta.szs\n\tmin\t3\n\tmax 6\n\tmoney 90\n\tunique yes\n\tindirect use\n\tend\n}\n{\n\tname b\n\tdepth_c 5\n\tend\n}\n";

    #[test]
    fn configs_parse() {
        let v = parse_configs(SAMPLE, Kind::Treasure).unwrap();
        assert_eq!(v.len(), 2);
        assert_eq!((v[0].name.as_str(), v[0].min, v[0].max, v[0].money), ("a", 3, 6, 90));
        assert!(v[0].unique && v[0].indirect == Indirect::Use);
        assert_eq!(v[0].model_dir(), "a");
        assert_eq!(v[1].index, 1);
        assert_eq!(v[1].depth_abcd, [10.0, 20.0, 5.0, 40.0]);
        assert!(parse_configs("3\n{\nname a\nend\n}\n", Kind::Treasure).is_none());
    }

    #[test]
    fn anim_mgr_and_coll_tree_parse() {
        let a = parse_anim_mgr("#\n1\n{\n\txxx\n\tcarry.bck\n\t8\t0 # loop start\n\t43\t1 # loop end\n\t-1\n}\n").unwrap();
        assert_eq!(a, AnimMgr { file: "carry.bck".into(), loop_start: 8.0, loop_end: 43.0 });
        let c = parse_coll_tree(
            "1 # n\n10.0 # r\n{none}\n{____}\n0 1 2\n0\n0\n{\n 0\n 5.0\n {none}\n {____}\n 0 0 0\n 0\n 0\n}\n",
        )
        .unwrap();
        assert_eq!(c.radius, 10.0);
        assert_eq!(c.offset, [0.0, 1.0, 2.0]);
        assert_eq!(c.flatten().len(), 2);
    }

    #[test]
    fn messages_parse_and_skip_control_codes() {
        // INF1 with 1 entry, DAT1 "Hi" + code + "!", MID1 id 5 << 8.
        let mut d = b"MESGbmg1".to_vec();
        d.extend([0; 24]);
        let mut inf = b"INF1".to_vec();
        inf.extend(24u32.to_be_bytes());
        inf.extend([0, 1, 0, 8, 0, 0, 0, 0]);
        inf.extend([0, 0, 0, 0, 0, 0, 0, 0]);
        let mut dat = b"DAT1".to_vec();
        let body = b"\0Hi\x1a\x04\x00\x01!\0";
        dat.extend((((body.len() + 8 + 7) & !7) as u32).to_be_bytes());
        dat.extend(body);
        while dat.len() % 8 != 0 {
            dat.push(0);
        }
        let mut mid = b"MID1".to_vec();
        mid.extend(20u32.to_be_bytes());
        mid.extend([0, 1, 0x10, 1, 0, 0, 0, 0]);
        mid.extend((5u32 << 8).to_be_bytes());
        // The entry's text offset is 1 (past the leading NUL).
        inf[16..20].copy_from_slice(&1u32.to_be_bytes());
        d.extend(inf);
        d.extend(dat);
        d.extend(mid);
        let m = Messages::parse(&d).unwrap();
        assert_eq!(m.group(5), Some("Hi!"));
    }

    fn cache() -> Option<std::path::PathBuf> {
        let dir = std::env::var_os("SMS_P2_ASSETS")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/p2"))
            .join("treasure");
        dir.join("config/otakara_config.txt").is_file().then_some(dir)
    }

    #[test]
    fn the_disc_table_loads() {
        let Some(dir) = cache() else { eprintln!("SKIP: no Pikmin 2 treasure cache"); return };
        let t = Table::load(&dir).expect("table");
        assert_eq!(t.treasures.len(), 188);
        assert_eq!(t.upgrades.len(), 13);
        assert!(t.all().all(|c| c.min >= 1 && c.max >= c.min && c.money > 0 && !c.bmd.is_empty()), "config sanity");
        // Names come from the message file, one per config.
        for c in t.all() {
            let n = t.display_name(c).unwrap_or_else(|| panic!("no name for {}", c.name));
            assert!(!n.is_empty(), "{}", c.name);
            assert!(t.description(c).is_some_and(|d| !d.is_empty()), "{}", c.name);
        }
        // The upgrades are named too: fue_a is the Power Punch, "Brute Knuckles".
        assert_eq!(t.display_name(t.find("fue_a").unwrap()).as_deref(), Some("Brute Knuckles"));
        // Every config's texts parse.
        let mut coll = 0;
        for c in t.all() {
            let kind = if c.kind == Kind::Treasure { "otakara" } else { "item" };
            let a = std::fs::read_to_string(dir.join(format!("texts/{kind}/{}/animmgr.txt", c.name))).unwrap_or_default();
            assert!(parse_anim_mgr(&a).is_some(), "animmgr {}", c.name);
            // dia_c_red, juji_key and loozy have no collinfo.txt on the disc, and pellets never read one.
            let Ok(s) = std::fs::read(dir.join(format!("texts/{kind}/{}/collinfo.txt", c.name))) else { continue };
            let s: String = s.iter().map(|&b| b as char).collect();
            assert!(parse_coll_tree(&s).is_some(), "collinfo {}", c.name);
            coll += 1;
        }
        assert_eq!(coll, 198);
    }
}
