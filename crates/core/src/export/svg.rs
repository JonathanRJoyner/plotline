//! SVG export: a drawing as SVG text with one group per pen layer.
//!
//! The page is in millimetres. `width` and `height` carry the unit and the `viewBox` has the same
//! numbers, so one user unit is one mm and coordinates are written as they are. The origin is the
//! top-left corner of the page and y grows down, as it does in the engine, so nothing is flipped.
//! Only plain SVG is written: no vendor-namespaced attributes.

use serde_json::Value;

use crate::types::Drawing;

/// `s` with the characters that are special in an XML attribute value written as entities.
fn escape(s: &str) -> String {
    // `&` goes first so that the entities the later replacements write are not escaped again.
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// The `d` attribute of one path: `M x y L x y ...` with three decimals.
fn path_data(path: &[[f64; 2]]) -> String {
    let mut points: Vec<String> = path
        .iter()
        .map(|p| format!("{:.3} {:.3}", p[0], p[1]))
        .collect();
    if let [only] = points.as_slice() {
        // A dot is a zero-length line, which a round line cap paints as a dot of the pen width.
        points.push(only.clone());
    }
    format!("M {}", points.join(" L "))
}

/// The SVG text for `d`.
///
/// Each layer is a `<g>` numbered from 1 in drawing order (`pen-1`, `pen-2`, ...) that carries the
/// pen's name in `data-pen`, its colour as the stroke and its width as the stroke width, with
/// round caps and joins. Each path in the layer is one `<path>`.
///
/// With a `preset`, the line after the opening tag is `<metadata id="plotline-preset">` holding it
/// as compact JSON with `&`, `<` and `>` written as entities, so the file loads as a preset again.
pub fn to_svg(d: &Drawing, preset: Option<&Value>) -> String {
    let (w, h) = (d.canvas.width_mm, d.canvas.height_mm);
    let mut lines = vec![format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}mm" height="{h}mm" viewBox="0 0 {w} {h}">"#
    )];
    if let Some(preset) = preset {
        let json = preset.to_string();
        let json = json
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;");
        lines.push(format!(
            r#"  <metadata id="plotline-preset">{json}</metadata>"#
        ));
    }
    for (i, layer) in d.layers.iter().enumerate() {
        let pen = &layer.pen;
        lines.push(format!(
            r#"  <g id="pen-{}" data-pen="{}" fill="none" stroke="{}" stroke-width="{}" stroke-linecap="round" stroke-linejoin="round">"#,
            i + 1,
            escape(&pen.name),
            escape(&pen.color),
            pen.width_mm
        ));
        lines.extend(
            layer
                .paths
                .iter()
                .map(|p| format!(r#"    <path d="{}"/>"#, path_data(p))),
        );
        lines.push("  </g>".to_string());
    }
    lines.push("</svg>".to_string());
    lines.join("\n") + "\n"
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::drawing_with;
    use serde_json::json;

    #[test]
    fn svg_structure_and_dots() {
        let d = drawing_with(vec![vec![[10.0, 10.0], [20.0, 15.5]], vec![[30.0, 30.0]]]);
        let svg = to_svg(&d, None);
        assert!(svg.starts_with(r#"<svg xmlns="http://www.w3.org/2000/svg" width="210mm" height="297mm" viewBox="0 0 210 297">"#));
        assert!(svg.contains(r##"<g id="pen-1" data-pen="Black 0.3" fill="none" stroke="#000000" stroke-width="0.3" stroke-linecap="round" stroke-linejoin="round">"##));
        assert!(svg.contains(r#"<path d="M 10.000 10.000 L 20.000 15.500"/>"#));
        assert!(svg.contains(r#"<path d="M 30.000 30.000 L 30.000 30.000"/>"#));
        assert!(svg.ends_with("</svg>\n"));
    }
    #[test]
    fn svg_escapes_pen_names() {
        let mut d = drawing_with(vec![]);
        d.layers[0].pen.name = "A&B \"x\"".into();
        assert!(to_svg(&d, None).contains(r#"data-pen="A&amp;B &quot;x&quot;""#));
    }
    #[test]
    fn svg_carries_the_preset_only_when_given_one() {
        let d = drawing_with(vec![]);
        let plain = to_svg(&d, None);
        assert!(!plain.contains("metadata"));
        let preset = json!({"style": "a & <b>"});
        let svg = to_svg(&d, Some(&preset));
        let mut lines: Vec<&str> = svg.lines().collect();
        assert_eq!(
            lines.remove(1),
            r#"  <metadata id="plotline-preset">{"style":"a &amp; &lt;b&gt;"}</metadata>"#
        );
        // Without that line it is the plain export.
        assert_eq!(lines.join("\n") + "\n", plain);
    }
}
