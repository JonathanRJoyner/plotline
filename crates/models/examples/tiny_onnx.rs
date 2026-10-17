//! Write the tiny test model to a file: `cargo run -p plotline-models --example tiny_onnx -- PATH`.

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: tiny_onnx <path to write>");
    std::fs::write(&path, plotline_models::tiny_conv_onnx()).expect("write the model");
}
