//! A bank of common drawing pens with the line widths their makers publish, so a pen can be
//! picked by name instead of typed in. Every entry is black; a front end can recolour it.
//!
//! Widths come from the makers' product pages and size charts as read on 2026-10-12. Some makers
//! name a pen by a code rather than its width: Sakura's "005" draws about 0.20 mm.

use crate::types::Pen;

/// Maker and model, then the (size name, width in mm) pairs sold under it.
const BANK: &[(&str, &[(&str, f64)])] = &[
    (
        "Sakura Pigma Micron",
        &[
            ("003", 0.15),
            ("005", 0.2),
            ("01", 0.25),
            ("02", 0.3),
            ("03", 0.35),
            ("04", 0.4),
            ("05", 0.45),
            ("08", 0.5),
        ],
    ),
    (
        "Staedtler pigment liner",
        &[
            ("0.05", 0.05),
            ("0.1", 0.1),
            ("0.2", 0.2),
            ("0.3", 0.3),
            ("0.5", 0.5),
            ("0.6", 0.6),
            ("0.7", 0.7),
            ("0.8", 0.8),
            ("1.0", 1.0),
            ("1.2", 1.2),
        ],
    ),
    (
        "Uni Pin",
        &[
            ("0.03", 0.03),
            ("0.05", 0.05),
            ("0.1", 0.1),
            ("0.2", 0.2),
            ("0.3", 0.3),
            ("0.4", 0.4),
            ("0.5", 0.5),
            ("0.6", 0.6),
            ("0.7", 0.7),
            ("0.8", 0.8),
        ],
    ),
    (
        "Copic Multiliner",
        &[
            ("0.03", 0.03),
            ("0.05", 0.05),
            ("0.1", 0.1),
            ("0.3", 0.3),
            ("0.5", 0.5),
            ("0.8", 0.8),
            ("1.0", 1.0),
        ],
    ),
    (
        "Faber-Castell Pitt Artist Pen",
        &[("XS", 0.1), ("S", 0.3), ("F", 0.5), ("M", 0.7)],
    ),
];

/// The bank in catalogue order, every pen black.
pub fn bank() -> Vec<Pen> {
    BANK.iter()
        .flat_map(|(model, sizes)| {
            sizes.iter().map(move |(size, width)| {
                Pen::new(&format!("{model} {size}"), *width, "#000000")
                    .expect("the bank holds valid pens")
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
        let names: HashSet<&str> = pens.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names.len(), pens.len());
        for p in &pens {
            Pen::new(&p.name, p.width_mm, &p.color).unwrap();
        }
    }
    #[test]
    fn codes_map_to_published_widths() {
        let width = |name: &str| bank().iter().find(|p| p.name == name).map(|p| p.width_mm);
        assert_eq!(width("Sakura Pigma Micron 005"), Some(0.2));
        assert_eq!(width("Sakura Pigma Micron 08"), Some(0.5));
        assert_eq!(width("Faber-Castell Pitt Artist Pen S"), Some(0.3));
        assert_eq!(width("Staedtler pigment liner 0.05"), Some(0.05));
    }
}
