//! A machine: where its pen may go, how the pen is lifted and how fast it moves, for the G-code
//! adapter in [`crate::export::gcode`]. Coordinates are the machine's own millimetres with y up,
//! as printers count it. The rest of the core stays machine-agnostic: only the exporter, the
//! preset and the session's speeds read this.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::stats::Speeds;

/// A machine value that cannot be used; the message names the field.
#[derive(Debug, Clone, PartialEq, Error)]
#[error("{0}")]
pub struct MachineError(pub String);

/// Where the pen may be below the safe height, in machine mm.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Area {
    pub x_min: f64,
    pub x_max: f64,
    pub y_min: f64,
    pub y_max: f64,
}

impl Area {
    /// The area's width in mm.
    pub fn width(&self) -> f64 {
        self.x_max - self.x_min
    }
    /// The area's height in mm.
    pub fn height(&self) -> f64 {
        self.y_max - self.y_min
    }
    /// Whether `p` lies inside the area, with `eps` of slack for rounding.
    pub fn contains(&self, [x, y]: [f64; 2], eps: f64) -> bool {
        self.x_min - eps <= x
            && x <= self.x_max + eps
            && self.y_min - eps <= y
            && y <= self.y_max + eps
    }
}

/// A machine that draws with a pen: its bed, drawable area, pen heights, feeds and the G-code
/// written around a plot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Machine {
    /// The machine's name, shown to users.
    pub name: String,
    /// The bed's width and height in mm; no move goes beyond it.
    pub bed_mm: [f64; 2],
    /// Where the pen may be below `z_safe_mm`.
    pub area: Area,
    /// The Z the pen draws at.
    pub z_down_mm: f64,
    /// The Z the pen travels at between paths.
    pub z_up_mm: f64,
    /// The Z every file lifts to before its first move and before its end lines.
    pub z_safe_mm: f64,
    /// Feed with the pen down, in mm per minute.
    pub draw_feed: f64,
    /// Feed with the pen up, in mm per minute.
    pub travel_feed: f64,
    /// Feed of Z moves, in mm per minute.
    pub z_feed: f64,
    /// The dwell after each Z move, in ms.
    pub settle_ms: u32,
    /// Lines written after `G21` and `G90`, as they are.
    pub start_gcode: String,
    /// Lines written after the final lift, as they are.
    pub end_gcode: String,
}

impl Machine {
    /// Check every field; the message names the first one that is wrong.
    pub fn validate(&self) -> Result<(), MachineError> {
        let bad = |what: String| Err(MachineError(what));
        if self.name.trim().is_empty() {
            return bad("name must not be empty".to_string());
        }
        let [bw, bh] = self.bed_mm;
        if !(bw > 0.0 && bh > 0.0) {
            return bad(format!("bed_mm must be positive, got {:?}", self.bed_mm));
        }
        let a = &self.area;
        let inside = 0.0 <= a.x_min && a.x_min < a.x_max && a.x_max <= bw;
        let inside = inside && 0.0 <= a.y_min && a.y_min < a.y_max && a.y_max <= bh;
        if !inside {
            return bad(format!(
                "area must lie inside the {bw} by {bh} mm bed with min below max, got x {}..{} y {}..{}",
                a.x_min, a.x_max, a.y_min, a.y_max
            ));
        }
        if !(0.0 <= self.z_down_mm && self.z_down_mm < self.z_up_mm) {
            return bad(format!(
                "z_up_mm must be above z_down_mm ({}), got {}",
                self.z_down_mm, self.z_up_mm
            ));
        }
        let safe_above_up = self.z_up_mm < self.z_safe_mm;
        if !safe_above_up {
            return bad(format!(
                "z_safe_mm must be above z_up_mm ({}), got {}",
                self.z_up_mm, self.z_safe_mm
            ));
        }
        let feeds = [
            ("draw_feed", self.draw_feed),
            ("travel_feed", self.travel_feed),
            ("z_feed", self.z_feed),
        ];
        for (name, feed) in feeds {
            let positive = feed > 0.0;
            if !positive {
                return bad(format!("{name} must be positive, got {feed}"));
            }
        }
        Ok(())
    }

    /// The plot speeds the machine implies: the feeds in mm per second, and a pen lift as the
    /// down and up Z moves at `z_feed` plus two dwells.
    pub fn speeds(&self) -> Speeds {
        Speeds {
            draw_mm_s: self.draw_feed / 60.0,
            travel_mm_s: self.travel_feed / 60.0,
            pen_lift_s: 2.0 * (self.z_up_mm - self.z_down_mm) / (self.z_feed / 60.0)
                + 2.0 * f64::from(self.settle_ms) / 1000.0,
        }
    }

    /// Where a page of `width_mm` by `height_mm` lands: centred in the area. Fails when the page
    /// is wider or taller than the area, naming both sizes.
    pub fn mapping(&self, width_mm: f64, height_mm: f64) -> Result<Mapping, MachineError> {
        let (w, h) = (self.area.width(), self.area.height());
        if width_mm > w + 1e-6 || height_mm > h + 1e-6 {
            return Err(MachineError(format!(
                "the {width_mm} by {height_mm} mm page is larger than the machine's {w} by {h} mm drawable area"
            )));
        }
        Ok(Mapping {
            ox: self.area.x_min + (w - width_mm) / 2.0,
            oy: self.area.y_max - (h - height_mm) / 2.0,
        })
    }
}

/// Page millimetres (origin at the top-left corner, y down) to machine millimetres (y up).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mapping {
    ox: f64,
    oy: f64,
}

impl Mapping {
    /// The machine point under page point `p`.
    pub fn to_machine(&self, [x, y]: [f64; 2]) -> [f64; 2] {
        [self.ox + x, self.oy - y]
    }
}

#[cfg(test)]
mod tests {
    use crate::test_util::machine;

    #[test]
    fn speeds_follow_the_feeds_and_the_lift() {
        let s = machine().speeds();
        assert_eq!((s.draw_mm_s, s.travel_mm_s), (60.0, 100.0));
        assert!((s.pen_lift_s - 0.6).abs() < 1e-9);
    }

    #[test]
    fn a_page_that_fills_the_area_starts_at_the_top_left() {
        let map = machine().mapping(200.0, 174.0).unwrap();
        assert_eq!(map.to_machine([0.0, 0.0]), [28.0, 216.0]);
        assert_eq!(map.to_machine([200.0, 174.0]), [228.0, 42.0]);
    }

    #[test]
    fn a_smaller_page_is_centred() {
        let map = machine().mapping(100.0, 74.0).unwrap();
        assert_eq!(map.to_machine([0.0, 0.0]), [78.0, 166.0]);
    }

    #[test]
    fn a_larger_page_is_refused_with_both_sizes() {
        let err = machine().mapping(201.0, 100.0).unwrap_err().to_string();
        assert!(
            err.contains("201 by 100 mm") && err.contains("200 by 174 mm"),
            "{err}"
        );
    }

    #[test]
    fn validation_names_the_field() {
        let mut m = machine();
        m.z_up_mm = 3.0;
        assert!(m.validate().unwrap_err().to_string().contains("z_up_mm"));
        let mut m = machine();
        m.area.x_max = 300.0;
        assert!(m.validate().unwrap_err().to_string().contains("area"));
        let mut m = machine();
        m.draw_feed = 0.0;
        assert!(m.validate().unwrap_err().to_string().contains("draw_feed"));
        let mut m = machine();
        m.name = String::new();
        assert!(m.validate().unwrap_err().to_string().contains("name"));
    }
}
