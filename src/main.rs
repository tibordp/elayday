#![feature(never_type)]
#![feature(async_closure)]

pub mod api;
pub mod codec;
pub mod error;

use tonic::transport::Server;

use std::time::{Duration, Instant};

use elayday::elayday_server::ElaydayServer;
use elayday::{frame, Fragment, Frame, FrameType, Value};

use std::collections::HashMap;

use clap::{App, AppSettings, Arg};
use futures::pin_mut;
use futures::SinkExt;
use prost::Message;
use std::net::SocketAddr;
use tokio::net::UdpSocket;
use tokio::sync::mpsc::{channel, Receiver};
use tokio::sync::oneshot;
use tokio_stream::StreamExt;
use tokio_util::codec::BytesCodec;
use tokio_util::udp::UdpFramed;

use crate::api::ElaydayService;
use crate::codec::FrameCodec;
use crate::error::ElaydayError;

pub mod elayday {
    tonic::include_proto!("elayday");

    pub(crate) const FILE_DESCRIPTOR_SET: &'static [u8] =
        tonic::include_file_descriptor_set!("elayday_descriptor");

    impl From<uuid::Uuid> for Uuid {
        fn from(uuid: uuid::Uuid) -> Self {
            Uuid {
                uuid: uuid.to_simple().to_string(),
            }
        }
    }

    impl From<Uuid> for uuid::Uuid {
        fn from(uuid: Uuid) -> Self {
            uuid::Uuid::parse_str(&uuid.uuid).unwrap()
        }
    }
}

#[derive(Debug)]
pub enum ApiMessage {
    Frame(Frame),
    Heartbeat(Instant),
    Put(String, Vec<u8>, oneshot::Sender<()>),
    Get(String, oneshot::Sender<Vec<u8>>),
}

pub enum ApiResponse {
    GetResponse(Vec<u8>),
}

struct GetClaim {
    fragments: Vec<Option<Vec<u8>>>,
    reply: oneshot::Sender<Vec<u8>>,
}

struct ProcessorState {
    frame_count: u64,
    key_lookup: HashMap<String, oneshot::Sender<Vec<u8>>>,
    ping_lookup: HashMap<uuid::Uuid, (u64, Instant)>,
    fragment_lookup: HashMap<uuid::Uuid, GetClaim>,
}

impl ProcessorState {
    fn new() -> Self {
        ProcessorState {
            frame_count: 0,
            ping_lookup: HashMap::new(),
            key_lookup: HashMap::new(),
            fragment_lookup: HashMap::new(),
        }
    }
}

struct Processor {
    bind_address: SocketAddr,
    destination_address: SocketAddr,
    state: ProcessorState,
    mtu: usize,
}

impl Processor {
    fn new(bind_address: SocketAddr, destination_address: SocketAddr) -> Self {
        Processor {
            bind_address,
            destination_address,
            state: ProcessorState::new(),
            mtu: 32767,
        }
    }

    fn with_mtu(&mut self, mtu: usize) -> &mut Self {
        self.mtu = mtu;
        self
    }

    fn process_message(&mut self, message: ApiMessage) -> Option<Vec<Frame>> {
        let mut outbox = Vec::new();

        match message {
            ApiMessage::Frame(frame) => {
                let original_frame = frame.clone();

                match FrameType::from_i32(frame.r#type) {
                    Some(FrameType::Ping) => {
                        if let Some(frame::Payload::PingId(val)) = frame.payload {
                            match self.state.ping_lookup.remove(&val.into()) {
                                Some((frame_count, ping_start)) => {
                                    println!(
                                        "Received pong, approximate message count={:?}, rtt={:?}",
                                        self.state.frame_count - frame_count,
                                        Instant::now() - ping_start
                                    );
                                }
                                None => {
                                    outbox.push(original_frame);
                                }
                            }
                        }
                    }
                    Some(FrameType::Value) => {
                        if let Some(frame::Payload::Value(val)) = frame.payload {
                            outbox.push(original_frame);
                            if let Some(reply) = self.state.key_lookup.remove(&val.key) {
                                self.state.fragment_lookup.insert(
                                    val.value_id.unwrap().into(),
                                    GetClaim {
                                        fragments: vec![None; val.num_fragments as usize],
                                        reply,
                                    },
                                );
                            }
                        }
                    }
                    Some(FrameType::Fragment) => {
                        use std::collections::hash_map::Entry;

                        if let Some(frame::Payload::Fragment(val)) = frame.payload {
                            outbox.push(original_frame);
                            if let Entry::Occupied(mut entry) = self
                                .state
                                .fragment_lookup
                                .entry(val.value_id.unwrap().into())
                            {
                                let claim = entry.get_mut();
                                claim.fragments[val.sequence_num as usize] = Some(val.value);
                                if claim.fragments.iter().all(|v| v.is_some()) {
                                    let (_, claim) = entry.remove_entry();
                                    let response = claim
                                        .fragments
                                        .into_iter()
                                        .flat_map(|x| x.unwrap())
                                        .collect();
                                    println!("Ahoj!");
                                    claim.reply.send(response).unwrap();
                                }
                            }
                        }
                    }
                    _ => {}
                }
                self.state.frame_count += 1;
            }
            ApiMessage::Put(key, value, reply) => {
                let mut num_fragments = 0;
                let value_id = uuid::Uuid::new_v4();
                println!("PUT {:?}, {:?}", key, value);
                for (sequence_num, chunk) in value.chunks(self.mtu).enumerate() {
                    outbox.push(Frame {
                        r#type: FrameType::Fragment as i32,
                        payload: Some(frame::Payload::Fragment(Fragment {
                            value_id: Some(value_id.into()),
                            sequence_num: sequence_num as u64,
                            value: chunk.to_vec(),
                        })),
                    });
                    num_fragments += 1;
                }
                outbox.push(Frame {
                    r#type: FrameType::Value as i32,
                    payload: Some(frame::Payload::Value(Value {
                        key,
                        value_id: Some(value_id.into()),
                        num_fragments,
                    })),
                });
                reply.send(()).unwrap();
            }
            ApiMessage::Get(key, reply) => {
                println!("GET {:?}", key);
                self.state.key_lookup.insert(key, reply);
            }
            ApiMessage::Heartbeat(instant) => {
                let ping_id = uuid::Uuid::new_v4();
                self.state
                    .ping_lookup
                    .insert(ping_id, (self.state.frame_count, instant));

                outbox.push(Frame {
                    r#type: FrameType::Ping as i32,
                    payload: Some(frame::Payload::PingId(ping_id.into())),
                });
            }
        }

        Some(outbox)
    }

    async fn run(&mut self, mailbox: Receiver<ApiMessage>) -> Result<(), ElaydayError> {
        let socket = UdpSocket::bind(&self.bind_address).await?;
        let (mut udp_write, udp_read) =
            futures::stream::StreamExt::split(UdpFramed::new(socket, FrameCodec::new()));

        let incoming = udp_read.filter_map(move |x| match x {
            Ok((frame, _)) => Some(ApiMessage::Frame(frame)),
            Err(e) => {
                println!("Malformed message received {:?}", e);
                None
            }
        });
        let heartbeat = tokio_stream::wrappers::IntervalStream::new(tokio::time::interval(
            Duration::from_millis(1000),
        ))
        .map(|x| ApiMessage::Heartbeat(x.into()));

        let stream = incoming
            .merge(tokio_stream::wrappers::ReceiverStream::new(mailbox))
            .merge(heartbeat);

        pin_mut!(stream);

        loop {
            if let Some(api_message) = stream.next().await {
                match self.process_message(api_message) {
                    Some(frames) => {
                        for frame in frames {
                            udp_write.send((frame, self.destination_address)).await?;
                        }
                    }
                    None => {
                        break;
                    }
                }
            }
        }

        Ok(())
    }
}

async fn run_server(
    bind_address: SocketAddr,
    destination_address: SocketAddr,
    api_address: SocketAddr,
) -> Result<(), Box<dyn std::error::Error>> {
    let (tx, rx) = channel(1);

    let processor = tokio::spawn(async move {
        Processor::new(bind_address, destination_address)
            .with_mtu(1000)
            .run(rx)
            .await
    });

    let elayday_service = ElaydayService::new(tx);

    let server = tokio::spawn(async move {
        let (mut health_reporter, health_service) = tonic_health::server::health_reporter();
        health_reporter
            .set_serving::<ElaydayServer<ElaydayService>>()
            .await;

        let reflection_service = tonic_reflection::server::Builder::configure()
            .register_encoded_file_descriptor_set(elayday::FILE_DESCRIPTOR_SET)
            .build()
            .unwrap();

        Server::builder()
            .add_service(reflection_service)
            .add_service(health_service)
            .add_service(ElaydayServer::new(elayday_service))
            .serve(api_address)
            .await
    });

    processor.await??;
    server.await??;

    Ok(())
}

async fn run_reflector(
    bind_address: SocketAddr,
    delay: std::time::Duration,
) -> Result<(), Box<dyn std::error::Error>> {
    let (mut tx, mut rx) = futures::channel::mpsc::unbounded();

    let socket = UdpSocket::bind(bind_address).await?;
    let (mut udp_tx, mut udp_rx) =
        futures::stream::StreamExt::split(UdpFramed::new(socket, BytesCodec::new()));

    let writer_task = tokio::spawn(async move {
        while let Some((execute_at, buf, return_address)) = rx.next().await {
            tokio::time::sleep_until(execute_at).await;
            udp_tx.send((buf, return_address)).await?;
        }
        Ok::<(), std::io::Error>(())
    });

    while let Some(Ok((buf, return_address))) = udp_rx.next().await {
        let execute_at = tokio::time::Instant::now() + delay;
        tx.send((execute_at, buf.into(), return_address)).await?;
    }
    drop(tx);

    writer_task.await??;

    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let matches = App::new("elayday")
        .version("1.0")
        .author("Tibor Djurica Potpara <tibor.djurica@ojdip.net>")
        .about("A UDP delay line key-value datastore")
        .setting(AppSettings::ArgRequiredElseHelp)
        .subcommand(
            App::new("server") // The name we call argument with
                .about("Runs the server") // The message displayed in "myapp -h"
                .arg(
                    Arg::new("bind")
                        .long("bind")
                        .about("Endpoint to bind the UDP socket to")
                        .default_value("[::1]:24601"),
                )
                .arg(
                    Arg::new("destination")
                        .long("destination")
                        .about("Where to send the packets")
                        .default_value("[::1]:24601"),
                )
                .arg(
                    Arg::new("bind-grpc")
                        .long("bind-grpc")
                        .about("TCP endpoint where to bind the gRPC server")
                        .default_value("[::1]:24602"),
                ),
        )
        .subcommand(
            App::new("reflector") // The name we call argument with
                .setting(AppSettings::ArgRequiredElseHelp)
                .about("Serves as a dumb reflector") // The message displayed in "myapp -h"
                .arg(
                    Arg::new("bind")
                        .long("bind")
                        .about("Endpoint to bind the UDP socket to")
                        .default_value("[::1]:24601"),
                )
                .arg(
                    Arg::new("delay")
                        .long("delay")
                        .about("How much to delay the packets")
                        .default_value("0"),
                ),
        )
        .get_matches();

    if let Some(ref matches) = matches.subcommand_matches("server") {
        run_server(
            matches.value_of("bind").unwrap().parse().unwrap(),
            match tokio::net::lookup_host(matches.value_of("destination").unwrap())
                .await?
                .next()
            {
                Some(addr) => addr,
                None => panic!(),
            },
            matches.value_of("bind-grpc").unwrap().parse().unwrap(),
        )
        .await?;
    }

    if let Some(ref matches) = matches.subcommand_matches("reflector") {
        run_reflector(
            matches.value_of("bind").unwrap().parse()?,
            Duration::from_secs_f64(matches.value_of("delay").unwrap().parse()?),
        )
        .await?;
    }
    Ok(())
}
