use rts_base::{IVec2, LoadError, MapData, Rules, Sim, TILE};

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
