use rts_base::{Command, IVec2, LoadError, MapData, PlayerId, Rules, Sim, TILE, UnitId, UnitKind};

const RULES: &str = include_str!("../../../assets/data/core.rules.ron");
const PROVING_GROUNDS: &str = include_str!("../../../assets/maps/proving_grounds.map.ron");

fn rules() -> Rules {
    Rules::from_ron(RULES).expect("bundled rules are valid")
}

fn sim_from(map: &str) -> Result<Sim, LoadError> {
    Sim::new(rules(), &MapData::from_ron(map)?)
}

/// Small valid map; tests break one thing at a time.
fn map(players: &str, tiles: &str, units: &str) -> String {
    format!("(name: \"t\", players: [{players}], tiles: [{tiles}], units: [{units}])")
}

const TILES: &str = r#""..~", "...""#;
const PLAYER: &str = "(start: (0.5, 0.5))";

#[test]
fn bundled_map_starts() {
    let map = MapData::from_ron(PROVING_GROUNDS).unwrap();
    let sim = Sim::new(rules(), &map).unwrap();
    assert_eq!(sim.terrain().size(), IVec2::new(48, 32));
    assert_eq!(sim.players().len(), 2);
    assert_eq!(sim.units().count(), map.units.len());
    assert_eq!(sim.players()[1].start, IVec2::new(42, 26) * TILE);
}

#[test]
fn spawns_units_in_file_order_with_full_health() {
    let sim = sim_from(&map(
        PLAYER,
        TILES,
        r#"(kind: "soldier", owner: 0, pos: (1.5, 0.5)), (kind: "worker", owner: 0, pos: (0.25, 1.75))"#,
    ))
    .unwrap();
    let units: Vec<_> = sim.units().collect();
    assert_eq!(units.len(), 2);
    assert_eq!(units[0].1.pos, IVec2::new(TILE * 3 / 2, TILE / 2));
    assert_eq!(units[0].1.health, 80);
    assert_eq!(units[1].1.pos, IVec2::new(TILE / 4, TILE * 7 / 4));
    assert!(units[0].0 < units[1].0);
}

#[test]
fn rejects_invalid_maps() {
    let cases = [
        (map(PLAYER, r#""..", "...""#, ""), "row 1 has 3 tiles"),
        (map(PLAYER, r#""..x""#, ""), "unknown terrain symbol `x`"),
        (map("", TILES, ""), "map has 0 players"),
        (map("(start: (2.5, 0.5))", TILES, ""), "player 0 starts"),
        (
            map(
                PLAYER,
                TILES,
                r#"(kind: "tank", owner: 0, pos: (0.5, 0.5))"#,
            ),
            "unknown kind `tank`",
        ),
        (
            map(
                PLAYER,
                TILES,
                r#"(kind: "worker", owner: 1, pos: (0.5, 0.5))"#,
            ),
            "missing player 1",
        ),
        (
            map(
                PLAYER,
                TILES,
                r#"(kind: "worker", owner: 0, pos: (2.5, 0.5))"#,
            ),
            "unit 0 is outside",
        ),
        (
            map(
                PLAYER,
                TILES,
                r#"(kind: "worker", owner: 0, pos: (0.5, 2.5))"#,
            ),
            "unit 0 is outside",
        ),
        (map(PLAYER, "", ""), "map is 0x0 tiles"),
    ];
    for (text, expected) in cases {
        let error = sim_from(&text)
            .err()
            .unwrap_or_else(|| panic!("accepted: {text}"));
        assert!(
            error.to_string().contains(expected),
            "`{error}` lacks `{expected}`"
        );
    }
}

#[test]
fn identical_setups_hash_identically() {
    let mut a = sim_from(PROVING_GROUNDS).unwrap();
    let mut b = sim_from(PROVING_GROUNDS).unwrap();
    assert_eq!(a.state_hash(), b.state_hash());
    a.step();
    assert_ne!(a.state_hash(), b.state_hash());
    b.step();
    assert_eq!(a.state_hash(), b.state_hash());
}

#[test]
fn sprite_paths_do_not_change_hashes_or_movement() {
    let map = MapData::from_ron(&map(
        PLAYER,
        TILES,
        r#"(kind: "worker", owner: 0, pos: (0.5, 0.5))"#,
    ))
    .unwrap();
    let changed = RULES.replace("textures/units/worker.png", "textures/units/custom.png");
    let mut original = Sim::new(rules(), &map).unwrap();
    let mut reskinned = Sim::new(Rules::from_ron(&changed).unwrap(), &map).unwrap();
    assert_ne!(
        original.rules().unit(UnitKind(0)).sprite,
        reskinned.rules().unit(UnitKind(0)).sprite
    );
    let unit = UnitId(0);
    let target = IVec2::new(TILE * 5 / 2, TILE * 3 / 2);
    for sim in [&mut original, &mut reskinned] {
        sim.queue_command(
            PlayerId(0),
            Command::Move {
                units: vec![unit],
                target,
            },
        );
    }
    for tick in 0..=80 {
        assert_eq!(original.state_hash(), reskinned.state_hash(), "tick {tick}");
        assert_eq!(
            original.unit(unit).unwrap().pos,
            reskinned.unit(unit).unwrap().pos,
            "tick {tick}"
        );
        assert_eq!(original.move_target(unit), reskinned.move_target(unit));
        if tick < 80 {
            original.step();
            reskinned.step();
        }
    }
    assert_eq!(original.unit(unit).unwrap().pos, target);
    assert_eq!(original.move_target(unit), None);
}

#[test]
fn visual_bounds_do_not_change_hashes_or_movement() {
    let map = MapData::from_ron(&map(
        PLAYER,
        TILES,
        r#"(kind: "worker", owner: 0, pos: (0.5, 0.5))"#,
    ))
    .unwrap();
    let mut sims: Vec<_> = [
        "",
        "visual_bounds: None,",
        "visual_bounds: Some((offset: (0.1, -0.2), size: (0.75, 0.5))),",
        "visual_bounds: Some((offset: (-2, 3), size: (4, 6))),",
    ]
    .into_iter()
    .map(|bounds| {
        let rules = Rules::from_ron(&format!(
            r##"(
                tile_capacity: 4,
                terrain: [
                    (id: "grass", symbol: '.', speed_modifier: 1, color: "#00ff00"),
                    (id: "water", symbol: '~', speed_modifier: 0, color: "#0000ff"),
                ],
                units: [(id: "worker", sprite: "textures/units/worker.png", size: 1, max_health: 40, radius: 0.25,
                         speed: 1.25, {bounds})],
            )"##
        ))
        .unwrap();
        Sim::new(rules, &map).unwrap()
    })
    .collect();
    assert_eq!(sims[0].rules().unit(UnitKind(0)).visual_bounds, None);
    assert_ne!(
        sims[2].rules().unit(UnitKind(0)).visual_bounds,
        sims[3].rules().unit(UnitKind(0)).visual_bounds
    );
    let unit = UnitId(0);
    let start = sims[0].unit(unit).unwrap().pos;
    let target = IVec2::new(TILE * 5 / 2, TILE * 3 / 2);
    for sim in &mut sims {
        sim.queue_command(
            PlayerId(0),
            Command::Move {
                units: vec![unit],
                target,
            },
        );
    }
    for tick in 0..=80 {
        let reference = &sims[0];
        for sim in &sims[1..] {
            assert_eq!(sim.state_hash(), reference.state_hash(), "tick {tick}");
            assert_eq!(
                sim.unit(unit).unwrap().pos,
                reference.unit(unit).unwrap().pos,
                "tick {tick}"
            );
            assert_eq!(sim.move_target(unit), reference.move_target(unit));
        }
        if tick < 80 {
            for sim in &mut sims {
                sim.step();
            }
        }
    }
    assert_ne!(start, target);
    assert_eq!(sims[0].unit(unit).unwrap().pos, target);
    assert_eq!(sims[0].move_target(unit), None);
}
