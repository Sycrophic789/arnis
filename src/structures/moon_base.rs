//! Deterministic surface habitat for lunar terrain-only worlds.

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
const SITE_SALT: u64 = 0x4D4F_4F4E_4241_5345;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MoonSite {
    pub x: i32,
    pub z: i32,
    pub deck_y: i32,
}

/// Selects one flat-enough, in-bounds site. Terrain samples and candidate order are stable.
pub fn find_site(editor: &WorldEditor, bbox: &XZBBox, body: CelestialBody) -> Option<MoonSite> {
    if body != CelestialBody::Moon
        || bbox.max_x() - bbox.min_x() + 1 < SITE_HALF_X * 2 + 1
        || bbox.max_z() - bbox.min_z() + 1 < SITE_HALF_Z * 2 + 1
    {
        return None;
    }

    let min_x = bbox.min_x() + SITE_HALF_X;
    let max_x = bbox.max_x() - SITE_HALF_X;
    let min_z = bbox.min_z() + SITE_HALF_Z;
    let max_z = bbox.max_z() - SITE_HALF_Z;
    let center = ((min_x + max_x) / 2, (min_z + max_z) / 2);
    let mut candidates = vec![center];
    let mut rng = coord_rng(center.0, center.1, SITE_SALT);
    for _ in 0..CANDIDATE_TRIES {
        candidates.push((
            rng.random_range(min_x..=max_x),
            rng.random_range(min_z..=max_z),
        ));
    }

    candidates.into_iter().find_map(|(x, z)| {
        let mut min_y = i32::MAX;
        let mut max_y = i32::MIN;
        for sx in x - SITE_HALF_X..=x + SITE_HALF_X {
            for sz in z - SITE_HALF_Z..=z + SITE_HALF_Z {
                let y = editor.get_ground_level(sx, sz);
                min_y = min_y.min(y);
                max_y = max_y.max(y);
            }
        }
        (max_y - min_y <= SITE_FLATNESS).then_some(MoonSite {
            x,
            z,
            // The deck bridges a few blocks of lunar relief on short pylons.
            deck_y: max_y + 1,
        })
    })
}

/// Adds one research habitat, with its deck at `site.deck_y`.
pub fn place(editor: &mut WorldEditor, site: MoonSite) {
    let (x, z, deck) = (site.x, site.z, site.deck_y);

    // Reject a site with existing structures in the build volume. Terrain is at or below
    // the deck, so any block above the deck is a real collision rather than ground.
    for bx in x - SITE_HALF_X..=x + SITE_HALF_X {
        for bz in z - SITE_HALF_Z..=z + SITE_HALF_Z {
            if (deck + 1..=deck + 10).any(|y| editor.get_block_absolute(bx, y, bz).is_some()) {
                return;
            }
        }
    }

    // Piled regolith footings level the deck without cutting into crater walls.
    for bx in x - SITE_HALF_X..=x + SITE_HALF_X {
        for bz in z - SITE_HALF_Z..=z + SITE_HALF_Z {
            let ground = editor.get_ground_level(bx, bz);
            for y in ground + 1..deck {
                editor.set_block_absolute(POLISHED_DEEPSLATE, bx, y, bz, None, None);
            }
            editor.set_block_absolute(SMOOTH_STONE, bx, deck, bz, None, None);
        }
    }

    // Central pressurized dome: concentric glass shell rings leave a walkable interior.
    for layer in 1..=6 {
        let radius = 6 - (layer - 1) / 2;
        for dx in -radius..=radius {
            for dz in -radius..=radius {
                let edge = dx.abs() == radius || dz.abs() == radius;
                if edge && dx * dx + dz * dz <= radius * radius + radius {
                    let frame = dx == 0 || dz == 0 || layer == 6;
                    let block = if frame {
                        LIGHT_GRAY_CONCRETE
                    } else {
                        WHITE_STAINED_GLASS
                    };
                    editor.set_block_absolute(block, x + dx, deck + layer, z + dz, None, None);
                }
            }
        }
    }
    editor.set_block_absolute(SEA_LANTERN, x, deck + 2, z, None, None);

    // Two sealed habitat cylinders and glass pressure corridors.
    for side in [-1, 1] {
        let hx = x + side * 10;
        for layer in 1..=4 {
            for dx in -2i32..=2 {
                for dz in -2i32..=2 {
                    if dx.abs() == 2 || dz.abs() == 2 {
                        let block = if dx == 0 && layer == 2 {
                            LIGHT_BLUE_STAINED_GLASS
                        } else {
                            WHITE_CONCRETE
                        };
                        editor.set_block_absolute(block, hx + dx, deck + layer, z + dz, None, None);
                    }
                }
            }
        }
        editor.set_block_absolute(SEA_LANTERN, hx, deck + 2, z, None, None);
        for dx in (5..=8).map(|n| n * side) {
            editor.set_block_absolute(SMOOTH_STONE, x + dx, deck + 1, z, None, None);
            editor.set_block_absolute(WHITE_STAINED_GLASS, x + dx, deck + 2, z, None, None);
            editor.set_block_absolute(WHITE_STAINED_GLASS, x + dx, deck + 3, z, None, None);
        }
    }

    // Airlock to the south with a correctly paired iron pressure door.
    for dx in -1..=1 {
        for dz in 7..=9 {
            if dx.abs() == 1 || dz == 7 || dz == 9 {
                for dy in 1..=3 {
                    editor.set_block_absolute(
                        WHITE_CONCRETE,
                        x + dx,
                        deck + dy,
                        z + dz,
                        None,
                        None,
                    );
                }
            }
        }
    }
    editor.set_block_with_properties_absolute(
        door(IRON_DOOR, "lower", "south"),
        x,
        deck + 1,
        z + 7,
        Some(&[WHITE_CONCRETE]),
        None,
    );
    editor.set_block_with_properties_absolute(
        door(IRON_DOOR, "upper", "south"),
        x,
        deck + 2,
        z + 7,
        Some(&[WHITE_CONCRETE]),
        None,
    );
    editor.set_block_absolute(SEA_LANTERN, x, deck + 2, z + 8, None, None);

    // Solar arrays: dark blue collector panels with a light-gray frame and support posts.
    for side in [-1, 1] {
        let panel_x = x + side * 10;
        let panel_z = z - 8;
        for dx in -3..=3 {
            for dz in -1..=1 {
                let border = dx.abs() == 3 || dz.abs() == 1;
                editor.set_block_absolute(
                    if border {
                        LIGHT_GRAY_CONCRETE
                    } else {
                        BLUE_STAINED_GLASS
                    },
                    panel_x + dx,
                    deck + 2,
                    panel_z + dz,
                    None,
                    None,
                );
            }
        }
        for dx in [-2, 2] {
            editor.set_block_absolute(IRON_BLOCK, panel_x + dx, deck + 1, panel_z, None, None);
        }
    }
}

fn door(block: Block, half: &str, facing: &str) -> BlockWithProperties {
    let mut properties = std::collections::HashMap::new();
    properties.insert("half".to_string(), fastnbt::Value::String(half.to_string()));
    properties.insert(
        "facing".to_string(),
        fastnbt::Value::String(facing.to_string()),
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
    ((site.x - SITE_HALF_X) >> 9..=(site.x + SITE_HALF_X) >> 9).flat_map(move |rx| {
        ((site.z - SITE_HALF_Z) >> 9..=(site.z + SITE_HALF_Z) >> 9).map(move |rz| (rx, rz))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coordinate_system::geographic::LLBBox;
    use std::path::PathBuf;

    fn editor(bbox: &XZBBox) -> WorldEditor<'_> {
        WorldEditor::new(
            PathBuf::from("/dev/null/moon-base-test"),
            bbox,
            LLBBox::new(0.0, 0.0, 0.01, 0.01).unwrap(),
        )
    }

    #[test]
    fn moon_base_is_moon_only_and_requires_space() {
        let bbox = XZBBox::rect_from_min_max(0, 0, 40, 40).unwrap();
        let ed = editor(&bbox);
        assert!(find_site(&ed, &bbox, CelestialBody::Moon).is_some());
        assert!(find_site(&ed, &bbox, CelestialBody::Earth).is_none());
        assert!(find_site(&ed, &bbox, CelestialBody::Mars).is_none());
        let small = XZBBox::rect_from_min_max(0, 0, 20, 20).unwrap();
        assert!(find_site(&ed, &small, CelestialBody::Moon).is_none());
    }

    #[test]
    fn moon_base_selection_is_deterministic_and_places_all_modules() {
        let bbox = XZBBox::rect_from_min_max(0, 0, 40, 40).unwrap();
        let mut ed = editor(&bbox);
        let site = find_site(&ed, &bbox, CelestialBody::Moon).unwrap();
        assert_eq!(Some(site), find_site(&ed, &bbox, CelestialBody::Moon));
        place(&mut ed, site);
        assert_eq!(
            Some(SMOOTH_STONE),
            ed.get_block_absolute(site.x, site.deck_y, site.z)
        );
        assert_eq!(
            Some(WHITE_STAINED_GLASS),
            ed.get_block_absolute(site.x + 5, site.deck_y + 1, site.z + 1)
        );
        assert_eq!(
            Some(BLUE_STAINED_GLASS),
            ed.get_block_absolute(site.x + 10, site.deck_y + 2, site.z - 8)
        );
        assert_eq!(
            Some(IRON_DOOR),
            ed.get_block_absolute(site.x, site.deck_y + 1, site.z + 7)
        );
        assert_eq!(
            Some(IRON_DOOR),
            ed.get_block_absolute(site.x, site.deck_y + 2, site.z + 7)
        );
        assert_eq!(
            Some(fastnbt::Value::String("upper".into())),
            ed.block_properties_absolute(site.x, site.deck_y + 2, site.z + 7)
                .and_then(|p| p.get("half").cloned())
        );
    }

    #[test]
    fn moon_base_rejects_an_obstructed_volume() {
        let bbox = XZBBox::rect_from_min_max(0, 0, 40, 40).unwrap();
        let mut ed = editor(&bbox);
        let site = find_site(&ed, &bbox, CelestialBody::Moon).unwrap();
        ed.set_block_absolute(STONE, site.x, site.deck_y + 4, site.z, None, None);
        place(&mut ed, site);
        assert_eq!(
            Some(STONE),
            ed.get_block_absolute(site.x, site.deck_y + 4, site.z)
        );
        assert_eq!(None, ed.get_block_absolute(site.x, site.deck_y, site.z));
    }
}
