//! Deterministic ATM selection and placement for mapped ATM points and eligible venues.

use crate::block_definitions::*;
use crate::coordinate_system::cartesian::XZBBox;
use crate::element_processing::get_nearest_road_block;
use crate::floodfill_cache::RoadMaskBitmap;
use crate::land_cover::coord_hash;
use crate::osm_parser::{ProcessedElement, ProcessedNode};
use crate::world_editor::WorldEditor;
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AtmVenue {
    Bank,
    Supermarket,
    Commercial,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AtmSpot {
    pub x: i32,
    pub z: i32,
    pub facing: &'static str,
}

#[derive(Clone, Debug)]
pub struct AtmFeature {
    pub id: u64,
    pub venue: AtmVenue,
    pub tags: HashMap<String, String>,
    pub spots: Vec<AtmSpot>,
    pub explicit: bool,
}

/// Returns a venue style only for mapped ATMs or building types commonly equipped with one.
pub fn venue_for_tags(tags: &HashMap<String, String>) -> Option<(AtmVenue, bool)> {
    let amenity = tags.get("amenity").map(String::as_str);
    let shop = tags.get("shop").map(String::as_str);
    let explicit = amenity == Some("atm") || tags.get("atm").map(String::as_str) == Some("yes");

    if amenity == Some("atm") {
        return Some((AtmVenue::Commercial, true));
    }
    if amenity == Some("bank") {
        return Some((AtmVenue::Bank, explicit));
    }
    if shop == Some("supermarket") {
        return Some((AtmVenue::Supermarket, explicit));
    }
    if explicit
        && (tags.contains_key("building")
            || tags.contains_key("building:part")
            || tags.contains_key("shop"))
    {
        return Some((AtmVenue::Commercial, true));
    }
    None
}

/// Collects explicit ATM features and reasonable bank/supermarket placements from OSM data.
/// Other commercial buildings only qualify with `atm=yes`.
pub fn collect(
    elements: &[ProcessedElement],
    road_mask: &RoadMaskBitmap,
    entrances: &[(i32, i32)],
) -> Vec<AtmFeature> {
    let mut features: Vec<AtmFeature> = elements
        .iter()
        .filter_map(|element| feature_for_element(element, road_mask, entrances))
        .collect();
    features.sort_by_key(|feature| (!feature.explicit, feature.id));

    let mut accepted: Vec<AtmFeature> = Vec::new();
    for feature in features {
        let duplicates = accepted.iter().any(|other| {
            feature.spots.iter().any(|a| {
                other
                    .spots
                    .iter()
                    .any(|b| (a.x - b.x).abs() <= 3 && (a.z - b.z).abs() <= 3)
            })
        });
        if !duplicates {
            accepted.push(feature);
        }
    }
    accepted
}

fn feature_for_element(
    element: &ProcessedElement,
    road_mask: &RoadMaskBitmap,
    entrances: &[(i32, i32)],
) -> Option<AtmFeature> {
    let (venue, explicit) = venue_for_tags(element.tags())?;
    let nodes: Vec<_> = element.nodes().collect();
    if nodes.is_empty() {
        return None;
    }
    let spots = if element.tags().get("amenity").map(String::as_str) == Some("atm")
        || matches!(element, ProcessedElement::Node(_))
    {
        point_spots(nodes[0].x, nodes[0].z, element.id(), road_mask, entrances)
    } else {
        building_spots(&nodes, element.id(), road_mask, entrances)
    };
    (!spots.is_empty()).then(|| AtmFeature {
        id: element.id(),
        venue,
        tags: element.tags().clone(),
        spots,
        explicit,
    })
}

fn point_spots(
    x: i32,
    z: i32,
    id: u64,
    road_mask: &RoadMaskBitmap,
    entrances: &[(i32, i32)],
) -> Vec<AtmSpot> {
    let mut candidates = vec![(x, z)];
    for radius in 1..=3 {
        candidates.extend([
            (x, z - radius),
            (x + radius, z),
            (x, z + radius),
            (x - radius, z),
        ]);
    }
    candidates
        .into_iter()
        .filter(|&(x, z)| {
            !road_mask.contains(x, z)
                && !entrances
                    .iter()
                    .any(|&(ex, ez)| (ex - x).abs() <= 2 && (ez - z).abs() <= 2)
        })
        .map(|(x, z)| {
            spot_toward_road(x, z, road_mask).unwrap_or_else(|| {
                let facing = match coord_hash(x, z) ^ id {
                    n if n & 3 == 0 => "north",
                    n if n & 3 == 1 => "east",
                    n if n & 3 == 2 => "south",
                    _ => "west",
                };
                AtmSpot { x, z, facing }
            })
        })
        .collect()
}

fn building_spots(
    nodes: &[&ProcessedNode],
    id: u64,
    road_mask: &RoadMaskBitmap,
    entrances: &[(i32, i32)],
) -> Vec<AtmSpot> {
    if nodes.len() < 3 {
        return Vec::new();
    }
    let mut candidates = Vec::new();
    let mut push = |x: i32, z: i32, outward: &'static str| {
        if road_mask.contains(x, z)
            || entrances
                .iter()
                .any(|&(ex, ez)| (ex - x).abs() <= 2 && (ez - z).abs() <= 2)
        {
            return;
        }
        let road = get_nearest_road_block(x, z, 16, road_mask);
        let facing = road
            .map(|(rx, rz)| facing_toward(rx - x, rz - z))
            .unwrap_or(outward);
        let road_distance = road
            .map(|(rx, rz)| (rx - x).abs() + (rz - z).abs())
            .unwrap_or(99);
        candidates.push((
            road_distance,
            coord_hash(x, z) ^ id,
            AtmSpot { x, z, facing },
        ));
    };

    let ring = if nodes.first().map(|n| (n.x, n.z)) == nodes.last().map(|n| (n.x, n.z)) {
        &nodes[..nodes.len() - 1]
    } else {
        nodes
    };
    if ring.len() < 3 {
        return Vec::new();
    }
    let area = (0..ring.len())
        .map(|i| {
            let a = ring[i];
            let b = ring[(i + 1) % ring.len()];
            i64::from(a.x) * i64::from(b.z) - i64::from(b.x) * i64::from(a.z)
        })
        .sum::<i64>();
    if area == 0 {
        return Vec::new();
    }
    let ccw = area > 0;

    // Each candidate sits just outside a real OSM wall segment. Sample three
    // positions along longer facades; the road score chooses the accessible one.
    for i in 0..ring.len() {
        let a = ring[i];
        let b = ring[(i + 1) % ring.len()];
        let dx = b.x - a.x;
        let dz = b.z - a.z;
        if dx == 0 && dz == 0 {
            continue;
        }
        let (nx, nz) = if ccw { (dz, -dx) } else { (-dz, dx) };
        let outward = if nx.abs() >= nz.abs() {
            if nx < 0 {
                "west"
            } else {
                "east"
            }
        } else if nz < 0 {
            "north"
        } else {
            "south"
        };
        let tangent_len = dx.abs().max(dz.abs());
        for numerator in [1, 2, 3] {
            let wall_x = a.x + dx * numerator / 4;
            let wall_z = a.z + dz * numerator / 4;
            let (x, z) = match outward {
                "west" => (wall_x - 1, wall_z),
                "east" => (wall_x + 1, wall_z),
                "north" => (wall_x, wall_z - 1),
                _ => (wall_x, wall_z + 1),
            };
            // Skip degenerate segments; entrance locations are filtered in `push`.
            if tangent_len < 4 {
                continue;
            }
            push(x, z, outward);
        }
    }
    candidates.sort_by_key(|(distance, tie, _)| (*distance, *tie));
    candidates.into_iter().map(|(_, _, spot)| spot).collect()
}

fn spot_toward_road(x: i32, z: i32, road_mask: &RoadMaskBitmap) -> Option<AtmSpot> {
    let (rx, rz) = get_nearest_road_block(x, z, 16, road_mask)?;
    Some(AtmSpot {
        x,
        z,
        facing: facing_toward(rx - x, rz - z),
    })
}

fn facing_toward(dx: i32, dz: i32) -> &'static str {
    if dx.abs() >= dz.abs() {
        if dx < 0 {
            "west"
        } else {
            "east"
        }
    } else if dz < 0 {
        "north"
    } else {
        "south"
    }
}

impl AtmFeature {
    pub fn deferred_region_keys(&self) -> impl Iterator<Item = (i32, i32)> + '_ {
        let min_x = self.spots.iter().map(|s| s.x).min().unwrap_or(0) - 2;
        let max_x = self.spots.iter().map(|s| s.x).max().unwrap_or(0) + 2;
        let min_z = self.spots.iter().map(|s| s.z).min().unwrap_or(0) - 2;
        let max_z = self.spots.iter().map(|s| s.z).max().unwrap_or(0) + 2;
        (min_x >> 9..=max_x >> 9)
            .flat_map(move |rx| (min_z >> 9..=max_z >> 9).map(move |rz| (rx, rz)))
    }
}

/// Places one compact machine on clear, supported ground, preferring a street-facing spot.
pub fn place_all(
    editor: &mut WorldEditor,
    features: &[AtmFeature],
    road_mask: &RoadMaskBitmap,
    bbox: &XZBBox,
) {
    for feature in features {
        for spot in &feature.spots {
            if !bbox.contains(&crate::coordinate_system::cartesian::XZPoint::new(
                spot.x, spot.z,
            )) || road_mask.contains(spot.x, spot.z)
            {
                continue;
            }
            let ground_y = editor.get_ground_level(spot.x, spot.z);
            let support = editor.get_block_absolute(spot.x, ground_y, spot.z);
            let clear = editor
                .get_block_absolute(spot.x, ground_y + 1, spot.z)
                .is_none()
                && editor
                    .get_block_absolute(spot.x, ground_y + 2, spot.z)
                    .is_none();
            if support.is_none() || support == Some(WATER) || !clear {
                continue;
            }

            let (base, screen) = match feature.venue {
                AtmVenue::Bank => (IRON_BLOCK, BLACK_CONCRETE),
                AtmVenue::Supermarket => (LIGHT_GRAY_CONCRETE, BLACK_STAINED_GLASS),
                AtmVenue::Commercial => (GRAY_CONCRETE, BLACK_STAINED_GLASS),
            };
            editor.set_block_absolute(base, spot.x, ground_y + 1, spot.z, None, None);
            editor.set_block_absolute(screen, spot.x, ground_y + 2, spot.z, None, None);

            if let Some(ctx) = editor.signage() {
                if let Some(key) =
                    crate::element_processing::signage::furniture_pictogram(&feature.tags)
                {
                    if ctx.registry.contains(&key) {
                        let face = match spot.facing {
                            "north" => 2,
                            "south" => 3,
                            "west" => 4,
                            _ => 5,
                        };
                        editor.place_decal(spot.x, ground_y + 2, spot.z, face, &key);
                    }
                }
            }
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coordinate_system::cartesian::XZBBox;
    use crate::coordinate_system::geographic::LLBBox;
    use crate::floodfill_cache::CoordinateBitmap;
    use crate::osm_parser::{ProcessedNode, ProcessedWay};
    use std::path::PathBuf;

    fn tags(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn atm_selection_is_limited_to_mapped_or_likely_venues() {
        assert_eq!(
            venue_for_tags(&tags(&[("amenity", "atm")])),
            Some((AtmVenue::Commercial, true))
        );
        assert_eq!(
            venue_for_tags(&tags(&[("amenity", "bank")])),
            Some((AtmVenue::Bank, false))
        );
        assert_eq!(
            venue_for_tags(&tags(&[("shop", "supermarket")])),
            Some((AtmVenue::Supermarket, false))
        );
        assert_eq!(venue_for_tags(&tags(&[("shop", "clothes")])), None);
        assert_eq!(venue_for_tags(&tags(&[("building", "commercial")])), None);
        assert_eq!(
            venue_for_tags(&tags(&[("building", "commercial"), ("atm", "yes")])),
            Some((AtmVenue::Commercial, true))
        );
    }

    #[test]
    fn mapped_atm_spots_face_a_nearby_road_and_avoid_the_road_cell() {
        let bbox = XZBBox::rect_from_min_max(-10, -10, 10, 10).unwrap();
        let mut roads = CoordinateBitmap::new(&bbox);
        roads.set(4, 0);
        let spots = point_spots(0, 0, 12, &roads, &[]);
        assert!(spots.iter().all(|spot| (spot.x, spot.z) != (4, 0)));
        assert_eq!(spots[0].facing, "east");
        let entrance_avoiding = point_spots(0, 0, 12, &roads, &[(0, 0)]);
        assert!(entrance_avoiding
            .iter()
            .all(|spot| (spot.x, spot.z) != (0, 0)));
    }

    #[test]
    fn building_candidates_keep_entrances_clear() {
        let bbox = XZBBox::rect_from_min_max(0, 0, 40, 40).unwrap();
        let roads = CoordinateBitmap::new(&bbox);
        let nodes = vec![
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
                z: 20,
                tags: HashMap::new(),
            },
            ProcessedNode {
                id: 4,
                x: 10,
                z: 20,
                tags: HashMap::new(),
            },
            ProcessedNode {
                id: 1,
                x: 10,
                z: 10,
                tags: HashMap::new(),
            },
        ];
        let node_refs: Vec<_> = nodes.iter().collect();
        let spots = building_spots(&node_refs, 9, &roads, &[(9, 15)]);
        assert!(!spots.iter().any(|spot| spot.x == 9 && spot.z == 15));
        assert!(spots
            .iter()
            .all(|spot| { spot.x < 10 || spot.x > 24 || spot.z < 10 || spot.z > 20 }));
    }

    #[test]
    fn placement_uses_venue_blocks_and_skips_obstructed_spots() {
        let bbox = XZBBox::rect_from_min_max(0, 0, 20, 20).unwrap();
        let ll = LLBBox::new(0.0, 0.0, 0.01, 0.01).unwrap();
        let mut editor = WorldEditor::new(PathBuf::from("/dev/null/atm-test"), &bbox, ll);
        let roads = CoordinateBitmap::new(&bbox);
        let feature = AtmFeature {
            id: 1,
            venue: AtmVenue::Bank,
            tags: tags(&[("amenity", "atm")]),
            spots: vec![AtmSpot {
                x: 5,
                z: 5,
                facing: "east",
            }],
            explicit: true,
        };
        editor.set_block_absolute(STONE, 5, 0, 5, None, None);
        editor.set_block_absolute(STONE, 5, 2, 5, None, None);
        place_all(&mut editor, std::slice::from_ref(&feature), &roads, &bbox);
        assert_eq!(None, editor.get_block_absolute(5, 1, 5));

        let mut clear_editor =
            WorldEditor::new(PathBuf::from("/dev/null/atm-clear-test"), &bbox, ll);
        clear_editor.set_block_absolute(STONE, 5, 0, 5, None, None);
        place_all(
            &mut clear_editor,
            std::slice::from_ref(&feature),
            &roads,
            &bbox,
        );
        assert_eq!(Some(IRON_BLOCK), clear_editor.get_block_absolute(5, 1, 5));
        assert_eq!(
            Some(BLACK_CONCRETE),
            clear_editor.get_block_absolute(5, 2, 5)
        );
    }

    #[test]
    fn collecting_explicit_and_venue_candidates_is_deterministic() {
        let bbox = XZBBox::rect_from_min_max(0, 0, 30, 30).unwrap();
        let roads = CoordinateBitmap::new(&bbox);
        let point = ProcessedElement::Node(ProcessedNode {
            id: 1,
            x: 2,
            z: 2,
            tags: tags(&[("amenity", "atm")]),
        });
        let supermarket = ProcessedElement::Way(ProcessedWay {
            id: 2,
            nodes: vec![
                ProcessedNode {
                    id: 20,
                    x: 10,
                    z: 10,
                    tags: HashMap::new(),
                },
                ProcessedNode {
                    id: 21,
                    x: 20,
                    z: 10,
                    tags: HashMap::new(),
                },
                ProcessedNode {
                    id: 22,
                    x: 20,
                    z: 20,
                    tags: HashMap::new(),
                },
                ProcessedNode {
                    id: 23,
                    x: 10,
                    z: 20,
                    tags: HashMap::new(),
                },
            ],
            tags: tags(&[("shop", "supermarket")]),
        });
        let elements = vec![point, supermarket];
        let first = collect(&elements, &roads, &[]);
        let second = collect(&elements, &roads, &[]);
        assert_eq!(
            first.iter().map(|f| (f.id, f.venue)).collect::<Vec<_>>(),
            second.iter().map(|f| (f.id, f.venue)).collect::<Vec<_>>()
        );
        assert_eq!(first.len(), 2);
    }
}
