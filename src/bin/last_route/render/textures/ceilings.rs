//! Ceiling painters of ref_textures.gd (32 x 32, sampled like a floor seen from below): coffered ceiling, sewer vault,
//! conservatory glass, cave rock.
//!
//! Flat style: `noise` paints the base only, the coffer beam, the rock chips and the glass panes take one flat tone
//! each, and the rivets (shaded in the painted look) are left out.

use super::canvas::*;
use super::rng::GodotRng;

/// Coffered grid with raised beams on the top and left edges, two rivets; variant 1 adds a dark centre panel.
/// Flat: the beam is a light tone 33 luma above the base, the groove stays dark.
pub fn paint_ceiling(variant: i32) -> Img {
    let mut rng = GodotRng::from_key(&format!("ceil{variant}"));
    let mut img = Img::new(PX, PX);
    let base = if variant == 0 {
        hex(0x100d0f)
    } else {
        hex(0x120a0c)
    };
    noise(&mut img, 0, 0, PX, PX, base, 0.14, &mut rng, 1);
    let beam = flat_or(mul(base, 2.2), hex(0x342c31));
    img.dot(0, 0, beam, PX, 2);
    img.dot(0, 0, beam, 2, PX);
    img.dot(2, 2, hex(0x050304), PX - 2, 1);
    img.dot(2, 2, hex(0x050304), 1, PX - 2);
    for (x, y) in [(8, 8), (22, 22)] {
        rivet(&mut img, x, y, base);
    }
    if variant == 1 {
        img.dot(14, 14, mul(base, 0.6), 6, 6);
    }
    img
}

/// The sewer's barrel vault: damp concrete with one course line. Flat: the base, the course line and the speckles
/// are each one tone; the grain is gone.
pub fn paint_vault(_variant: i32) -> Img {
    let mut rng = GodotRng::from_key("vault");
    let mut img = Img::new(PX, PX);
    let base = hex(0x4a443d);
    noise(&mut img, 0, 0, PX, PX, base, 0.18, &mut rng, 1);
    img.dot(0, 0, mul(base, 1.35), PX, 1);
    for _ in 0..4 {
        img.dot(
            rng.randi_range(1, PX - 3),
            rng.randi_range(3, PX - 3),
            mul(base, 0.55),
            2,
            1,
        );
    }
    img
}

/// Glass-and-iron roof: night panes in a black lattice every 16 px with a bronze edge. Flat: every pane is the one
/// pane tone; the per-pane shade is drawn but not applied.
pub fn paint_glass(_variant: i32) -> Img {
    let mut rng = GodotRng::from_key("glass");
    let mut img = Img::new(PX, PX);
    for y in 0..PX {
        for x in 0..PX {
            let shade = 1.0 + rng.randf_range(-0.15, 0.15);
            img.dot1(x, y, flat_or(mul(hex(0x0c1426), shade), hex(0x0c1426)));
        }
    }
    for _ in 0..5 {
        img.dot1(
            rng.randi_range(2, 29),
            rng.randi_range(2, 29),
            hex(0x2a4a66),
        );
    }
    for k in [0, 16] {
        img.dot(k, 0, hex(0x07070a), 2, PX);
        img.dot(0, k, hex(0x07070a), PX, 2);
        img.dot(k + 2, 0, hex(0x3a2e1c), 1, PX);
        img.dot(0, k + 2, hex(0x3a2e1c), PX, 1);
    }
    img
}

/// Cave rock: dark chips on the rock base. Flat: the light chips are lifted a little so they sit in their own band,
/// 20 luma above the base and chips.
pub fn paint_rock_ceiling(variant: i32) -> Img {
    let mut rng = GodotRng::from_key(&format!("rockceil{variant}"));
    let mut img = Img::new(PX, PX);
    noise(&mut img, 0, 0, PX, PX, hex(0x151012), 0.22, &mut rng, 2);
    for i in 0..14 {
        let c = if i % 2 == 0 {
            hex(0x07040a)
        } else {
            flat_or(hex(0x2a2022), hex(0x2e2428))
        };
        img.dot(
            rng.randi_range(0, 31),
            rng.randi_range(0, 31),
            c,
            rng.randi_range(2, 5),
            rng.randi_range(2, 4),
        );
    }
    img
}

/// Skylight for the ceiling table: the painted beach look, or in the flat style three sky tones (pane, cloud flecks,
/// timber lattice) and a dark lattice shadow. The flecks take the same RNG draws in both styles.
pub fn styled_skylight(variant: i32) -> Img {
    if !flat() {
        return super::beach::paint_skylight(variant);
    }
    let mut rng = GodotRng::from_key("skylight");
    let mut img = Img::new(PX, PX);
    img.fill(hex(0x7a9fb2));
    for _ in 0..7 {
        img.dot(
            rng.randi_range(2, 28),
            rng.randi_range(2, 28),
            hex(0xa8c4cc),
            rng.randi_range(2, 4),
            1,
        );
    }
    let beam = hex(0xa8946e);
    for k in [0, 16] {
        img.dot(k, 0, beam, 2, PX);
        img.dot(0, k, beam, PX, 2);
        img.dot(k + 2, 0, mul(beam, 0.6), 1, PX);
        img.dot(0, k + 2, mul(beam, 0.6), PX, 1);
    }
    img
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_ceiling_coffers_have_3_to_6_colours() {
        check_flat_painter("ceiling 0", paint_ceiling, 0);
        check_flat_painter("ceiling 1", paint_ceiling, 1);
    }

    #[test]
    fn flat_vault_has_3_to_6_colours() {
        check_flat_painter("vault", paint_vault, 0);
    }

    #[test]
    fn flat_glass_panes_have_3_to_6_colours() {
        check_flat_painter("glass", paint_glass, 0);
    }

    #[test]
    fn flat_skylight_has_3_to_6_colours() {
        check_flat_painter("skylight", styled_skylight, 0);
    }

    #[test]
    fn painted_skylight_is_the_beach_painter() {
        let painted = with_style(false, || styled_skylight(0));
        assert_eq!(painted.px, super::super::beach::paint_skylight(0).px);
    }

    #[test]
    fn flat_rock_ceiling_has_3_to_6_colours() {
        check_flat_painter("rockceil", paint_rock_ceiling, 0);
    }
}
