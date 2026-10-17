//! A tiny ONNX model built in code, for tests of the runtime here and of the browser's.
//!
//! The model is one 3 x 3 convolution with padding 1 and a Sigmoid after it. Its weights follow a
//! formula, so a test can compute the same answer directly and compare, and no model file from
//! anywhere else is needed.

use prost::Message;
use tract_onnx::pb::{
    attribute_proto::AttributeType, tensor_proto::DataType,
    tensor_shape_proto::dimension::Value as Dim, tensor_shape_proto::Dimension, type_proto,
    AttributeProto, GraphProto, ModelProto, NodeProto, OperatorSetIdProto, TensorProto,
    TensorShapeProto, TypeProto, ValueInfoProto,
};

/// The model that takes 3 channels and answers with 1: the one the runtime accepts.
pub fn tiny_conv_onnx() -> Vec<u8> {
    tiny_conv_onnx_with(3, 1)
}

/// The tiny model with `in_channels` going in and `out_channels` coming out.
///
/// The weight from input channel `c` to output channel `o` at kernel row `y` and column `x` is
/// `(o + c + 1) * 0.01 * (y * 3 + x - 4)`; the bias of every output channel is 0.5.
pub fn tiny_conv_onnx_with(in_channels: usize, out_channels: usize) -> Vec<u8> {
    let mut weights = Vec::with_capacity(out_channels * in_channels * 9);
    for o in 0..out_channels {
        for c in 0..in_channels {
            for y in 0..3 {
                for x in 0..3 {
                    weights.push((o + c + 1) as f32 * 0.01 * ((y * 3 + x) as f32 - 4.0));
                }
            }
        }
    }
    let initializer = |name: &str, dims: Vec<i64>, float_data: Vec<f32>| TensorProto {
        name: name.into(),
        dims,
        data_type: DataType::Float as i32,
        float_data,
        ..Default::default()
    };
    let ints = |name: &str, ints: Vec<i64>| AttributeProto {
        name: name.into(),
        r#type: AttributeType::Ints as i32,
        ints,
        ..Default::default()
    };
    let node = |op_type: &str, input: &[&str], output: &str, attribute| NodeProto {
        op_type: op_type.into(),
        input: input.iter().map(|s| s.to_string()).collect(),
        output: vec![output.into()],
        attribute,
        ..Default::default()
    };
    let graph = GraphProto {
        name: "tiny-conv".into(),
        node: vec![
            node(
                "Conv",
                &["x", "w", "b"],
                "c",
                vec![ints("kernel_shape", vec![3, 3]), ints("pads", vec![1; 4])],
            ),
            node("Sigmoid", &["c"], "y", vec![]),
        ],
        initializer: vec![
            initializer(
                "w",
                vec![out_channels as i64, in_channels as i64, 3, 3],
                weights,
            ),
            initializer("b", vec![out_channels as i64], vec![0.5; out_channels]),
        ],
        input: vec![value_info("x", in_channels)],
        output: vec![value_info("y", out_channels)],
        ..Default::default()
    };
    ModelProto {
        ir_version: 8,
        opset_import: vec![OperatorSetIdProto {
            domain: String::new(),
            version: 13,
        }],
        producer_name: "plotline".into(),
        graph: Some(graph),
        ..Default::default()
    }
    .encode_to_vec()
}

/// A float tensor `1 x channels x h x w` with the picture's height and width left open.
fn value_info(name: &str, channels: usize) -> ValueInfoProto {
    let dim = |value| Dimension {
        denotation: String::new(),
        value: Some(value),
    };
    ValueInfoProto {
        name: name.into(),
        r#type: Some(TypeProto {
            value: Some(type_proto::Value::TensorType(type_proto::Tensor {
                elem_type: DataType::Float as i32,
                shape: Some(TensorShapeProto {
                    dim: vec![
                        dim(Dim::DimValue(1)),
                        dim(Dim::DimValue(channels as i64)),
                        dim(Dim::DimParam("h".into())),
                        dim(Dim::DimParam("w".into())),
                    ],
                }),
            })),
            ..Default::default()
        }),
        ..Default::default()
    }
}
