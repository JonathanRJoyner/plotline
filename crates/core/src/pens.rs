//! A bank of common drawing pens with the line widths their makers publish, so a pen can be
//! picked by name instead of typed in. Every entry is black; a front end can recolour it, and
//! [`INKS`] lists common ink colours to offer when it does.
//!
//! Widths come from the makers' product pages and size charts as read on 2026-10-12. Each is the
//! width of the drawn line, which for gel pens and ballpoints is narrower than the ball size the
//! pen is sold under; a pen whose maker gives only a ball size or a range is left out, and so is
//! a brush pen, whose width follows the pressure. Several makers call their figure approximate.
//! Some makers name a pen by a code rather than its width: Sakura's "005" draws about 0.20 mm,
//! and edding's 1800 "0.1" draws 0.25 mm. The comment above a model says where its widths are.

use serde::Serialize;

use crate::types::Pen;

/// Common ink colours as (plain name, `#rrggbb`), in the order a front end lists them. The values
/// are screen colours chosen for this project to look like each ink on white paper, not any
/// maker's figures; a pen whose exact colour matters should be given its own.
pub const INKS: &[(&str, &str)] = &[
    ("black", "#000000"),
    ("grey", "#808080"),
    ("brown", "#7b4a2a"),
    ("red", "#d0202e"),
    ("orange", "#f07c1a"),
    ("yellow", "#f5c400"),
    ("green", "#1e8a3c"),
    ("blue", "#1d4fb8"),
    ("purple", "#6a2c91"),
    ("pink", "#e8478b"),
];

/// A pen from the bank, with the kind and model a front end groups it by.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BankPen {
    /// One of the kinds, such as "fineliner" or "gel pen".
    pub kind: &'static str,
    /// Maker and model, the start of the pen's name.
    pub model: &'static str,
    /// The pen itself, black, named by model and size.
    #[serde(flatten)]
    pub pen: Pen,
}

/// Maker and model, then the (size name, line width in mm) pairs sold under it.
type Model = (&'static str, &'static [(&'static str, f64)]);

/// Each kind of pen with its models, in catalogue order. Laid out by hand as a table.
#[rustfmt::skip]
const BANK: &[(&str, &[Model])] = &[
    (
        "fineliner",
        &[
            ("Sakura Pigma Micron", &[
                ("003", 0.15), ("005", 0.2), ("01", 0.25), ("02", 0.3), ("03", 0.35), ("04", 0.4),
                ("05", 0.45), ("08", 0.5),
            ]),
            ("Staedtler pigment liner", &[
                ("0.05", 0.05), ("0.1", 0.1), ("0.2", 0.2), ("0.3", 0.3), ("0.5", 0.5),
                ("0.6", 0.6), ("0.7", 0.7), ("0.8", 0.8), ("1.0", 1.0), ("1.2", 1.2),
            ]),
            ("Uni Pin", &[
                ("0.03", 0.03), ("0.05", 0.05), ("0.1", 0.1), ("0.2", 0.2), ("0.3", 0.3),
                ("0.4", 0.4), ("0.5", 0.5), ("0.6", 0.6), ("0.7", 0.7), ("0.8", 0.8),
            ]),
            ("Copic Multiliner", &[
                ("0.03", 0.03), ("0.05", 0.05), ("0.1", 0.1), ("0.3", 0.3), ("0.5", 0.5),
                ("0.8", 0.8), ("1.0", 1.0),
            ]),
            ("Faber-Castell Pitt Artist Pen", &[("XS", 0.1), ("S", 0.3), ("F", 0.5), ("M", 0.7)]),
            // faber-castell.com: nine line widths in black
            ("Faber-Castell Ecco Pigment", &[
                ("0.05", 0.05), ("0.1", 0.1), ("0.2", 0.2), ("0.3", 0.3), ("0.4", 0.4),
                ("0.5", 0.5), ("0.6", 0.6), ("0.7", 0.7), ("0.8", 0.8),
            ]),
            // staedtler.com: line width approx. 0.3 mm
            ("Staedtler triplus fineliner", &[("0.3", 0.3)]),
            // stabilo.com: line width 0.4 mm
            ("Stabilo point 88", &[("0.4", 0.4)]),
            // rotring.com: sold in line widths named by the size
            ("Rotring Tikky Graphic", &[
                ("0.1", 0.1), ("0.2", 0.2), ("0.3", 0.3), ("0.4", 0.4), ("0.5", 0.5), ("0.7", 0.7),
            ]),
            // pentel.co.jp and pentel.de: line widths named by the size
            ("Pentel Pointliner", &[
                ("0.03", 0.03), ("0.05", 0.05), ("0.1", 0.1), ("0.2", 0.2), ("0.3", 0.3),
                ("0.4", 0.4), ("0.5", 0.5), ("0.8", 0.8), ("1.0", 1.0),
            ]),
            // shop.molotow.com: stroke widths named by the size
            ("Molotow Blackliner", &[
                ("0.05", 0.05), ("0.1", 0.1), ("0.2", 0.2), ("0.3", 0.3), ("0.4", 0.4),
                ("0.5", 0.5), ("0.7", 0.7), ("0.9", 0.9), ("1.0", 1.0),
            ]),
            // edding.com: "0.1 = 0.25 mm, 0.3 = 0.35 mm, 0.5 = 0.5 mm and 0.7 = 0.7 mm"
            ("edding 1800 profipen", &[("0.1", 0.25), ("0.3", 0.35), ("0.5", 0.5), ("0.7", 0.7)]),
            // edding.com: stroke width 0.3 mm
            ("edding 55", &[("0.3", 0.3)]),
        ],
    ),
    (
        "technical pen",
        &[
            // rotring.com: sold in ISO line widths named by the size
            ("Rotring Isograph", &[
                ("0.1", 0.1), ("0.2", 0.2), ("0.25", 0.25), ("0.3", 0.3), ("0.35", 0.35),
                ("0.4", 0.4), ("0.5", 0.5), ("0.6", 0.6), ("0.8", 0.8),
            ]),
            // kohinoorusa.com point size chart: each nib size and its line width
            ("Koh-I-Noor Rapidograph", &[
                ("6x0", 0.13), ("4x0", 0.18), ("3x0", 0.25), ("00", 0.3), ("0", 0.35), ("1", 0.5),
                ("2", 0.6), ("2.5", 0.7), ("3", 0.8), ("4", 1.2),
            ]),
        ],
    ),
    (
        "gel pen",
        &[
            // Pilot's Japanese web catalogue: line width for each ball size
            ("Pilot Juice", &[("0.38", 0.26), ("0.5", 0.31), ("0.7", 0.34), ("1.0", 0.37)]),
            // Pilot's Japanese web catalogue: line width for each ball size
            ("Pilot Juice up", &[("0.3", 0.15), ("0.4", 0.2), ("0.5", 0.25)]),
            // Pilot's Japanese web catalogue: line width for each ball size
            ("Pilot Hi-Tec-C", &[("0.3", 0.15), ("0.4", 0.2), ("0.5", 0.25)]),
            // pentel.co.uk: approximate line width for each tip size
            ("Pentel EnerGel", &[("0.5", 0.25), ("0.7", 0.35), ("1.0", 0.5)]),
            // sakuraofamerica.com: Fine 0.3 mm line, Medium 0.4 mm line
            ("Sakura Gelly Roll Classic", &[("Fine", 0.3), ("Medium", 0.4)]),
        ],
    ),
    (
        "ballpoint",
        &[
            // pentel.co.uk: approximate line width for each tip size
            ("Pentel Superb", &[("0.7", 0.25), ("1.0", 0.5)]),
        ],
    ),
    (
        "marker",
        &[
            // staedtler.com: approximate line widths; B is a range, so left out
            ("Staedtler Lumocolor permanent", &[("S", 0.4), ("F", 0.6), ("M", 1.0)]),
            // stabilo.com: M tip, line width 1 mm
            ("Stabilo Pen 68", &[("M", 1.0)]),
            // edding.com: stroke width 1 mm
            ("edding 400", &[("1", 1.0)]),
            // edding.com: stroke width 0.75 mm
            ("edding 404", &[("0.75", 0.75)]),
        ],
    ),
];

/// The bank in catalogue order, every pen black.
pub fn bank() -> Vec<BankPen> {
    BANK.iter()
        .flat_map(|(kind, models)| models.iter().map(move |model| (*kind, model)))
        .flat_map(|(kind, (model, sizes))| {
            sizes.iter().map(move |(size, width)| BankPen {
                kind,
                model,
                pen: Pen::new(&format!("{model} {size}"), *width, "#000000")
                    .expect("the bank holds valid pens"),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn bank_has_unique_valid_pens() {
        let pens = bank();
        assert!(pens.len() >= 30);
        let names: HashSet<&str> = pens.iter().map(|p| p.pen.name.as_str()).collect();
        assert_eq!(names.len(), pens.len());
        for p in &pens {
            Pen::new(&p.pen.name, p.pen.width_mm, &p.pen.color).unwrap();
            assert!(p.pen.name.starts_with(p.model));
        }
    }
    #[test]
    fn inks_are_unique_valid_colours() {
        let names: HashSet<&str> = INKS.iter().map(|(name, _)| *name).collect();
        assert_eq!(names.len(), INKS.len());
        assert_eq!(INKS[0], ("black", "#000000"));
        for (name, color) in INKS {
            Pen::new(name, 0.3, color).unwrap();
        }
    }
    #[test]
    fn kinds_come_in_catalogue_order() {
        let mut kinds: Vec<&str> = bank().iter().map(|p| p.kind).collect();
        kinds.dedup();
        assert_eq!(
            kinds,
            [
                "fineliner",
                "technical pen",
                "gel pen",
                "ballpoint",
                "marker"
            ]
        );
    }
    #[test]
    fn bank_pens_list_flat() {
        let p = &bank()[0];
        let v = serde_json::to_value(p).unwrap();
        assert_eq!(v["kind"], "fineliner");
        assert_eq!(v["model"], "Sakura Pigma Micron");
        assert_eq!(v["name"], p.pen.name);
        assert_eq!(v["width_mm"], p.pen.width_mm);
    }
    #[test]
    fn codes_map_to_published_widths() {
        let width = |name: &str| {
            bank()
                .into_iter()
                .find(|p| p.pen.name == name)
                .map(|p| p.pen.width_mm)
        };
        assert_eq!(width("Sakura Pigma Micron 005"), Some(0.2));
        assert_eq!(width("Sakura Pigma Micron 08"), Some(0.5));
        assert_eq!(width("edding 1800 profipen 0.1"), Some(0.25));
        assert_eq!(width("Koh-I-Noor Rapidograph 6x0"), Some(0.13));
        assert_eq!(width("Pilot Juice 0.5"), Some(0.31));
        assert_eq!(width("Sakura Gelly Roll Classic Fine"), Some(0.3));
        assert_eq!(width("Faber-Castell Pitt Artist Pen S"), Some(0.3));
        assert_eq!(width("Staedtler pigment liner 0.05"), Some(0.05));
    }
}
