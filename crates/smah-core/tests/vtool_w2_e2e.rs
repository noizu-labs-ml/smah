//! Env-gated real-model e2e for the wave-2 drivers (tickets #22 soap,
//! #21 grpc). `#[ignore]`d — never runs in CI. Runs only when BOTH env
//! vars are set:
//!
//! - `SMAH_ALIBABA_API_KEY` — dashscope key (never printed, never logged)
//! - `ALIBABA_BASE_URL` — OpenAI-compatible base, e.g.
//!   `https://dashscope.aliyuncs.com/compatible-mode/v1`
//!
//! By hand: `cargo test -p smah-core --test vtool_w2_e2e -- --ignored --nocapture`

use std::sync::Arc;
use std::time::Duration;

use smah_core::errors::SmahError;
use smah_core::virtual_tools::driver::{ChatBackend, ChatMessage, FsMode, Role, Sink};
use smah_core::virtual_tools::drivers::grpc;
use smah_core::virtual_tools::drivers::soap;
use smah_core::virtual_tools::loader::ToolDef;
use tonic::Status;

const WSDL: &str = r#"<?xml version="1.0"?>
<wsdl:definitions xmlns:wsdl="http://schemas.xmlsoap.org/wsdl/"
    xmlns:tns="urn:soaptest"
    targetNamespace="urn:soaptest">
  <wsdl:message name="pingIn"><wsdl:part name="body" element="tns:ping"/></wsdl:message>
  <wsdl:message name="pingOut"><wsdl:part name="body" element="tns:pingResponse"/></wsdl:message>
  <wsdl:portType name="pt">
    <wsdl:operation name="Ping">
      <wsdl:input message="tns:pingIn"/>
      <wsdl:output message="tns:pingOut"/>
    </wsdl:operation>
  </wsdl:portType>
  <wsdl:binding name="b" type="tns:pt">
    <wsdl:operation name="Ping"/>
  </wsdl:binding>
  <wsdl:service name="svc"><wsdl:port name="p" binding="tns:b"/></wsdl:service>
</wsdl:definitions>"#;

const PROTO: &str = r#"syntax = "proto3";
package demo.v1;
message PingRequest { string name = 1; }
message PingReply { string message = 1; }
service Greeter {
  rpc Ping(PingRequest) returns (PingReply);
}
"#;

fn gated_env() -> Option<(String, String)> {
    let key = std::env::var("SMAH_ALIBABA_API_KEY").ok()?;
    let base = std::env::var("ALIBABA_BASE_URL").ok()?;
    if key.is_empty() || base.is_empty() {
        return None;
    }
    Some((key, base))
}

/// Tiny OpenAI-compatible chat client (the real-model stand-in for
/// `ChatBackend` in e2e runs only).
struct QwenBackend {
    client: reqwest::Client,
    key: String,
    base: String,
    model: String,
}

impl ChatBackend for QwenBackend {
    fn stream_turn(
        &self,
        model: &str,
        system: &str,
        messages: &[ChatMessage],
        _on_delta: &mut dyn FnMut(&str),
    ) -> Result<String, SmahError> {
        let mut body = serde_json::json!({
            "model": if model.is_empty() { &self.model } else { model },
            "stream": false,
            "messages": [],
        });
        let list = body
            .get_mut("messages")
            .and_then(|v| v.as_array_mut())
            .expect("messages array");
        list.push(serde_json::json!({ "role": "system", "content": system }));
        for m in messages {
            let role = match m.role {
                Role::User => "user",
                Role::Assistant => "assistant",
            };
            list.push(serde_json::json!({ "role": role, "content": m.content }));
        }

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| SmahError::Internal {
                detail: format!("e2e runtime: {e}"),
            })?;
        let url = format!("{}/chat/completions", self.base.trim_end_matches('/'));
        let text = rt.block_on(async {
            let resp = self
                .client
                .post(&url)
                .bearer_auth(&self.key)
                .json(&body)
                .timeout(Duration::from_secs(90))
                .send()
                .await
                .map_err(|e| SmahError::Internal {
                    detail: format!("e2e request failed: {e}"),
                })?;
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            if !status.is_success() {
                // Truncate upstream error bodies; they must never carry the key.
                let snippet: String = text.chars().take(400).collect();
                return Err(SmahError::Internal {
                    detail: format!("e2e upstream {status}: {snippet}"),
                });
            }
            Ok(text)
        })?;
        let parsed: serde_json::Value =
            serde_json::from_str(&text).map_err(|e| SmahError::Internal {
                detail: format!("e2e response parse: {e}"),
            })?;
        parsed
            .pointer("/choices/0/message/content")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .ok_or_else(|| SmahError::Internal {
                detail: "e2e response missing choices[0].message.content".to_string(),
            })
    }
}

fn e2e_tool(name: &str, interface: &str, dir: &std::path::Path) -> ToolDef {
    let path = dir.join(format!("{name}.md"));
    std::fs::write(&path, format!("# {name}\npersona\n")).expect("tool file");
    ToolDef {
        name: name.to_string(),
        description: "e2e fixture".to_string(),
        model: None,
        driver: "x".to_string(),
        constraints: Vec::new(),
        allow_roots: Vec::new(),
        interface: Some(interface.to_string()),
        body: "You are a helpful sandbox service emulator.".to_string(),
        path,
    }
}

fn wait_for_addr(sink: &Arc<std::sync::Mutex<Vec<String>>>) -> String {
    for _ in 0..200 {
        {
            let lines = sink.lock().expect("sink lock");
            for line in lines.iter() {
                if let Some(idx) = line.find("127.0.0.1:") {
                    let rest = &line[idx..];
                    let end = rest
                        .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == ':'))
                        .unwrap_or(rest.len());
                    return rest[..end].to_string();
                }
            }
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("driver never advertised a loopback address");
}

#[test]
#[ignore = "real-qwen e2e; runs only with SMAH_ALIBABA_API_KEY + ALIBABA_BASE_URL set"]
fn e2e_soap_real_qwen_fabricates_the_declared_operation() {
    let Some((key, base)) = gated_env() else {
        panic!("set SMAH_ALIBABA_API_KEY and ALIBABA_BASE_URL to run this e2e");
    };
    let dir = tempfile::tempdir().expect("tmp");
    let tool = e2e_tool("itw2e2es", WSDL, dir.path());
    let backend = Arc::new(QwenBackend {
        client: reqwest::Client::new(),
        key,
        base,
        model: "qwen3.8-flash".to_string(),
    });
    let sink_lines: Arc<std::sync::Mutex<Vec<String>>> = Default::default();
    let sink_for_wait = Arc::clone(&sink_lines);
    let audit = smah_core::virtual_tools::audit::Audit::open("e2e");
    struct Tee(Arc<std::sync::Mutex<Vec<String>>>);
    impl Sink for Tee {
        fn content(&mut self, _p: &str) {}
        fn notice(&mut self, l: &str) {
            self.0.lock().expect("lock").push(l.to_string());
        }
    }
    let handle = std::thread::spawn(move || {
        let mut s = Tee(sink_lines);
        soap::run(
            &tool,
            &*backend,
            &mut s,
            "qwen3.8-flash",
            &[
                "--port".to_string(),
                "0".to_string(),
                "--max-serve-seconds".to_string(),
                "45".to_string(),
            ],
            None,
            false,
            &FsMode::NoneDeclared,
            &audit,
            "e2e",
        )
    });
    let addr = wait_for_addr(&sink_for_wait);

    let envelope = "<soapenv:Envelope xmlns:soapenv=\"http://schemas.xmlsoap.org/soap/envelope/\"><soapenv:Body><tns:ping xmlns:tns=\"urn:soaptest\"><tns:name>e2e</tns:name></tns:ping></soapenv:Body></soapenv:Envelope>";
    let request = format!(
        "POST / HTTP/1.1\r\nHost: {addr}\r\nContent-Type: text/xml; charset=utf-8\r\n\
         SOAPAction: \"Ping\"\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{envelope}",
        envelope.len()
    );
    let resp = {
        use std::io::{Read, Write};
        let mut stream = std::net::TcpStream::connect(&addr).expect("connect");
        stream
            .set_read_timeout(Some(Duration::from_secs(120)))
            .expect("timeout");
        stream.write_all(request.as_bytes()).expect("write");
        let mut buf = Vec::new();
        let _ = stream.read_to_end(&mut buf);
        String::from_utf8_lossy(&buf).into_owned()
    };
    handle.join().expect("serve thread").expect("run ok");
    assert!(resp.contains("200"), "e2e invoke failed: {resp}");
    assert!(
        resp.contains("pingResponse"),
        "fabricated element present: {resp}"
    );
}

#[test]
#[ignore = "real-qwen e2e; runs only with SMAH_ALIBABA_API_KEY + ALIBABA_BASE_URL set"]
fn e2e_grpc_real_qwen_fabricates_protojson() {
    let Some((key, base)) = gated_env() else {
        panic!("set SMAH_ALIBABA_API_KEY and ALIBABA_BASE_URL to run this e2e");
    };
    let dir = tempfile::tempdir().expect("tmp");
    let tool = e2e_tool("itw2e2eg", PROTO, dir.path());
    let backend = Arc::new(QwenBackend {
        client: reqwest::Client::new(),
        key,
        base,
        model: "qwen3.8-flash".to_string(),
    });
    let sink_lines: Arc<std::sync::Mutex<Vec<String>>> = Default::default();
    let sink_for_wait = Arc::clone(&sink_lines);

    let audit = smah_core::virtual_tools::audit::Audit::open("e2e");
    struct Tee(Arc<std::sync::Mutex<Vec<String>>>);
    impl Sink for Tee {
        fn content(&mut self, _p: &str) {}
        fn notice(&mut self, l: &str) {
            self.0.lock().expect("lock").push(l.to_string());
        }
    }
    let handle = std::thread::spawn(move || {
        let mut s = Tee(sink_lines);
        grpc::run(
            &tool,
            &*backend,
            &mut s,
            "qwen3.8-flash",
            &[
                "--port".to_string(),
                "0".to_string(),
                "--max-serve-seconds".to_string(),
                "45".to_string(),
            ],
            None,
            false,
            &FsMode::NoneDeclared,
            &audit,
            "e2e",
        )
    });
    let addr = wait_for_addr(&sink_for_wait);

    let pool = {
        let tdir = tempfile::tempdir().expect("tmp");
        let ppath = tdir.path().join("p.proto");
        std::fs::write(&ppath, PROTO).expect("write");
        let fds = protox::compile([&ppath], [tdir.path()]).expect("compile");
        prost_reflect::DescriptorPool::from_file_descriptor_set(fds).expect("pool")
    };
    let mut req = prost_reflect::DynamicMessage::new(
        pool.get_message_by_name("demo.v1.PingRequest")
            .expect("req"),
    );
    req.set_field_by_name("name", prost_reflect::Value::String("e2e".to_string()));

    let reply = {
        let input = pool.get_message_by_name("demo.v1.PingRequest").expect("in");
        let output = pool.get_message_by_name("demo.v1.PingReply").expect("out");
        let codec = E2eCodec {
            input: input.clone(),
            output: output.clone(),
        };
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        rt.block_on(async move {
            let channel = tonic::transport::Endpoint::from_shared(format!("http://{addr}"))
                .expect("endpoint")
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(90))
                .connect()
                .await
                .expect("connect");
            let mut client = tonic::client::Grpc::new(channel);
            client.ready().await.expect("ready");
            let uri = http::uri::PathAndQuery::from_static("/demo.v1.Greeter/Ping");
            client
                .unary(tonic::Request::new(req), uri, codec)
                .await
                .expect("unary")
                .into_inner()
        })
    };
    handle.join().expect("serve thread").expect("run ok");
    let text = reply
        .get_field_by_name("message")
        .and_then(|v| v.as_str().map(str::to_string));
    assert!(
        text.as_deref().map(|s| !s.is_empty()).unwrap_or(false),
        "expected a fabricated message field"
    );
}

/// Client-side codec twin (encodes requests, decodes responses).
#[derive(Clone)]
struct E2eCodec {
    input: prost_reflect::MessageDescriptor,
    output: prost_reflect::MessageDescriptor,
}

impl tonic::codec::Codec for E2eCodec {
    type Encoder = E2eEnc;
    type Decoder = E2eDec;
    type Encode = prost_reflect::DynamicMessage;
    type Decode = prost_reflect::DynamicMessage;
    fn encoder(&mut self) -> Self::Encoder {
        E2eEnc(self.input.clone())
    }
    fn decoder(&mut self) -> Self::Decoder {
        E2eDec(self.output.clone())
    }
}

struct E2eEnc(prost_reflect::MessageDescriptor);
struct E2eDec(prost_reflect::MessageDescriptor);

impl tonic::codec::Encoder for E2eEnc {
    type Item = prost_reflect::DynamicMessage;
    type Error = Status;
    fn encode(
        &mut self,
        item: Self::Item,
        dst: &mut tonic::codec::EncodeBuf<'_>,
    ) -> Result<(), Status> {
        use prost::Message as _;
        use prost_reflect::ReflectMessage as _;
        debug_assert_eq!(
            item.descriptor().full_name(),
            self.0.full_name(),
            "e2e client encoded a message outside the declared input type"
        );
        item.encode(dst)
            .map_err(|e| Status::internal(format!("e2e encode: {e}")))
    }
}

impl tonic::codec::Decoder for E2eDec {
    type Item = prost_reflect::DynamicMessage;
    type Error = Status;
    fn decode(
        &mut self,
        src: &mut tonic::codec::DecodeBuf<'_>,
    ) -> Result<Option<Self::Item>, Status> {
        prost_reflect::DynamicMessage::decode(self.0.clone(), src)
            .map(Some)
            .map_err(|e| Status::internal(format!("e2e decode: {e}")))
    }
}
