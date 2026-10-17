//! OSM-tagged reinforced bunkers, carved after terrain filling.

use crate::block_definitions::*;
use crate::coordinate_system::cartesian::XZBBox;
use crate::element_processing::get_nearest_road_block;
use crate::floodfill_cache::RoadMaskBitmap;
use crate::osm_parser::ProcessedElement;
use crate::world_editor::{min_y, WorldEditor};
use fastnbt::Value;
use std::collections::HashMap;

const MAX_BUNKER_SIDE: i32 = 17;
const DEFAULT_BUNKER_SIDE: i32 = 11;
const BUNKER_ROCK: &[Block] = crate::caves::CAVE_HOST;
const SURFACE_HOST: &[Block] = &[
    STONE,
    DIRT,
    GRASS_BLOCK,
    GRAVEL,
    ANDESITE,
    GRANITE,
    DIORITE,
    TUFF,
    SAND,
    SANDSTONE,
    SNOW_BLOCK,
    TERRACOTTA,
    RED_TERRACOTTA,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BunkerFeature {
    pub id: u64,
    pub x: i32,
    pub z: i32,
    pub width: i32,
    pub depth: i32,
    pub entrance_facing: &'static str,
}

pub fn is_bunker(tags: &HashMap<String, String>) -> bool {
    tags.get("military").map(String::as_str) == Some("bunker")
        || tags.get("building").map(String::as_str) == Some("bunker")
        || tags.contains_key("bunker")
        || tags.contains_key("bunker_type")
}

/// Extracts explicitly mapped bunker areas/nodes. No untagged or probabilistic placement occurs.
pub fn collect(elements: &[ProcessedElement], road_mask: &RoadMaskBitmap) -> Vec<BunkerFeature> {
    let mut features: Vec<_> = elements
        .iter()
        .filter_map(|element| feature_for_element(element, road_mask))
        .collect();
    features.sort_by_key(|feature| feature.id);
    features
}

fn feature_for_element(
    element: &ProcessedElement,
    road_mask: &RoadMaskBitmap,
) -> Option<BunkerFeature> {
    if !is_bunker(element.tags()) {
        return None;
    }
    let nodes: Vec<_> = element.nodes().collect();
    let node = nodes.first()?;
    let (x, z, width, depth) = if nodes.len() == 1 {
        (node.x, node.z, DEFAULT_BUNKER_SIDE, DEFAULT_BUNKER_SIDE)
    } else {
        let min_x = nodes.iter().map(|n| n.x).min()?;
        let max_x = nodes.iter().map(|n| n.x).max()?;
        let min_z = nodes.iter().map(|n| n.z).min()?;
        let max_z = nodes.iter().map(|n| n.z).max()?;
        let width = (max_x - min_x - 1).min(MAX_BUNKER_SIDE);
        let depth = (max_z - min_z - 1).min(MAX_BUNKER_SIDE);
        if width < 7 || depth < 7 {
            return None;
        }
        ((min_x + max_x) / 2, (min_z + max_z) / 2, width, depth)
    };
    Some(BunkerFeature {
        id: element.id(),
        x,
        z,
        width,
        depth,
        entrance_facing: facing_to_road(x, z, width, depth, road_mask)
            .unwrap_or_else(|| ["north", "east", "south", "west"][(element.id() & 3) as usize]),
    })
}

fn facing_to_road(
    x: i32,
    z: i32,
    width: i32,
    depth: i32,
    road_mask: &RoadMaskBitmap,
) -> Option<&'static str> {
    let candidates = [
        (x, z - depth / 2, "north"),
        (x + width / 2, z, "east"),
        (x, z + depth / 2, "south"),
        (x - width / 2, z, "west"),
    ];
    candidates
        .into_iter()
        .filter_map(|(cx, cz, outward)| {
            get_nearest_road_block(cx, cz, 24, road_mask)
                .map(|(rx, rz)| ((rx - cx).abs() + (rz - cz).abs(), outward))
        })
        .min_by_key(|(distance, direction)| (*distance, direction_order(direction)))
        .map(|(_, direction)| direction)
}

fn direction_order(direction: &str) -> u8 {
    match direction {
        "north" => 0,
        "east" => 1,
        "south" => 2,
        _ => 3,
    }
}

impl BunkerFeature {
    pub fn deferred_region_keys(&self) -> impl Iterator<Item = (i32, i32)> {
        let min_x = self.x - self.width / 2 - 2;
        let max_x = self.x + self.width / 2 + 2;
        let min_z = self.z - self.depth / 2 - 2;
        let max_z = self.z + self.depth / 2 + 2;
        (min_x >> 9..=max_x >> 9)
            .flat_map(move |rx| (min_z >> 9..=max_z >> 9).map(move |rz| (rx, rz)))
    }
}

/// Carves and finishes one bunker after ground/cave filling. Returns false if terrain or existing
/// structures leave no safe, supported build volume.
pub fn place(
    editor: &mut WorldEditor,
    feature: BunkerFeature,
    road_mask: &RoadMaskBitmap,
    bbox: &XZBBox,
) -> bool {
    let x0 = feature.x - feature.width / 2;
    let x1 = x0 + feature.width - 1;
    let z0 = feature.z - feature.depth / 2;
    let z1 = z0 + feature.depth - 1;
    if x0 < bbox.min_x() || x1 > bbox.max_x() || z0 < bbox.min_z() || z1 > bbox.max_z() {
        return false;
    }

    let mut min_surface = i32::MAX;
    let mut max_surface = i32::MIN;
    for (sx, sz) in [
        (x0, z0),
        (x1, z0),
        (x0, z1),
        (x1, z1),
        (feature.x, feature.z),
    ] {
        let surface = editor.get_ground_level(sx, sz);
        min_surface = min_surface.min(surface);
        max_surface = max_surface.max(surface);
        if editor.get_block_absolute(sx, surface, sz) == Some(WATER) {
            return false;
        }
    }
    if max_surface - min_surface > 2 {
        return false;
    }
    let ground_y = editor.get_ground_level(feature.x, feature.z);
    let boundary_access = entrance_coords(feature, x0, x1, z0, z1);
    let vents = [(x0 + 2, z0 + 2), (x1 - 2, z1 - 2)];
    if vents.iter().any(|&(vx, vz)| {
        editor.get_block_absolute(vx, editor.get_ground_level(vx, vz), vz) == Some(WATER)
    }) {
        return false;
    }

    // A deep chamber stays below grade. Near vanilla's world floor, use a shallow
    // earth-banked room instead of writing below legal build height.
    let underground = ground_y - min_y() >= 6;
    let (floor_y, roof_y) = if underground {
        (ground_y - 5, ground_y - 1)
    } else {
        (ground_y - 1, ground_y + 3)
    };
    let (access_x, access_z) = if underground {
        match feature.entrance_facing {
            "east" => (x1 - 1, feature.z),
            "south" => (feature.x, z1 - 1),
            "west" => (x0 + 1, feature.z),
            _ => (feature.x, z0 + 1),
        }
    } else {
        boundary_access
    };
    if road_mask.contains(access_x, access_z) {
        return false;
    }
    let support = if underground {
        let (out_dx, out_dz) = facing_offset(feature.entrance_facing);
        let support_x = access_x + out_dx;
        let support_z = access_z + out_dz;
        if road_mask.contains(support_x, support_z)
            || editor.get_ground_level(support_x, support_z) != ground_y
            || !editor
                .get_block_absolute(support_x, ground_y, support_z)
                .is_some_and(|block| SURFACE_HOST.contains(&block))
            || editor
                .get_block_absolute(support_x, ground_y + 1, support_z)
                .is_some()
        {
            return false;
        }
        Some((support_x, support_z))
    } else {
        None
    };
    if floor_y < min_y() || roof_y > crate::world_editor::world_max_y() {
        return false;
    }

    let (clear_min, clear_max) = if underground {
        (floor_y, ground_y - 1)
    } else {
        (floor_y, roof_y)
    };
    for bx in x0..=x1 {
        for bz in z0..=z1 {
            for y in clear_min..=clear_max {
                if let Some(existing) = editor.get_block_absolute(bx, y, bz) {
                    let natural = BUNKER_ROCK.contains(&existing)
                        || (!underground && y == ground_y && SURFACE_HOST.contains(&existing));
                    if !natural {
                        return false;
                    }
                }
            }
        }
    }
    // Keep the surface hatch clear of trees/buildings and preserve its supporting apron.
    if [ground_y + 1, ground_y + 2]
        .into_iter()
        .any(|y| editor.get_block_absolute(access_x, y, access_z).is_some())
    {
        return false;
    }

    // Reinforced shell; the inner volume replaces only natural host rock.
    for bx in x0..=x1 {
        for bz in z0..=z1 {
            let boundary = bx == x0 || bx == x1 || bz == z0 || bz == z1;
            editor.set_block_absolute(
                SMOOTH_STONE,
                bx,
                floor_y,
                bz,
                Some(if underground {
                    BUNKER_ROCK
                } else {
                    SURFACE_HOST
                }),
                None,
            );
            for y in floor_y + 1..roof_y {
                if boundary {
                    let whitelist = if !underground && y == ground_y {
                        SURFACE_HOST
                    } else {
                        BUNKER_ROCK
                    };
                    editor.set_block_absolute(
                        if y == floor_y + 1 {
                            DEEPSLATE_BRICKS
                        } else {
                            GRAY_CONCRETE
                        },
                        bx,
                        y,
                        bz,
                        Some(whitelist),
                        None,
                    );
                } else {
                    let whitelist = if !underground && y == ground_y {
                        SURFACE_HOST
                    } else {
                        BUNKER_ROCK
                    };
                    editor.set_block_absolute(AIR, bx, y, bz, Some(whitelist), None);
                }
            }
            // Leave a roof opening for the shaft in deep sites; shallow sites use a side door.
            if !underground || (bx, bz) != (access_x, access_z) {
                editor.set_block_absolute(
                    if (bx - feature.x).abs() % 4 == 0 {
                        IRON_BLOCK
                    } else {
                        GRAY_CONCRETE
                    },
                    bx,
                    roof_y,
                    bz,
                    Some(BUNKER_ROCK),
                    None,
                );
            }
        }
    }

    // Two narrow iron-bar ventilation ducts connect the chamber to the surface. In a
    // shallow low-elevation site, they terminate as short grilles above the roof instead.
    for (vent_x, vent_z) in vents {
        if underground {
            for y in roof_y..ground_y {
                let allowed = if y == roof_y {
                    &[GRAY_CONCRETE, IRON_BLOCK][..]
                } else {
                    BUNKER_ROCK
                };
                editor.set_block_absolute(AIR, vent_x, y, vent_z, Some(allowed), None);
                editor.set_block_absolute(IRON_BARS, vent_x, y, vent_z, Some(&[AIR]), None);
            }
            editor.set_block_absolute(
                IRON_BARS,
                vent_x,
                ground_y,
                vent_z,
                Some(SURFACE_HOST),
                None,
            );
        } else {
            editor.set_block_absolute(
                IRON_BARS,
                vent_x,
                roof_y,
                vent_z,
                Some(&[GRAY_CONCRETE, IRON_BLOCK]),
                None,
            );
        }
    }

    // Secure surface entry. The deeper design uses a ladder shaft and closed steel hatch;
    // shallow sites use an outward-facing paired iron door at grade.
    if underground {
        for y in floor_y + 1..ground_y {
            editor.set_block_absolute(AIR, access_x, y, access_z, Some(BUNKER_ROCK), None);
        }
        let ladder_facing = opposite(feature.entrance_facing);
        let ladder = ladder_block(ladder_facing);
        for y in floor_y + 1..ground_y {
            editor.set_block_with_properties_absolute(
                ladder.clone(),
                access_x,
                y,
                access_z,
                Some(&[AIR]),
                None,
            );
        }
        editor.set_block_with_properties_absolute(
            trapdoor(feature.entrance_facing),
            access_x,
            ground_y + 1,
            access_z,
            None,
            None,
        );
        editor.set_block_absolute(AIR, access_x, ground_y, access_z, Some(SURFACE_HOST), None);
        editor.set_block_with_properties_absolute(
            ladder,
            access_x,
            ground_y,
            access_z,
            Some(&[AIR]),
            None,
        );
        let (support_x, support_z) = support.expect("underground entry support was prechecked");
        editor.set_block_absolute(
            LIGHT_GRAY_CONCRETE,
            support_x,
            ground_y,
            support_z,
            Some(SURFACE_HOST),
            None,
        );
        editor.set_block_absolute(
            LIGHT_GRAY_CONCRETE,
            support_x,
            ground_y + 1,
            support_z,
            None,
            None,
        );
    } else {
        let (door_x, door_z) = (access_x, access_z);
        let wall_allowed = if matches!(feature.entrance_facing, "east" | "west") {
            door_x == x0 || door_x == x1
        } else {
            door_z == z0 || door_z == z1
        };
        if wall_allowed {
            editor.set_block_absolute(AIR, door_x, ground_y, door_z, Some(SURFACE_HOST), None);
            editor.set_block_with_properties_absolute(
                door_block("lower", feature.entrance_facing),
                door_x,
                ground_y + 1,
                door_z,
                Some(&[DEEPSLATE_BRICKS, GRAY_CONCRETE]),
                None,
            );
            editor.set_block_with_properties_absolute(
                door_block("upper", feature.entrance_facing),
                door_x,
                ground_y + 2,
                door_z,
                Some(&[DEEPSLATE_BRICKS, GRAY_CONCRETE]),
                None,
            );
        }
        // Low earthen shoulders partly bury the shallow reinforced shell.
        for (bx, bz) in [
            (x0 - 1, feature.z),
            (x1 + 1, feature.z),
            (feature.x, z0 - 1),
            (feature.x, z1 + 1),
        ] {
            if bbox.contains(&crate::coordinate_system::cartesian::XZPoint::new(bx, bz))
                && (bx, bz) != (door_x, door_z)
            {
                editor.set_block_absolute(DIRT, bx, ground_y + 1, bz, None, None);
            }
        }
    }

    // Low-light security fixtures keep the underground room usable.
    for (lx, lz) in [
        (x0 + 1, z0 + 1),
        (x1 - 1, z0 + 1),
        (x0 + 1, z1 - 1),
        (x1 - 1, z1 - 1),
    ] {
        editor.set_block_absolute(SEA_LANTERN, lx, floor_y + 1, lz, None, None);
    }
    true
}

fn entrance_coords(feature: BunkerFeature, x0: i32, x1: i32, z0: i32, z1: i32) -> (i32, i32) {
    match feature.entrance_facing {
        "east" => (x1, feature.z),
        "south" => (feature.x, z1),
        "west" => (x0, feature.z),
        _ => (feature.x, z0),
    }
}

fn opposite(facing: &str) -> &'static str {
    match facing {
        "north" => "south",
        "south" => "north",
        "west" => "east",
        _ => "west",
    }
}

fn facing_offset(facing: &str) -> (i32, i32) {
    match facing {
        "east" => (1, 0),
        "south" => (0, 1),
        "west" => (-1, 0),
        _ => (0, -1),
    }
}

fn door_block(half: &str, facing: &str) -> BlockWithProperties {
    let mut properties = HashMap::new();
    properties.insert("half".to_string(), Value::String(half.to_string()));
    properties.insert("facing".to_string(), Value::String(facing.to_string()));
    properties.insert("hinge".to_string(), Value::String("left".to_string()));
    properties.insert("open".to_string(), Value::String("false".to_string()));
    properties.insert("powered".to_string(), Value::String("false".to_string()));
    BlockWithProperties::new(IRON_DOOR, Some(Value::Compound(properties)))
}

fn trapdoor(facing: &str) -> BlockWithProperties {
    let mut properties = HashMap::new();
    properties.insert("facing".to_string(), Value::String(facing.to_string()));
    properties.insert("half".to_string(), Value::String("bottom".to_string()));
    properties.insert("open".to_string(), Value::String("false".to_string()));
    properties.insert("powered".to_string(), Value::String("false".to_string()));
    BlockWithProperties::new(IRON_TRAPDOOR, Some(Value::Compound(properties)))
}

fn ladder_block(facing: &str) -> BlockWithProperties {
    let mut properties = HashMap::new();
    properties.insert("facing".to_string(), Value::String(facing.to_string()));
    BlockWithProperties::new(LADDER, Some(Value::Compound(properties)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coordinate_system::geographic::LLBBox;
    use crate::floodfill_cache::CoordinateBitmap;
    use crate::osm_parser::{ProcessedNode, ProcessedWay};
    use std::path::PathBuf;
    use std::sync::Arc;

    fn tags(items: &[(&str, &str)]) -> HashMap<String, String> {
        items
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn editor(bbox: &XZBBox) -> WorldEditor<'_> {
        WorldEditor::new(
            PathBuf::from("/dev/null/bunker-test"),
            bbox,
            LLBBox::new(0.0, 0.0, 0.01, 0.01).unwrap(),
        )
    }

    fn fill_rock(editor: &mut WorldEditor, cx: i32, cz: i32) {
        for x in cx - 12..=cx + 12 {
            for z in cz - 12..=cz + 12 {
                for y in -10..=0 {
                    editor.set_block_absolute(STONE, x, y, z, None, None);
                }
            }
        }
    }

    #[test]
    fn only_explicit_bunker_tags_create_features() {
        assert!(is_bunker(&tags(&[("military", "bunker")])));
        assert!(is_bunker(&tags(&[("building", "bunker")])));
        assert!(is_bunker(&tags(&[("bunker_type", "pillbox")])));
        assert!(!is_bunker(&tags(&[("building", "warehouse")])));

        let bbox = XZBBox::rect_from_min_max(0, 0, 50, 50).unwrap();
        let roads = CoordinateBitmap::new(&bbox);
        let regular = ProcessedElement::Way(ProcessedWay {
            id: 1,
            nodes: vec![
                ProcessedNode {
                    id: 10,
                    x: 10,
                    z: 10,
                    tags: HashMap::new(),
                },
                ProcessedNode {
                    id: 11,
                    x: 25,
                    z: 10,
                    tags: HashMap::new(),
                },
                ProcessedNode {
                    id: 12,
                    x: 25,
                    z: 25,
                    tags: HashMap::new(),
                },
                ProcessedNode {
                    id: 13,
                    x: 10,
                    z: 25,
                    tags: HashMap::new(),
                },
            ],
            tags: tags(&[("building", "warehouse")]),
        });
        assert!(collect(&[regular], &roads).is_empty());
    }

    #[test]
    fn bunker_selection_uses_road_side_and_is_stable() {
        let bbox = XZBBox::rect_from_min_max(0, 0, 60, 60).unwrap();
        let mut roads = CoordinateBitmap::new(&bbox);
        for z in 0..=60 {
            roads.set(31, z);
        }
        let bunker = ProcessedElement::Way(ProcessedWay {
            id: 22,
            nodes: vec![
                ProcessedNode {
                    id: 1,
                    x: 10,
                    z: 10,
                    tags: HashMap::new(),
                },
                ProcessedNode {
                    id: 2,
                    x: 24,
                    z: 10,
                    tags: HashMap::new(),
                },
                ProcessedNode {
                    id: 3,
                    x: 24,
                    z: 24,
                    tags: HashMap::new(),
                },
                ProcessedNode {
                    id: 4,
                    x: 10,
                    z: 24,
                    tags: HashMap::new(),
                },
            ],
            tags: tags(&[("military", "bunker")]),
        });
        let first = collect(std::slice::from_ref(&bunker), &roads);
        let second = collect(&[bunker], &roads);
        assert_eq!(first, second);
        assert_eq!(first[0].entrance_facing, "east");
        assert_eq!(first[0].width, 13);
    }

    #[test]
    fn deep_bunker_carves_a_room_and_builds_a_paired_oriented_access() {
        let bbox = XZBBox::rect_from_min_max(0, 0, 40, 40).unwrap();
        let mut ed = editor(&bbox);
        fill_rock(&mut ed, 20, 20);
        let roads = CoordinateBitmap::new(&bbox);
        let feature = BunkerFeature {
            id: 7,
            x: 20,
            z: 20,
            width: 11,
            depth: 11,
            entrance_facing: "east",
        };
        assert!(place(&mut ed, feature, &roads, &bbox));
        let access = (24, 20);
        assert_eq!(
            Some(IRON_TRAPDOOR),
            ed.get_block_absolute(access.0, 1, access.1)
        );
        assert_eq!(Some(LADDER), ed.get_block_absolute(access.0, -4, access.1));
        assert_eq!(Some(AIR), ed.get_block_absolute(20, -2, 20));
        assert_eq!(Some(IRON_BARS), ed.get_block_absolute(17, -1, 17));
        assert_eq!(Some(IRON_BARS), ed.get_block_absolute(17, 0, 17));
        assert_eq!(
            Some(fastnbt::Value::String("east".into())),
            ed.block_properties_absolute(access.0, 1, access.1)
                .and_then(|p| p.get("facing").cloned())
        );
    }

    #[test]
    fn shallow_bunker_builds_a_paired_door_through_the_surface_wall() {
        let bbox = XZBBox::rect_from_min_max(0, 0, 40, 40).unwrap();
        let mut ed = editor(&bbox);
        let surface = min_y() + 4;
        ed.set_ground(Arc::new(crate::ground::Ground::new_flat(surface)));
        for x in 8..=32 {
            for z in 8..=32 {
                for y in min_y()..=surface {
                    ed.set_block_absolute(STONE, x, y, z, None, None);
                }
            }
        }
        let roads = CoordinateBitmap::new(&bbox);
        let feature = BunkerFeature {
            id: 9,
            x: 20,
            z: 20,
            width: 11,
            depth: 11,
            entrance_facing: "east",
        };

        assert!(place(&mut ed, feature, &roads, &bbox));
        assert_eq!(Some(AIR), ed.get_block_absolute(25, surface, 20));
        assert_eq!(Some(IRON_DOOR), ed.get_block_absolute(25, surface + 1, 20));
        assert_eq!(Some(IRON_DOOR), ed.get_block_absolute(25, surface + 2, 20));
        assert_eq!(
            Some(fastnbt::Value::String("lower".into())),
            ed.block_properties_absolute(25, surface + 1, 20)
                .and_then(|p| p.get("half").cloned())
        );
        assert_eq!(
            Some(fastnbt::Value::String("east".into())),
            ed.block_properties_absolute(25, surface + 1, 20)
                .and_then(|p| p.get("facing").cloned())
        );
    }

    #[test]
    fn bunker_rejects_slope_existing_obstructions_and_road_access() {
        let bbox = XZBBox::rect_from_min_max(0, 0, 40, 40).unwrap();
        let mut ed = editor(&bbox);
        fill_rock(&mut ed, 20, 20);
        let roads = CoordinateBitmap::new(&bbox);
        let feature = BunkerFeature {
            id: 8,
            x: 20,
            z: 20,
            width: 11,
            depth: 11,
            entrance_facing: "north",
        };
        ed.set_block_absolute(STONE, 20, 1, 15, None, None);
        assert!(!place(&mut ed, feature, &roads, &bbox));
        let outside = XZBBox::rect_from_min_max(0, 0, 20, 20).unwrap();
        assert!(!place(&mut ed, feature, &roads, &outside));

        let mut road_editor = editor(&bbox);
        fill_rock(&mut road_editor, 20, 20);
        let mut roads = CoordinateBitmap::new(&bbox);
        roads.set(20, 15);
        assert!(!place(&mut road_editor, feature, &roads, &bbox));
    }
}
