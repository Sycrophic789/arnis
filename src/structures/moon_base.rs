//! Deterministic surface habitat layouts for lunar terrain-only worlds.

use crate::block_definitions::*;
use crate::celestial::CelestialBody;
use crate::coordinate_system::cartesian::XZBBox;
use crate::deterministic_rng::coord_rng;
use crate::world_editor::WorldEditor;
use rand::Rng;

const SITE_HALF_X: i32 = 15;
const SITE_HALF_Z: i32 = 11;
const SITE_FLATNESS: i32 = 4;
const CANDIDATE_TRIES: usize = 96;
const CLEARANCE_HEIGHT: i32 = 10;
const SITE_SALT: u64 = 0x4D4F_4F4E_4241_5345;
const ROTATION_SALT: u64 = 0x4D4F_4F4E_524F_5441;
const LAYOUT_SALT: u64 = 0x4D4F_4F4E_4C41_594F;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MoonBaseLayout {
    /// Two wings branch west and east from the central lab.
    TwinWings,
    /// A habitation wing and a research wing branch north and east.
    ResearchHub,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Direction {
    North,
    East,
    South,
    West,
}

impl Direction {
    fn offset(self) -> (i32, i32) {
        match self {
            Self::North => (0, -1),
            Self::East => (1, 0),
            Self::South => (0, 1),
            Self::West => (-1, 0),
        }
    }

    fn opposite(self) -> Self {
        match self {
            Self::North => Self::South,
            Self::East => Self::West,
            Self::South => Self::North,
            Self::West => Self::East,
        }
    }

    fn rotate(self, turns: u8) -> Self {
        let (mut x, mut z) = self.offset();
        for _ in 0..turns % 4 {
            (x, z) = (-z, x);
        }
        match (x, z) {
            (0, -1) => Self::North,
            (1, 0) => Self::East,
            (0, 1) => Self::South,
            _ => Self::West,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::North => "north",
            Self::East => "east",
            Self::South => "south",
            Self::West => "west",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ModuleRole {
    Habitation,
    Research,
}

#[derive(Clone, Copy, Debug)]
struct ModuleSpec {
    x: i32,
    z: i32,
    connector: Direction,
    role: ModuleRole,
}

fn modules(layout: MoonBaseLayout) -> [ModuleSpec; 2] {
    match layout {
        MoonBaseLayout::TwinWings => [
            ModuleSpec {
                x: -10,
                z: 0,
                connector: Direction::West,
                role: ModuleRole::Habitation,
            },
            ModuleSpec {
                x: 10,
                z: 0,
                connector: Direction::East,
                role: ModuleRole::Research,
            },
        ],
        MoonBaseLayout::ResearchHub => [
            ModuleSpec {
                x: 0,
                z: -9,
                connector: Direction::North,
                role: ModuleRole::Habitation,
            },
            ModuleSpec {
                x: 10,
                z: 0,
                connector: Direction::East,
                role: ModuleRole::Research,
            },
        ],
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MoonSite {
    pub x: i32,
    pub z: i32,
    pub deck_y: i32,
    pub layout: MoonBaseLayout,
    /// Number of clockwise quarter-turns applied to the local plan.
    pub rotation: u8,
}

fn half_extents(rotation: u8) -> (i32, i32) {
    if rotation % 2 == 0 {
        (SITE_HALF_X, SITE_HALF_Z)
    } else {
        (SITE_HALF_Z, SITE_HALF_X)
    }
}

fn local_to_world(site: MoonSite, dx: i32, dz: i32) -> (i32, i32) {
    let (dx, dz) = match site.rotation % 4 {
        0 => (dx, dz),
        1 => (-dz, dx),
        2 => (-dx, -dz),
        _ => (dz, -dx),
    };
    (site.x + dx, site.z + dz)
}

fn put(editor: &mut WorldEditor, site: MoonSite, block: Block, dx: i32, dy: i32, dz: i32) {
    let (x, z) = local_to_world(site, dx, dz);
    editor.set_block_absolute(block, x, site.deck_y + dy, z, None, None);
}

fn put_with_properties(
    editor: &mut WorldEditor,
    site: MoonSite,
    block: BlockWithProperties,
    dx: i32,
    dy: i32,
    dz: i32,
) {
    let (x, z) = local_to_world(site, dx, dz);
    editor.set_block_with_properties_absolute(block, x, site.deck_y + dy, z, None, None);
}

fn layout_for_site(x: i32, z: i32) -> MoonBaseLayout {
    let mut rng = coord_rng(x, z, LAYOUT_SALT);
    if rng.random_range(0..2u8) == 0 {
        MoonBaseLayout::TwinWings
    } else {
        MoonBaseLayout::ResearchHub
    }
}

fn center_of(minimum: i32, maximum: i32) -> i32 {
    (i64::from(minimum) + (i64::from(maximum) - i64::from(minimum)) / 2) as i32
}

fn choose_rotation(bbox: &XZBBox) -> Option<u8> {
    let width = i64::from(bbox.max_x()) - i64::from(bbox.min_x()) + 1;
    let depth = i64::from(bbox.max_z()) - i64::from(bbox.min_z()) + 1;
    let landscape_fits =
        width >= i64::from(SITE_HALF_X * 2 + 1) && depth >= i64::from(SITE_HALF_Z * 2 + 1);
    let portrait_fits =
        width >= i64::from(SITE_HALF_Z * 2 + 1) && depth >= i64::from(SITE_HALF_X * 2 + 1);
    if !landscape_fits && !portrait_fits {
        return None;
    }

    let center_x = center_of(bbox.min_x(), bbox.max_x());
    let center_z = center_of(bbox.min_z(), bbox.max_z());
    let mut rng = coord_rng(center_x, center_z, ROTATION_SALT);

    match (landscape_fits, portrait_fits) {
        (true, false) => Some(rng.random_range(0u8..2) * 2),
        (false, true) => Some(1 + rng.random_range(0u8..2) * 2),
        (true, true) if width > depth => Some(rng.random_range(0u8..2) * 2),
        (true, true) if depth > width => Some(1 + rng.random_range(0u8..2) * 2),
        (true, true) => Some(rng.random_range(0u8..4)),
        (false, false) => None,
    }
}

fn site_has_clearance(editor: &WorldEditor, site: MoonSite) -> bool {
    let (half_x, half_z) = half_extents(site.rotation);
    for dx in -half_x..=half_x {
        for dz in -half_z..=half_z {
            let (x, z) = local_to_world(site, dx, dz);
            if (1..=CLEARANCE_HEIGHT)
                .any(|dy| editor.get_block_absolute(x, site.deck_y + dy, z).is_some())
            {
                return false;
            }
        }
    }
    true
}

/// Selects a stable, flat-enough site that fits the plan orientation and has clear build volume.
pub fn find_site(editor: &WorldEditor, bbox: &XZBBox, body: CelestialBody) -> Option<MoonSite> {
    if body != CelestialBody::Moon {
        return None;
    }
    let rotation = choose_rotation(bbox)?;
    let (half_x, half_z) = half_extents(rotation);
    let min_x = bbox.min_x() + half_x;
    let max_x = bbox.max_x() - half_x;
    let min_z = bbox.min_z() + half_z;
    let max_z = bbox.max_z() - half_z;
    if min_x > max_x || min_z > max_z {
        return None;
    }

    let center = (center_of(min_x, max_x), center_of(min_z, max_z));
    let mut candidates = vec![center];
    let mut rng = coord_rng(center.0, center.1, SITE_SALT);
    for _ in 0..CANDIDATE_TRIES {
        candidates.push((
            rng.random_range(min_x..=max_x),
            rng.random_range(min_z..=max_z),
        ));
    }

    candidates.into_iter().find_map(|(x, z)| {
        let site = MoonSite {
            x,
            z,
            deck_y: 0,
            layout: layout_for_site(x, z),
            rotation,
        };
        let (half_x, half_z) = half_extents(rotation);
        let mut min_y = i32::MAX;
        let mut max_y = i32::MIN;
        for dx in -half_x..=half_x {
            for dz in -half_z..=half_z {
                let (sx, sz) = local_to_world(site, dx, dz);
                let y = editor.get_ground_level(sx, sz);
                min_y = min_y.min(y);
                max_y = max_y.max(y);
            }
        }
        if max_y - min_y > SITE_FLATNESS {
            return None;
        }

        let site = MoonSite {
            deck_y: max_y + 1,
            ..site
        };
        site_has_clearance(editor, site).then_some(site)
    })
}

/// Adds a research habitat, with the module deck at site.deck_y.
pub fn place(editor: &mut WorldEditor, site: MoonSite) {
    if !site_has_clearance(editor, site) {
        return;
    }
    let (half_x, half_z) = half_extents(site.rotation);

    // Piled regolith footings level the deck without cutting into the terrain.
    for dx in -half_x..=half_x {
        for dz in -half_z..=half_z {
            let (x, z) = local_to_world(site, dx, dz);
            let ground = editor.get_ground_level(x, z);
            for y in ground + 1..site.deck_y {
                editor.set_block_absolute(POLISHED_DEEPSLATE, x, y, z, None, None);
            }
            put(editor, site, SMOOTH_STONE, dx, 0, dz);
        }
    }

    // Central pressurized lab dome. Doorways are cut where each pressure corridor meets it.
    for layer in 1..=6 {
        let radius: i32 = 6 - (layer - 1) / 2;
        for dx in -radius..=radius {
            for dz in -radius..=radius {
                let edge = dx.abs() == radius || dz.abs() == radius;
                if !edge || dx * dx + dz * dz > radius * radius + radius {
                    continue;
                }
                if layer <= 3 && dome_opening(dx, dz, radius, site.layout) {
                    continue;
                }
                let frame = dx == 0 || dz == 0 || layer == 6;
                put(
                    editor,
                    site,
                    if frame {
                        LIGHT_GRAY_CONCRETE
                    } else {
                        WHITE_STAINED_GLASS
                    },
                    dx,
                    layer,
                    dz,
                );
            }
        }
    }
    put(editor, site, SEA_LANTERN, 0, 2, 0);

    for module in modules(site.layout) {
        place_corridor(editor, site, module);
        place_module(editor, site, module);
    }

    place_airlock(editor, site);
    for (panel_x, panel_z) in solar_sites(site.layout) {
        place_solar_array(editor, site, panel_x, panel_z);
    }
}

fn dome_opening(dx: i32, dz: i32, radius: i32, layout: MoonBaseLayout) -> bool {
    let on_opening = |direction: Direction| match direction {
        Direction::North => dz == -radius && dx.abs() <= 1,
        Direction::East => dx == radius && dz.abs() <= 1,
        Direction::South => dz == radius && dx.abs() <= 1,
        Direction::West => dx == -radius && dz.abs() <= 1,
    };
    on_opening(Direction::South) || modules(layout).iter().any(|m| on_opening(m.connector))
}

fn place_corridor(editor: &mut WorldEditor, site: MoonSite, module: ModuleSpec) {
    let (direction_x, direction_z) = module.connector.offset();
    let distance = if direction_x != 0 {
        module.x.abs()
    } else {
        module.z.abs()
    };
    let (tangent_x, tangent_z) = (-direction_z, direction_x);

    // The corridor joins the dome at radius six and stops just before the module wall.
    for step in 6..distance - 2 {
        let dx = direction_x * step;
        let dz = direction_z * step;
        for side in [-1, 1] {
            for dy in 1..=3 {
                put(
                    editor,
                    site,
                    WHITE_STAINED_GLASS,
                    dx + tangent_x * side,
                    dy,
                    dz + tangent_z * side,
                );
            }
        }
        for side in -1..=1 {
            put(
                editor,
                site,
                LIGHT_GRAY_CONCRETE,
                dx + tangent_x * side,
                4,
                dz + tangent_z * side,
            );
        }
    }
}

fn place_module(editor: &mut WorldEditor, site: MoonSite, module: ModuleSpec) {
    let (wall, window) = match module.role {
        ModuleRole::Habitation => (WHITE_CONCRETE, LIGHT_BLUE_STAINED_GLASS),
        ModuleRole::Research => (LIGHT_GRAY_CONCRETE, BLUE_STAINED_GLASS),
    };

    for dx in -2i32..=2 {
        for dz in -2i32..=2 {
            let edge = dx.abs() == 2 || dz.abs() == 2;
            if edge {
                for dy in 1..=3 {
                    if module_doorway(module, dx, dz) {
                        continue;
                    }
                    let is_window = dy == 2 && (dx == 0 || dz == 0);
                    put(
                        editor,
                        site,
                        if is_window { window } else { wall },
                        module.x + dx,
                        dy,
                        module.z + dz,
                    );
                }
            }
            put(
                editor,
                site,
                if dx == 0 && dz == 0 {
                    WHITE_STAINED_GLASS
                } else {
                    wall
                },
                module.x + dx,
                4,
                module.z + dz,
            );
        }
    }

    match module.role {
        ModuleRole::Habitation => {
            put(editor, site, BARREL, module.x + 1, 1, module.z + 1);
            put(editor, site, SEA_LANTERN, module.x - 1, 3, module.z - 1);
        }
        ModuleRole::Research => {
            put(editor, site, CRAFTING_TABLE, module.x + 1, 1, module.z + 1);
            put(editor, site, LECTERN, module.x - 1, 1, module.z + 1);
            put(editor, site, SEA_LANTERN, module.x - 1, 3, module.z - 1);
        }
    }
}

fn module_doorway(module: ModuleSpec, dx: i32, dz: i32) -> bool {
    let entrance = module.connector.opposite();
    match entrance {
        Direction::North => dz == -2 && dx.abs() <= 1,
        Direction::East => dx == 2 && dz.abs() <= 1,
        Direction::South => dz == 2 && dx.abs() <= 1,
        Direction::West => dx == -2 && dz.abs() <= 1,
    }
}

fn place_airlock(editor: &mut WorldEditor, site: MoonSite) {
    // The south entry is a sealed vestibule with inner and outer paired pressure doors.
    for dx in -1i32..=1 {
        for dz in 7..=9 {
            let edge = dx.abs() == 1 || dz == 7 || dz == 9;
            for dy in 1..=3 {
                let door_position = dx == 0 && (dz == 7 || dz == 9) && dy <= 2;
                if edge && !door_position {
                    put(editor, site, WHITE_CONCRETE, dx, dy, dz);
                }
            }
            put(
                editor,
                site,
                if dx == 0 && dz == 8 {
                    SEA_LANTERN
                } else {
                    WHITE_CONCRETE
                },
                dx,
                4,
                dz,
            );
        }
    }
    for door_z in [7, 9] {
        put_with_properties(
            editor,
            site,
            door(IRON_DOOR, "lower", Direction::South, site.rotation),
            0,
            1,
            door_z,
        );
        put_with_properties(
            editor,
            site,
            door(IRON_DOOR, "upper", Direction::South, site.rotation),
            0,
            2,
            door_z,
        );
    }
}

fn solar_sites(layout: MoonBaseLayout) -> [(i32, i32); 2] {
    match layout {
        MoonBaseLayout::TwinWings => [(-10, -8), (10, -8)],
        MoonBaseLayout::ResearchHub => [(-10, -8), (-10, 8)],
    }
}

fn place_solar_array(editor: &mut WorldEditor, site: MoonSite, panel_x: i32, panel_z: i32) {
    for dx in -3i32..=3 {
        for dz in -1i32..=1 {
            let border = dx.abs() == 3 || dz.abs() == 1;
            put(
                editor,
                site,
                if border {
                    LIGHT_GRAY_CONCRETE
                } else {
                    BLUE_STAINED_GLASS
                },
                panel_x + dx,
                2,
                panel_z + dz,
            );
        }
    }
    for dx in [-2, 2] {
        put(editor, site, IRON_BLOCK, panel_x + dx, 1, panel_z);
    }
}

fn door(block: Block, half: &str, facing: Direction, rotation: u8) -> BlockWithProperties {
    let mut properties = std::collections::HashMap::new();
    properties.insert("half".to_string(), fastnbt::Value::String(half.to_string()));
    properties.insert(
        "facing".to_string(),
        fastnbt::Value::String(facing.rotate(rotation).as_str().to_string()),
    );
    properties.insert(
        "hinge".to_string(),
        fastnbt::Value::String("left".to_string()),
    );
    properties.insert(
        "open".to_string(),
        fastnbt::Value::String("false".to_string()),
    );
    BlockWithProperties::new(block, Some(fastnbt::Value::Compound(properties)))
}

pub fn deferred_region_keys(site: MoonSite) -> impl Iterator<Item = (i32, i32)> {
    let (half_x, half_z) = half_extents(site.rotation);
    let first = local_to_world(site, -half_x, -half_z);
    let opposite = local_to_world(site, half_x, half_z);
    let min_world_x = first.0.min(opposite.0);
    let max_world_x = first.0.max(opposite.0);
    let min_world_z = first.1.min(opposite.1);
    let max_world_z = first.1.max(opposite.1);
    (min_world_x >> 9..=max_world_x >> 9)
        .flat_map(move |rx| (min_world_z >> 9..=max_world_z >> 9).map(move |rz| (rx, rz)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coordinate_system::cartesian::XZPoint;
    use crate::coordinate_system::geographic::LLBBox;
    use std::path::PathBuf;

    fn editor(bbox: &XZBBox) -> WorldEditor<'_> {
        WorldEditor::new(
            PathBuf::from("/dev/null/moon-base-test"),
            bbox,
            LLBBox::new(0.0, 0.0, 0.01, 0.01).unwrap(),
        )
    }

    fn property(ed: &WorldEditor<'_>, x: i32, y: i32, z: i32, key: &str) -> Option<fastnbt::Value> {
        match ed.block_properties_absolute(x, y, z)? {
            fastnbt::Value::Compound(properties) => properties.get(key).cloned(),
            _ => None,
        }
    }

    fn explicit_site(layout: MoonBaseLayout, rotation: u8) -> MoonSite {
        MoonSite {
            x: 50,
            z: 50,
            deck_y: 10,
            layout,
            rotation,
        }
    }

    fn snapshot(editor: &WorldEditor<'_>, site: MoonSite) -> Vec<Option<Block>> {
        (-16..=16)
            .flat_map(|dx| {
                (-16..=16).flat_map(move |dz| {
                    (0..=CLEARANCE_HEIGHT).map(move |dy| {
                        let (x, z) = local_to_world(site, dx, dz);
                        editor.get_block_absolute(x, site.deck_y + dy, z)
                    })
                })
            })
            .collect()
    }

    #[test]
    fn moon_base_is_moon_only_and_requires_a_fitting_footprint() {
        let bbox = XZBBox::rect_from_min_max(0, 0, 40, 40).unwrap();
        let ed = editor(&bbox);
        assert!(find_site(&ed, &bbox, CelestialBody::Moon).is_some());
        assert!(find_site(&ed, &bbox, CelestialBody::Earth).is_none());
        assert!(find_site(&ed, &bbox, CelestialBody::Mars).is_none());
        let small = XZBBox::rect_from_min_max(0, 0, 20, 20).unwrap();
        assert!(find_site(&ed, &small, CelestialBody::Moon).is_none());
    }

    #[test]
    fn layout_selection_is_deterministic_and_has_multiple_plans() {
        let mut saw_twin = false;
        let mut saw_hub = false;
        for x in (-100..=100).step_by(7) {
            let first = layout_for_site(x, -37);
            assert_eq!(first, layout_for_site(x, -37));
            saw_twin |= first == MoonBaseLayout::TwinWings;
            saw_hub |= first == MoonBaseLayout::ResearchHub;
        }
        assert!(saw_twin && saw_hub);
    }

    #[test]
    fn layout_orientation_fits_the_available_long_axis() {
        let landscape = XZBBox::rect_from_min_max(0, 0, 30, 22).unwrap();
        let portrait = XZBBox::rect_from_min_max(0, 0, 22, 30).unwrap();
        let too_narrow = XZBBox::rect_from_min_max(0, 0, 21, 60).unwrap();

        assert!(choose_rotation(&landscape).unwrap() % 2 == 0);
        assert!(choose_rotation(&portrait).unwrap() % 2 == 1);
        assert_eq!(None, choose_rotation(&too_narrow));
        assert_eq!(choose_rotation(&landscape), choose_rotation(&landscape));
    }

    #[test]
    fn moon_base_selection_is_deterministic_and_places_layout_roles() {
        let bbox = XZBBox::rect_from_min_max(0, 0, 40, 40).unwrap();
        let mut ed = editor(&bbox);
        let site = find_site(&ed, &bbox, CelestialBody::Moon).unwrap();
        assert_eq!(Some(site), find_site(&ed, &bbox, CelestialBody::Moon));
        place(&mut ed, site);

        assert_eq!(
            Some(SMOOTH_STONE),
            ed.get_block_absolute(site.x, site.deck_y, site.z)
        );
        for module in modules(site.layout) {
            let expected = match module.role {
                ModuleRole::Habitation => BARREL,
                ModuleRole::Research => CRAFTING_TABLE,
            };
            let (furniture_x, furniture_z) = local_to_world(site, module.x + 1, module.z + 1);
            assert_eq!(
                Some(expected),
                ed.get_block_absolute(furniture_x, site.deck_y + 1, furniture_z)
            );
        }
        for (panel_x, panel_z) in solar_sites(site.layout) {
            let (x, z) = local_to_world(site, panel_x, panel_z);
            assert_eq!(
                Some(BLUE_STAINED_GLASS),
                ed.get_block_absolute(x, site.deck_y + 2, z)
            );
        }
        let (inner_x, inner_z) = local_to_world(site, 0, 7);
        let (outer_x, outer_z) = local_to_world(site, 0, 9);
        assert_eq!(
            Some(IRON_DOOR),
            ed.get_block_absolute(inner_x, site.deck_y + 1, inner_z)
        );
        assert_eq!(
            Some(IRON_DOOR),
            ed.get_block_absolute(outer_x, site.deck_y + 1, outer_z)
        );
        assert_eq!(
            Some(fastnbt::Value::String("upper".into())),
            property(&ed, outer_x, site.deck_y + 2, outer_z, "half")
        );
        let facing = Direction::South.rotate(site.rotation).as_str();
        assert_eq!(
            Some(fastnbt::Value::String(facing.into())),
            property(&ed, outer_x, site.deck_y + 1, outer_z, "facing")
        );
    }

    #[test]
    fn every_layout_connects_modules_to_the_dome_and_airlock() {
        for layout in [MoonBaseLayout::TwinWings, MoonBaseLayout::ResearchHub] {
            for rotation in 0..4 {
                let bbox = XZBBox::rect_from_min_max(-100, -100, 200, 200).unwrap();
                let mut ed = editor(&bbox);
                let site = explicit_site(layout, rotation);
                place(&mut ed, site);

                for module in modules(layout) {
                    let (direction_x, direction_z) = module.connector.offset();
                    let distance = if direction_x != 0 {
                        module.x.abs()
                    } else {
                        module.z.abs()
                    };
                    for step in 6..=distance - 2 {
                        let (x, z) = local_to_world(site, direction_x * step, direction_z * step);
                        for dy in 1..=3 {
                            assert_eq!(
                                None,
                                ed.get_block_absolute(x, site.deck_y + dy, z),
                                "{layout:?} rotation {rotation} has an obstructed corridor"
                            );
                        }
                    }
                    let doorway_x = module.x - direction_x * 2;
                    let doorway_z = module.z - direction_z * 2;
                    for tangent in -1..=1 {
                        let dx = doorway_x - direction_z * tangent;
                        let dz = doorway_z + direction_x * tangent;
                        let (x, z) = local_to_world(site, dx, dz);
                        for dy in 1..=3 {
                            assert_eq!(
                                None,
                                ed.get_block_absolute(x, site.deck_y + dy, z),
                                "{layout:?} rotation {rotation} has a blocked module doorway"
                            );
                        }
                    }
                }

                let (entry_x, entry_z) = local_to_world(site, 0, 6);
                for dy in 1..=3 {
                    assert_eq!(
                        None,
                        ed.get_block_absolute(entry_x, site.deck_y + dy, entry_z),
                        "{layout:?} rotation {rotation} has a blocked dome entry"
                    );
                }
            }
        }
    }

    #[test]
    fn site_search_skips_a_blocked_center_and_uses_an_open_fallback() {
        let bbox = XZBBox::rect_from_min_max(0, 0, 100, 100).unwrap();
        let mut ed = editor(&bbox);
        ed.set_block_absolute(STONE, 50, 5, 50, None, None);

        let site = find_site(&ed, &bbox, CelestialBody::Moon).unwrap();
        assert_ne!((50, 50), (site.x, site.z));
        assert!(site_has_clearance(&ed, site));
        place(&mut ed, site);
        assert_eq!(Some(STONE), ed.get_block_absolute(50, 5, 50));
    }

    #[test]
    fn site_search_returns_none_when_every_candidate_is_obstructed() {
        let bbox = XZBBox::rect_from_min_max(0, 0, 40, 40).unwrap();
        let mut ed = editor(&bbox);
        for x in bbox.min_x()..=bbox.max_x() {
            for z in bbox.min_z()..=bbox.max_z() {
                ed.set_block_absolute(STONE, x, 5, z, None, None);
            }
        }

        assert_eq!(None, find_site(&ed, &bbox, CelestialBody::Moon));
    }

    #[test]
    fn generated_blocks_stay_inside_the_selected_bbox() {
        let bbox = XZBBox::rect_from_min_max(0, 0, 40, 40).unwrap();
        let mut ed = editor(&bbox);
        let site = find_site(&ed, &bbox, CelestialBody::Moon).unwrap();
        let (ground_x, ground_z) = local_to_world(site, SITE_HALF_X, SITE_HALF_Z);
        ed.set_block_absolute(STONE, ground_x, site.deck_y - 1, ground_z, None, None);
        place(&mut ed, site);
        assert_eq!(
            Some(STONE),
            ed.get_block_absolute(ground_x, site.deck_y - 1, ground_z),
            "footings must remain above the existing ground block"
        );

        for x in -1..=41 {
            for z in -1..=41 {
                if bbox.contains(&XZPoint::new(x, z)) {
                    continue;
                }
                for y in site.deck_y..=site.deck_y + CLEARANCE_HEIGHT {
                    assert_eq!(
                        None,
                        ed.get_block_absolute(x, y, z),
                        "structure escaped the selected generation bounds at ({x}, {y}, {z})"
                    );
                }
            }
        }
    }

    #[test]
    fn rotated_plan_stays_inside_a_portrait_bbox() {
        let bbox = XZBBox::rect_from_min_max(0, 0, 22, 30).unwrap();
        let mut ed = editor(&bbox);
        let site = find_site(&ed, &bbox, CelestialBody::Moon).unwrap();
        assert_eq!(1, site.rotation % 2);
        place(&mut ed, site);

        for x in -1..=23 {
            for z in -1..=31 {
                if bbox.contains(&XZPoint::new(x, z)) {
                    continue;
                }
                for y in site.deck_y..=site.deck_y + CLEARANCE_HEIGHT {
                    assert_eq!(
                        None,
                        ed.get_block_absolute(x, y, z),
                        "rotated structure escaped the selected generation bounds at ({x}, {y}, {z})"
                    );
                }
            }
        }
    }

    #[test]
    fn placement_is_deterministic_for_the_same_selected_site() {
        let bbox = XZBBox::rect_from_min_max(0, 0, 60, 60).unwrap();
        let first_site = find_site(&editor(&bbox), &bbox, CelestialBody::Moon).unwrap();
        let second_site = find_site(&editor(&bbox), &bbox, CelestialBody::Moon).unwrap();
        assert_eq!(first_site, second_site);

        let mut first = editor(&bbox);
        let mut second = editor(&bbox);
        place(&mut first, first_site);
        place(&mut second, second_site);
        assert_eq!(snapshot(&first, first_site), snapshot(&second, second_site));
    }

    #[test]
    fn moon_base_rejects_an_obstructed_volume() {
        let bbox = XZBBox::rect_from_min_max(0, 0, 40, 40).unwrap();
        let mut ed = editor(&bbox);
        let site = find_site(&ed, &bbox, CelestialBody::Moon).unwrap();
        let (x, z) = local_to_world(site, 0, 0);
        ed.set_block_absolute(STONE, x, site.deck_y + 4, z, None, None);
        place(&mut ed, site);
        assert_eq!(Some(STONE), ed.get_block_absolute(x, site.deck_y + 4, z));
        assert_eq!(None, ed.get_block_absolute(site.x, site.deck_y, site.z));
    }
}
