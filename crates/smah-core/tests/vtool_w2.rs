//! Wave-2 integration tests (tickets #22 soap + #21 grpc): wire-level
//! coverage of the two interface-first drivers against a fake chat backend.
//!
//! Everything runs on loopback: the drivers bind 127.0.0.1 with
//! `--port 0`, the test parses the real address out of the driver's
//! notices, and `--max-serve-seconds` bounds every run. Soap is driven
//! with raw HTTP/1.1 over `TcpStream` (SOAP needs nothing fancier); grpc
//! is driven with a tonic client using a test-local dynamic codec, the
//! same DynamicMessage pattern the driver serves with.

use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use prost::Message as _;
use prost_reflect::{DynamicMessage, MessageDescriptor, ReflectMessage};
use smah_core::errors::SmahError;
use smah_core::virtual_tools::audit::{cache_dir, Audit};
use smah_core::virtual_tools::driver::{ChatBackend, ChatMessage, FsMode, RunOutcome, Sink};
use smah_core::virtual_tools::drivers::grpc;
use smah_core::virtual_tools::drivers::soap;
use smah_core::virtual_tools::loader::ToolDef;
use tonic::codec::{Codec, DecodeBuf, EncodeBuf};
use tonic::Status;

// ---------------------------------------------------------------- fixtures

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
  rpc Observe(PingRequest) returns (stream PingReply);
}
"#;

const GOOD_SOAP_REPLY: &str =
    "<tns:pingResponse xmlns:tns=\"urn:soaptest\"><tns:pong>hi</tns:pong></tns:pingResponse>";

type BackendFn = Arc<dyn Fn(&str, &[ChatMessage]) -> Result<String, SmahError> + Send + Sync>;

/// ChatBackend stub whose reply comes from a closure.
struct FakeBackend(BackendFn);

impl ChatBackend for FakeBackend {
    fn stream_turn(
        &self,
        _model: &str,
        _system: &str,
        messages: &[ChatMessage],
        _on_delta: &mut dyn FnMut(&str),
    ) -> Result<String, SmahError> {
        (self.0)("model", messages)
    }
}

/// Sink that keeps every notice for the test to inspect.
#[derive(Clone, Default)]
struct CollectSink {
    lines: Arc<Mutex<Vec<String>>>,
}

impl Sink for CollectSink {
    fn content(&mut self, _piece: &str) {}
    fn notice(&mut self, line: &str) {
        self.lines.lock().expect("sink lock").push(line.to_string());
    }
}

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    format!("{prefix}{nanos}")
}

fn tool(name: &str, interface: &str, dir: &std::path::Path) -> ToolDef {
    let path = dir.join(format!("{name}.md"));
    std::fs::write(
        &path,
        format!("# {name}\npersona\ninterface: {interface}\n"),
    )
    .expect("tool file");
    ToolDef {
        name: name.to_string(),
        description: "integration fixture".to_string(),
        model: None,
        driver: "x".to_string(),
        constraints: Vec::new(),
        allow_roots: Vec::new(),
        interface: Some(interface.to_string()),
        body: "persona".to_string(),
        path,
    }
}

/// Run the driver on a thread with `--port 0` + a serve deadline; returns
/// the join handle, the bound 127.0.0.1:PORT parsed from the notices, and
/// the shared sink.
fn serve(
    soap_driver: bool,
    tool: ToolDef,
    backend: FakeBackend,
    extra: &[&str],
) -> (
    std::thread::JoinHandle<Result<RunOutcome, SmahError>>,
    String,
    CollectSink,
) {
    let sink = CollectSink::default();
    let tname = tool.name.clone();
    let mut args: Vec<String> = vec![
        "--port".into(),
        "0".into(),
        "--max-serve-seconds".into(),
        "15".into(),
    ];
    args.extend(extra.iter().map(|s| s.to_string()));
    let shared = sink.clone();
    let handle = std::thread::spawn(move || {
        let audit = Audit::open_at(
            &std::env::temp_dir().join(format!("{tname}-audit.jsonl")),
            "it",
        );
        let fs = FsMode::NoneDeclared;
        let mut s = SharedSink(shared);
        if soap_driver {
            soap::run(
                &tool,
                &backend,
                &mut s,
                "test-model",
                &args,
                None,
                false,
                &fs,
                &audit,
                "it",
            )
        } else {
            grpc::run(
                &tool,
                &backend,
                &mut s,
                "test-model",
                &args,
                None,
                false,
                &fs,
                &audit,
                "it",
            )
        }
    });
    let addr = wait_for_addr(&sink);
    (handle, addr, sink)
}

struct SharedSink(CollectSink);

impl Sink for SharedSink {
    fn content(&mut self, piece: &str) {
        self.0.content(piece);
    }
    fn notice(&mut self, line: &str) {
        self.0.notice(line);
    }
}

/// Poll the sink until a notice carries the bound loopback address.
fn wait_for_addr(sink: &CollectSink) -> String {
    for _ in 0..200 {
        {
            let lines = sink.lines.lock().expect("sink lock");
            for line in lines.iter() {
                if let Some(idx) = line.find("127.0.0.1:") {
                    let rest = &line[idx..];
                    let end = rest
                        .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == ':'))
                        .unwrap_or(rest.len());
                    let candidate = &rest[..end];
                    if candidate
                        .rsplit(':')
                        .next()
                        .map(|p| p.parse::<u16>().is_ok())
                        == Some(true)
                    {
                        return candidate.to_string();
                    }
                }
            }
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("driver never advertised a loopback address");
}

/// One raw HTTP/1.1 transaction; returns the full response text.
fn http_txn(addr: &str, request: &str) -> String {
    use std::io::{Read, Write};
    let mut stream = std::net::TcpStream::connect(addr).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("read timeout");
    stream.write_all(request.as_bytes()).expect("write");
    let mut buf = Vec::new();
    let _ = stream.read_to_end(&mut buf);
    String::from_utf8_lossy(&buf).into_owned()
}

fn soap_post(addr: &str, body: &str) -> String {
    http_txn(
        addr,
        &format!(
            "POST / HTTP/1.1\r\nHost: {addr}\r\nContent-Type: text/xml; charset=utf-8\r\n\
             SOAPAction: \"Ping\"\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        ),
    )
}

// ------------------------------------------------------------ test codec

/// Test-local twin of the driver's DynamicCodec, with CLIENT-side roles:
/// a client encodes requests (Encode = input) and decodes responses
/// (Decode = output) — the mirror image of the server-side codec.
#[derive(Clone)]
struct TestCodec {
    input: MessageDescriptor,
    output: MessageDescriptor,
}

impl Codec for TestCodec {
    type Encoder = TestEnc;
    type Decoder = TestDec;
    type Encode = DynamicMessage;
    type Decode = DynamicMessage;

    fn encoder(&mut self) -> Self::Encoder {
        TestEnc(self.input.clone())
    }
    fn decoder(&mut self) -> Self::Decoder {
        TestDec(self.output.clone())
    }
}

struct TestEnc(MessageDescriptor);
struct TestDec(MessageDescriptor);

impl tonic::codec::Encoder for TestEnc {
    type Item = DynamicMessage;
    type Error = Status;
    fn encode(&mut self, item: Self::Item, dst: &mut EncodeBuf<'_>) -> Result<(), Status> {
        debug_assert_eq!(
            item.descriptor().full_name(),
            self.0.full_name(),
            "test encoder descriptor must match the message type"
        );
        item.encode(dst)
            .map_err(|e| Status::internal(format!("test encode: {e}")))
    }
}

impl tonic::codec::Decoder for TestDec {
    type Item = DynamicMessage;
    type Error = Status;
    fn decode(&mut self, src: &mut DecodeBuf<'_>) -> Result<Option<Self::Item>, Status> {
        DynamicMessage::decode(self.0.clone(), src)
            .map(Some)
            .map_err(|e| Status::internal(format!("test decode: {e}")))
    }
}

fn proto_pool(proto: &str) -> prost_reflect::DescriptorPool {
    let dir = tempfile::tempdir().expect("tmp");
    let path = dir.path().join("it.proto");
    std::fs::write(&path, proto).expect("write");
    let fds = protox::compile([&path], [dir.path()]).expect("compile");
    prost_reflect::DescriptorPool::from_file_descriptor_set(fds).expect("pool")
}

// ------------------------------------------------------------------ soap

#[test]
fn soap_missing_interface_is_a_hard_config_error() {
    let dir = tempfile::tempdir().expect("tmp");
    let tool = tool(&unique("itw2s"), "", dir.path());
    let backend = FakeBackend(Arc::new(|_, _| Ok(String::new())));
    let err = soap::run(
        &tool,
        &backend,
        &mut CollectSink::default(),
        "m",
        &["--no-serve".to_string()],
        None,
        false,
        &FsMode::NoneDeclared,
        &Audit::open("it"),
        "it",
    )
    .expect_err("must refuse");
    assert_eq!(err.exit_code(), smah_core::ExitCode::Config);
}

#[test]
fn soap_freeze_reuse_and_tamper_refreeze() {
    let dir = tempfile::tempdir().expect("tmp");
    let name = unique("itw2sf");
    let tool = tool(&name, WSDL, dir.path());
    let backend = FakeBackend(Arc::new(|_, _| Ok(String::new())));

    let run = |tool: &ToolDef| {
        soap::run(
            tool,
            &backend,
            &mut CollectSink::default(),
            "m",
            &["--no-serve".to_string()],
            None,
            false,
            &FsMode::NoneDeclared,
            &Audit::open("it"),
            "it",
        )
    };
    run(&tool).expect("first run freezes");
    let cache = cache_dir().join("virtual-tools").join(&name).join("soap");
    let wsdl_cache = cache.join("wsdl.xml");
    let frozen = std::fs::read_to_string(&wsdl_cache).expect("frozen wsdl");
    assert_eq!(frozen, WSDL, "freeze stores the contract verbatim");

    // Tamper: integrity pin must catch it and re-freeze the original.
    std::fs::write(&wsdl_cache, "<wsdl:definitions xmlns:wsdl=\"x\"/>").expect("tamper");
    run(&tool).expect("tampered load re-freezes");
    let restored = std::fs::read_to_string(&wsdl_cache).expect("restored");
    assert_eq!(restored, WSDL, "tampered cache was re-frozen from the tool");
}

#[test]
fn soap_serves_wsdl_and_fabricates_declared_operation() {
    let dir = tempfile::tempdir().expect("tmp");
    let name = unique("itw2sw");
    let tool = tool(&name, WSDL, dir.path());
    let backend = FakeBackend(Arc::new(|_, _| Ok(GOOD_SOAP_REPLY.to_string())));
    let (handle, addr, _sink) = serve(true, tool, backend, &[]);

    // Discovery: the frozen document, byte-for-byte.
    let wsdl_resp = http_txn(
        &addr,
        "GET /?wsdl HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
    );
    assert!(wsdl_resp.starts_with("HTTP/1.1 200"), "{wsdl_resp}");
    let body = wsdl_resp.split("\r\n\r\n").nth(1).unwrap_or_default();
    assert_eq!(body, WSDL, "?wsdl must serve the frozen contract verbatim");

    // Invocation: envelope in, fabricated element wrapped in a reply.
    let envelope = "<soapenv:Envelope xmlns:soapenv=\"http://schemas.xmlsoap.org/soap/envelope/\"><soapenv:Body><tns:ping xmlns:tns=\"urn:soaptest\"/></soapenv:Body></soapenv:Envelope>";
    let resp = soap_post(&addr, envelope);
    assert!(resp.contains("200"), "invoke must succeed: {resp}");
    assert!(
        resp.contains("pingResponse"),
        "fabricated element present: {resp}"
    );
    assert!(resp.contains("Envelope"), "reply is a SOAP envelope");

    handle.join().expect("serve thread").expect("run ok");
}

#[test]
fn soap_refuses_dtd_requests_with_a_fault() {
    let dir = tempfile::tempdir().expect("tmp");
    let tool = tool(&unique("itw2sd"), WSDL, dir.path());
    let backend = FakeBackend(Arc::new(|_, _| Ok(GOOD_SOAP_REPLY.to_string())));
    let (handle, addr, _sink) = serve(true, tool, backend, &[]);

    let xxe = "<?xml version=\"1.0\"?><!DOCTYPE p [<!ENTITY xxe SYSTEM \"file:///etc/passwd\">]><soapenv:Envelope xmlns:soapenv=\"http://schemas.xmlsoap.org/soap/envelope/\"><soapenv:Body><tns:ping xmlns:tns=\"urn:soaptest\">&xxe;</tns:ping></soapenv:Body></soapenv:Envelope>";
    let resp = soap_post(&addr, xxe);
    assert!(
        resp.contains("Fault"),
        "XXE attempt must be answered with a SOAP fault: {resp}"
    );
    assert!(!resp.contains("/etc/passwd"), "no resolved entity content");

    handle.join().expect("serve thread").expect("run ok");
}

#[test]
fn soap_drift_is_a_bounded_loop_overflow() {
    let dir = tempfile::tempdir().expect("tmp");
    let tool = tool(&unique("itw2sx"), WSDL, dir.path());
    let backend = FakeBackend(Arc::new(|_, _| Ok("<wrong>nope</wrong>".to_string())));
    let (handle, addr, _sink) = serve(true, tool, backend, &[]);

    let envelope = "<soapenv:Envelope xmlns:soapenv=\"http://schemas.xmlsoap.org/soap/envelope/\"><soapenv:Body><tns:ping xmlns:tns=\"urn:soaptest\"/></soapenv:Body></soapenv:Envelope>";
    let resp = soap_post(&addr, envelope);
    assert!(resp.contains("Fault") || resp.contains("500"), "{resp}");

    let outcome = handle.join().expect("serve thread");
    let err = outcome.expect_err("double drift must end the run");
    assert!(
        matches!(err, SmahError::DirectiveLoopOverflow { .. }),
        "expected loop overflow, got: {err}"
    );
}

#[test]
fn soap_audit_is_metadata_only() {
    let dir = tempfile::tempdir().expect("tmp");
    let name = unique("itw2sa");
    let tool = tool(&name, WSDL, dir.path());
    let backend = FakeBackend(Arc::new(|_, _| Ok(GOOD_SOAP_REPLY.to_string())));
    let (handle, addr, _sink) = serve(true, tool, backend, &[]);

    let envelope = "<soapenv:Envelope xmlns:soapenv=\"http://schemas.xmlsoap.org/soap/envelope/\"><soapenv:Body><tns:ping xmlns:tns=\"urn:soaptest\">SECRETPAYLOAD</tns:ping></soapenv:Body></soapenv:Envelope>";
    let resp = soap_post(&addr, envelope);
    assert!(resp.contains("200"), "{resp}");
    handle.join().expect("serve thread").expect("run ok");

    let audit_path = std::env::temp_dir().join(format!("{name}-audit.jsonl"));
    let audit = std::fs::read_to_string(&audit_path).expect("audit file");
    assert!(
        audit.contains("soap.request"),
        "invocations audited: {audit}"
    );
    assert!(
        !audit.contains("SECRETPAYLOAD"),
        "audit must never carry request bytes: {audit}"
    );
}

#[test]
fn soap_unknown_flag_is_a_usage_error() {
    let dir = tempfile::tempdir().expect("tmp");
    let tool = tool(&unique("itw2su"), WSDL, dir.path());
    let backend = FakeBackend(Arc::new(|_, _| Ok(String::new())));
    let err = soap::run(
        &tool,
        &backend,
        &mut CollectSink::default(),
        "m",
        &["--wat".to_string()],
        None,
        false,
        &FsMode::NoneDeclared,
        &Audit::open("it"),
        "it",
    )
    .expect_err("unknown flag");
    assert_eq!(err.exit_code(), smah_core::ExitCode::Usage);
}

// ------------------------------------------------------------------ grpc

#[test]
fn grpc_missing_interface_is_a_hard_config_error() {
    let dir = tempfile::tempdir().expect("tmp");
    let tool = tool(&unique("itw2g"), "", dir.path());
    let backend = FakeBackend(Arc::new(|_, _| Ok(String::new())));
    let err = grpc::run(
        &tool,
        &backend,
        &mut CollectSink::default(),
        "m",
        &["--no-serve".to_string()],
        None,
        false,
        &FsMode::NoneDeclared,
        &Audit::open("it"),
        "it",
    )
    .expect_err("must refuse");
    assert_eq!(err.exit_code(), smah_core::ExitCode::Config);
}

#[test]
fn grpc_freeze_reuse_and_tamper_refreeze() {
    let dir = tempfile::tempdir().expect("tmp");
    let name = unique("itw2gf");
    let tool = tool(&name, PROTO, dir.path());
    let backend = FakeBackend(Arc::new(|_, _| Ok(String::new())));
    let run = |tool: &ToolDef| {
        grpc::run(
            tool,
            &backend,
            &mut CollectSink::default(),
            "m",
            &["--no-serve".to_string()],
            None,
            false,
            &FsMode::NoneDeclared,
            &Audit::open("it"),
            "it",
        )
    };
    run(&tool).expect("first run freezes");
    let proto_cache = cache_dir()
        .join("virtual-tools")
        .join(&name)
        .join("grpc")
        .join("grpc.proto");
    assert_eq!(
        std::fs::read_to_string(&proto_cache).expect("frozen proto"),
        PROTO
    );
    std::fs::write(&proto_cache, "syntax = \"proto3\";").expect("tamper");
    run(&tool).expect("tampered load re-freezes");
    assert_eq!(
        std::fs::read_to_string(&proto_cache).expect("restored"),
        PROTO,
        "tampered cache was re-frozen from the tool"
    );
}

#[test]
fn grpc_unknown_flag_is_a_usage_error() {
    let dir = tempfile::tempdir().expect("tmp");
    let tool = tool(&unique("itw2gu"), PROTO, dir.path());
    let backend = FakeBackend(Arc::new(|_, _| Ok(String::new())));
    let err = grpc::run(
        &tool,
        &backend,
        &mut CollectSink::default(),
        "m",
        &["--wat".to_string()],
        None,
        false,
        &FsMode::NoneDeclared,
        &Audit::open("it"),
        "it",
    )
    .expect_err("unknown flag");
    assert_eq!(err.exit_code(), smah_core::ExitCode::Usage);
}

/// Serve the grpc driver and drive it with a tonic client.
fn grpc_client_call(
    addr: &str,
    path: &'static str,
    request: DynamicMessage,
    streaming: bool,
) -> Result<DynamicMessage, Status> {
    let pool = proto_pool(PROTO);
    let input = pool
        .get_message_by_name("demo.v1.PingRequest")
        .expect("input");
    let output = pool
        .get_message_by_name("demo.v1.PingReply")
        .expect("output");
    let codec = TestCodec {
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
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(5))
            .connect()
            .await
            .expect("connect");
        let mut grpc = tonic::client::Grpc::new(channel);
        grpc.ready().await.expect("ready");
        let uri = http::uri::PathAndQuery::from_static(path);
        if streaming {
            let mut stream = grpc
                .server_streaming(tonic::Request::new(request), uri, codec)
                .await?
                .into_inner();
            // Trailers-only refusals surface as the first message error.
            match stream.message().await {
                Ok(Some(msg)) => Ok(msg),
                Ok(None) => Ok(DynamicMessage::new(output)),
                Err(status) => Err(status),
            }
        } else {
            let resp = grpc.unary(tonic::Request::new(request), uri, codec).await?;
            Ok(resp.into_inner())
        }
    })
}

#[test]
fn grpc_unary_round_trip_through_the_wire() {
    let dir = tempfile::tempdir().expect("tmp");
    let tool = tool(&unique("itw2gr"), PROTO, dir.path());
    let backend = FakeBackend(Arc::new(|_, _| {
        Ok("{\"message\": \"fabricated hello\"}".to_string())
    }));
    let (handle, addr, _sink) = serve(false, tool, backend, &[]);

    let pool = proto_pool(PROTO);
    let mut req = DynamicMessage::new(
        pool.get_message_by_name("demo.v1.PingRequest")
            .expect("req"),
    );
    req.set_field_by_name("name", prost_reflect::Value::String("wire".to_string()));

    let reply = grpc_client_call(&addr, "/demo.v1.Greeter/Ping", req, false)
        .expect("unary call must succeed");
    let text = reply
        .get_field_by_name("message")
        .and_then(|v| v.as_str().map(str::to_string));
    assert_eq!(text.as_deref(), Some("fabricated hello"));

    handle.join().expect("serve thread").expect("run ok");
}

#[test]
fn grpc_streaming_methods_are_refused_unary_only() {
    let dir = tempfile::tempdir().expect("tmp");
    let tool = tool(&unique("itw2gs"), PROTO, dir.path());
    let backend = FakeBackend(Arc::new(|_, _| Ok("{}".to_string())));
    let (handle, addr, _sink) = serve(false, tool, backend, &[]);

    let pool = proto_pool(PROTO);
    let req = DynamicMessage::new(
        pool.get_message_by_name("demo.v1.PingRequest")
            .expect("req"),
    );
    let err = grpc_client_call(&addr, "/demo.v1.Greeter/Observe", req, true)
        .expect_err("streaming must be refused");
    assert_eq!(err.code(), tonic::Code::Unimplemented, "{err}");
    assert!(err.message().contains("unary"), "{err}");

    handle.join().expect("serve thread").expect("run ok");
}

#[test]
fn grpc_unknown_method_and_service_are_refused() {
    let dir = tempfile::tempdir().expect("tmp");
    let tool = tool(&unique("itw2gm"), PROTO, dir.path());
    let backend = FakeBackend(Arc::new(|_, _| Ok("{}".to_string())));
    let (handle, addr, _sink) = serve(false, tool, backend, &[]);

    let pool = proto_pool(PROTO);
    let req = DynamicMessage::new(
        pool.get_message_by_name("demo.v1.PingRequest")
            .expect("req"),
    );
    let err = grpc_client_call(&addr, "/demo.v1.Greeter/Nope", req.clone(), false)
        .expect_err("unknown method");
    assert_eq!(err.code(), tonic::Code::Unimplemented, "{err}");
    let err =
        grpc_client_call(&addr, "/other.svc.Thing/Ping", req, false).expect_err("unknown service");
    assert_eq!(err.code(), tonic::Code::Unimplemented, "{err}");

    handle.join().expect("serve thread").expect("run ok");
}
