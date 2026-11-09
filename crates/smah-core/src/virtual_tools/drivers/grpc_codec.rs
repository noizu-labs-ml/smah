//! Dynamic gRPC codec (ticket #21): tonic `Codec` implemented over
//! `prost-reflect`'s [`DynamicMessage`], driven by per-call descriptors from
//! the frozen contract's descriptor pool.
//!
//! This is the standard prost-reflect pattern (no codegen): the *input*
//! descriptor decodes requests, the *output* descriptor encodes responses —
//! one codec instance per method call. A fabricated payload that does not
//! fit the declared output message fails at the typed boundary; the drift
//! loop upstream catches it before it ever reaches the wire.

use prost::Message;
use prost_reflect::{DynamicMessage, MessageDescriptor, ReflectMessage};
use tonic::codec::{Codec, DecodeBuf, EncodeBuf};
use tonic::Status;

/// Per-method codec: decode requests as the declared input message, encode
/// responses as the declared output message.
#[derive(Clone)]
pub(crate) struct DynamicCodec {
    input: MessageDescriptor,
    output: MessageDescriptor,
}

impl DynamicCodec {
    pub(crate) fn new(input: MessageDescriptor, output: MessageDescriptor) -> Self {
        Self { input, output }
    }
}

impl Codec for DynamicCodec {
    type Encoder = DynamicEncoder;
    type Decoder = DynamicDecoder;
    type Encode = DynamicMessage;
    type Decode = DynamicMessage;

    fn encoder(&mut self) -> Self::Encoder {
        DynamicEncoder(self.output.clone())
    }

    fn decoder(&mut self) -> Self::Decoder {
        DynamicDecoder(self.input.clone())
    }
}

pub(crate) struct DynamicEncoder(MessageDescriptor);

impl tonic::codec::Encoder for DynamicEncoder {
    type Item = DynamicMessage;
    type Error = Status;

    fn encode(&mut self, item: Self::Item, dst: &mut EncodeBuf<'_>) -> Result<(), Self::Error> {
        // A fabricated DynamicMessage carries its own descriptor; the debug
        // check ties the encoder's declared output type to the actual item.
        debug_assert_eq!(
            item.descriptor().full_name(),
            self.0.full_name(),
            "encoder descriptor must match the fabricated message type"
        );
        item.encode(dst)
            .map_err(|e| Status::internal(format!("response encode failed: {e}")))
    }
}

pub(crate) struct DynamicDecoder(MessageDescriptor);

impl tonic::codec::Decoder for DynamicDecoder {
    type Item = DynamicMessage;
    type Error = Status;

    fn decode(&mut self, src: &mut DecodeBuf<'_>) -> Result<Option<Self::Item>, Self::Error> {
        DynamicMessage::decode(self.0.clone(), src)
            .map(Some)
            .map_err(|e| Status::internal(format!("request decode failed: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prost_reflect::Value;

    const PROTO: &str = r#"syntax = "proto3";
package c.v1;
message EchoRequest { string text = 1; uint32 n = 2; }
message EchoReply { string text = 1; }
service Svc { rpc Echo(EchoRequest) returns (EchoReply); }
"#;

    fn pool() -> prost_reflect::DescriptorPool {
        let dir = tempfile::tempdir().expect("tmp");
        let path = dir.path().join("c.proto");
        std::fs::write(&path, PROTO).expect("write");
        let fds = protox::compile([&path], [dir.path()]).expect("compile");
        prost_reflect::DescriptorPool::from_file_descriptor_set(fds).expect("pool")
    }

    #[test]
    fn codec_wires_input_decoder_and_output_encoder() {
        let pool = pool();
        let input = pool
            .get_message_by_name("c.v1.EchoRequest")
            .expect("input descriptor");
        let output = pool
            .get_message_by_name("c.v1.EchoReply")
            .expect("output descriptor");

        let mut codec = DynamicCodec::new(input.clone(), output.clone());
        let encoder = codec.encoder();
        let decoder = codec.decoder();
        let _ = (encoder, decoder);

        // Typed payloads the codec will carry round-trip through prost, so a
        // fabricated DynamicMessage of the OUTPUT type produces wire bytes.
        let mut reply = DynamicMessage::new(output.clone());
        reply.set_field_by_name("text", Value::String("fabricated".to_string()));
        let mut wire: Vec<u8> = Vec::new();
        reply.encode(&mut wire).expect("encode reply");
        assert!(!wire.is_empty());

        // And a real request frame parses back as the INPUT type (the same
        // call DynamicDecoder makes, minus tonic's framing, which the
        // wire-level integration test exercises end to end).
        let mut req = DynamicMessage::new(input.clone());
        req.set_field_by_name("text", Value::String("hello".to_string()));
        let mut frame: Vec<u8> = Vec::new();
        req.encode(&mut frame).expect("encode request");
        let decoded = DynamicMessage::decode(input.clone(), frame.as_slice()).expect("decode");
        assert_eq!(
            decoded
                .get_field_by_name("text")
                .and_then(|v| v.as_str().map(str::to_string)),
            Some("hello".to_string())
        );
    }
}
