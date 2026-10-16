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

/// Where the pen may be below the safe height, in machine mm: a corner and a size.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Area {
    /// X of the corner with the smallest X and Y; may be negative.
    pub x: f64,
    /// Y of the corner with the smallest X and Y; may be negative.
    pub y: f64,
    /// The size along X.
    pub width: f64,
    /// The size along Y.
    pub height: f64,
}

impl Area {
    /// Whether `p` lies inside the area, with `eps` of slack for rounding.
    pub fn contains(&self, [x, y]: [f64; 2], eps: f64) -> bool {
        self.x - eps <= x
            && x <= self.x + self.width + eps
            && self.y - eps <= y
            && y <= self.y + self.height + eps
    }
}

/// How the pen goes up and down.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PenLift {
    /// Z moves between the machine's pen heights.
    #[default]
    Z,
    /// The machine's own lines, such as a servo or solenoid command.
    Gcode,
}

impl PenLift {
    fn is_z(&self) -> bool {
        *self == PenLift::Z
    }
}

/// A machine that draws with a pen: its drawable area, pen lift, feeds and the G-code written
/// around a plot. The fields after `end_gcode` are left out of the JSON while they hold their
/// defaults, so a machine that does not use them reads and writes as it did before they existed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Machine {
    /// The machine's name, shown to users.
    pub name: String,
    /// Where the pen may be below `z_safe_mm`, or anywhere with a pen lifted by G-code lines.
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
    /// The dwell after each pen move, in ms.
    pub settle_ms: u32,
    /// Lines written after `G21` and `G90`, as they are.
    pub start_gcode: String,
    /// Lines written after the final lift, as they are.
    pub end_gcode: String,
    /// How the pen goes up and down; with [`PenLift::Gcode`] the Z heights and `z_feed` are unused.
    #[serde(default, skip_serializing_if = "PenLift::is_z")]
    pub pen_lift: PenLift,
    /// Lines that put the pen down, as they are, with [`PenLift::Gcode`].
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub pen_down_gcode: String,
    /// Lines that lift the pen, as they are, with [`PenLift::Gcode`].
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub pen_up_gcode: String,
    /// The longest straight move, in mm, for firmware that would draw a long one as a curve; none
    /// keeps every move whole.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_segment_mm: Option<f64>,
}

/// The shortest `max_segment_mm`, which bounds the number of lines a file can have.
pub const MIN_SEGMENT_MM: f64 = 0.5;

impl Machine {
    /// Check every field; the message names the first one that is wrong.
    pub fn validate(&self) -> Result<(), MachineError> {
        let bad = |what: String| Err(MachineError(what));
        if self.name.trim().is_empty() {
            return bad("name must not be empty".to_string());
        }
        let a = &self.area;
        for (name, v) in [("area.x", a.x), ("area.y", a.y)] {
            if !v.is_finite() {
                return bad(format!("{name} must be finite, got {v}"));
            }
        }
        for (name, v) in [("area.width", a.width), ("area.height", a.height)] {
            if !(v > 0.0 && v.is_finite()) {
                return bad(format!("{name} must be positive, got {v}"));
            }
        }
        match self.pen_lift {
            PenLift::Z => {
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
            }
            PenLift::Gcode => {
                for (name, lines) in [
                    ("pen_down_gcode", &self.pen_down_gcode),
                    ("pen_up_gcode", &self.pen_up_gcode),
                ] {
                    if lines.trim().is_empty() {
                        return bad(format!("{name} must have a line when pen_lift is gcode"));
                    }
                }
            }
        }
        let mut feeds = vec![
            ("draw_feed", self.draw_feed),
            ("travel_feed", self.travel_feed),
        ];
        if self.pen_lift == PenLift::Z {
            feeds.push(("z_feed", self.z_feed));
        }
        for (name, feed) in feeds {
            let positive = feed > 0.0;
            if !positive {
                return bad(format!("{name} must be positive, got {feed}"));
            }
        }
        if let Some(v) = self.max_segment_mm {
            if !(v >= MIN_SEGMENT_MM && v.is_finite()) {
                return bad(format!(
                    "max_segment_mm must be at least {MIN_SEGMENT_MM}, or null to keep moves whole, got {v}"
                ));
            }
        }
        Ok(())
    }

    /// The plot speeds the machine implies: the feeds in mm per second, and a pen lift as two
    /// dwells, plus the down and up Z moves at `z_feed` when the pen lifts by Z.
    pub fn speeds(&self) -> Speeds {
        let dwells = 2.0 * f64::from(self.settle_ms) / 1000.0;
        Speeds {
            draw_mm_s: self.draw_feed / 60.0,
            travel_mm_s: self.travel_feed / 60.0,
            pen_lift_s: match self.pen_lift {
                PenLift::Z => 2.0 * (self.z_up_mm - self.z_down_mm) / (self.z_feed / 60.0) + dwells,
                PenLift::Gcode => dwells,
            },
        }
    }

    /// Where a page of `width_mm` by `height_mm` lands: centred in the area. Fails when the page
    /// is wider or taller than the area, naming both sizes.
    pub fn mapping(&self, width_mm: f64, height_mm: f64) -> Result<Mapping, MachineError> {
        let (w, h) = (self.area.width, self.area.height);
        if width_mm > w + 1e-6 || height_mm > h + 1e-6 {
            return Err(MachineError(format!(
                "the {width_mm} by {height_mm} mm page is larger than the machine's {w} by {h} mm drawable area"
            )));
        }
        Ok(Mapping {
            ox: self.area.x + (w - width_mm) / 2.0,
            oy: self.area.y + (h + height_mm) / 2.0,
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
    use super::Machine;
    use crate::test_util::{machine, servo_machine};

    #[test]
    fn speeds_follow_the_feeds_and_the_lift() {
        let s = machine().speeds();
        assert_eq!((s.draw_mm_s, s.travel_mm_s), (60.0, 100.0));
        assert!((s.pen_lift_s - 0.6).abs() < 1e-9);
        // Lifted by G-code lines, the lift is the two 100 ms dwells alone.
        assert!((servo_machine().speeds().pen_lift_s - 0.2).abs() < 1e-9);
    }

    #[test]
    fn the_new_fields_are_left_out_at_their_defaults() {
        let v = serde_json::to_value(machine()).unwrap();
        for key in [
            "pen_lift",
            "pen_down_gcode",
            "pen_up_gcode",
            "max_segment_mm",
        ] {
            assert!(v.get(key).is_none(), "{key} in {v}");
        }
        assert_eq!(serde_json::from_value::<Machine>(v).unwrap(), machine());
        let mut m = servo_machine();
        m.max_segment_mm = Some(2.0);
        let v = serde_json::to_value(&m).unwrap();
        assert_eq!(
            (&v["pen_lift"], &v["max_segment_mm"]),
            (&serde_json::json!("gcode"), &serde_json::json!(2.0))
        );
        assert_eq!(serde_json::from_value::<Machine>(v).unwrap(), m);
    }

    #[test]
    fn a_pen_lifted_by_gcode_lines_needs_both_lines_and_no_z() {
        let mut m = servo_machine();
        (m.z_down_mm, m.z_up_mm, m.z_safe_mm, m.z_feed) = (0.0, 0.0, 0.0, 0.0);
        m.validate().unwrap();
        m.pen_up_gcode = " \n".to_string();
        let err = m.validate().unwrap_err().to_string();
        assert_eq!(err, "pen_up_gcode must have a line when pen_lift is gcode");
        let mut m = servo_machine();
        m.pen_down_gcode = String::new();
        assert!(m
            .validate()
            .unwrap_err()
            .to_string()
            .contains("pen_down_gcode"));
    }

    #[test]
    fn the_longest_move_has_a_floor() {
        let mut m = machine();
        m.max_segment_mm = Some(0.5);
        m.validate().unwrap();
        for bad in [0.4, 0.0, -1.0, f64::NAN, f64::INFINITY] {
            m.max_segment_mm = Some(bad);
            let err = m.validate().unwrap_err().to_string();
            assert!(
                err.starts_with("max_segment_mm must be at least 0.5"),
                "{err}"
            );
        }
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
    fn a_negative_offset_is_accepted() {
        let mut m = machine();
        m.area.x = -100.0;
        m.area.y = -100.0;
        m.validate().unwrap();
        let map = m.mapping(200.0, 174.0).unwrap();
        assert_eq!(map.to_machine([0.0, 0.0]), [-100.0, 74.0]);
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
        m.area.width = 0.0;
        assert!(m.validate().unwrap_err().to_string().contains("area.width"));
        let mut m = machine();
        m.area.x = f64::NAN;
        assert!(m.validate().unwrap_err().to_string().contains("area.x"));
        let mut m = machine();
        m.draw_feed = 0.0;
        assert!(m.validate().unwrap_err().to_string().contains("draw_feed"));
        let mut m = machine();
        m.name = String::new();
        assert!(m.validate().unwrap_err().to_string().contains("name"));
    }
}
