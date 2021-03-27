#![feature(never_type)]

use tonic::{transport::Server, Request, Response, Status};

use std::time::{Duration, Instant};

use elayday::elayday_server::{Elayday, ElaydayServer};
use elayday::{
    frame, Fragment, Frame, FrameType, GetValueRequest, GetValueResponse, PutValueRequest,
    PutValueResponse, Value,
};

use std::collections::HashMap;

use std::fmt::{Display, Formatter};

use bytes::BytesMut;
use clap::{App, AppSettings, Arg};
use futures::pin_mut;
use futures::SinkExt;
use prost::Message;
use std::net::SocketAddr;
use tokio::net::UdpSocket;
use tokio::stream::StreamExt;
use tokio::sync::mpsc::{channel, unbounded_channel, Receiver, Sender};
use tokio::sync::oneshot;
use tokio_util::codec::{BytesCodec, Decoder, Encoder};
use tokio_util::udp::UdpFramed;

pub mod elayday {
    tonic::include_proto!("elayday");

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

struct FrameCodec {}

impl FrameCodec {
    fn new() -> Self {
        Self {}
    }
}

#[derive(Debug)]
pub enum ElaydayError {
    IOError(std::io::Error),
    EncodeError(prost::EncodeError),
    DecodeError(prost::DecodeError),
}

impl Display for ElaydayError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            ElaydayError::IOError(status) => write!(f, "{}", status),
            ElaydayError::EncodeError(status) => write!(f, "{}", status),
            ElaydayError::DecodeError(status) => write!(f, "{}", status),
        }
    }
}

impl std::error::Error for ElaydayError {
    fn cause(&self) -> Option<&dyn std::error::Error> {
        match self {
            ElaydayError::IOError(ref err) => Some(err),
            ElaydayError::EncodeError(ref err) => Some(err),
            ElaydayError::DecodeError(ref err) => Some(err),
        }
    }
}

impl From<std::io::Error> for ElaydayError {
    fn from(e: std::io::Error) -> ElaydayError {
        ElaydayError::IOError(e)
    }
}

impl From<prost::EncodeError> for ElaydayError {
    fn from(e: prost::EncodeError) -> ElaydayError {
        ElaydayError::EncodeError(e)
    }
}

impl From<prost::DecodeError> for ElaydayError {
    fn from(e: prost::DecodeError) -> ElaydayError {
        ElaydayError::DecodeError(e)
    }
}

impl Encoder<Frame> for FrameCodec {
    type Error = ElaydayError;

    fn encode(&mut self, item: Frame, dst: &mut BytesMut) -> Result<(), Self::Error> {
        item.encode(dst)?;
        Ok(())
    }
}

impl Decoder for FrameCodec {
    type Item = Frame;
    type Error = ElaydayError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        Ok(Some(Frame::decode(src)?))
    }
}

pub struct ElaydayService {
    mailbox: Sender<ApiMessage>,
}

#[tonic::async_trait]
impl Elayday for ElaydayService {
    async fn put_value(
        &self,
        request: Request<PutValueRequest>,
    ) -> Result<Response<PutValueResponse>, Status> {
        let request = request.into_inner();
        let (tx, rx) = oneshot::channel();
        self.mailbox
            .clone()
            .send(ApiMessage::Put(request.key, request.value, tx))
            .await;

        rx.await.unwrap();

        Ok(Response::new(PutValueResponse {}))
    }

    async fn get_value(
        &self,
        request: Request<GetValueRequest>,
    ) -> Result<Response<GetValueResponse>, Status> {
        let request = request.into_inner();
        let (tx, rx) = oneshot::channel();
        self.mailbox
            .clone()
            .send(ApiMessage::Get(request.key, tx))
            .await;

        Ok(Response::new(GetValueResponse {
            value: rx.await.unwrap(),
        }))
    }
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
            bind_address: bind_address,
            destination_address: destination_address,
            state: ProcessorState::new(),
            mtu: 32767,
        }
    }

    fn with_mtu(&mut self, mtu: usize) -> &mut Self {
        self.mtu = mtu;
        self
    }

    fn process_message(&mut self, message: ApiMessage) -> Vec<Frame> {
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
                                        reply: reply,
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
                                    claim.reply.send(response);
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
                        key: key,
                        value_id: Some(value_id.into()),
                        num_fragments: num_fragments,
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
                    .insert(ping_id, (self.state.frame_count, Instant::now()));

                outbox.push(Frame {
                    r#type: FrameType::Ping as i32,
                    payload: Some(frame::Payload::PingId(ping_id.into())),
                });
            }
        }

        outbox
    }

    async fn run(&mut self, mailbox: Receiver<ApiMessage>) -> Result<!, ElaydayError> {
        let socket = UdpSocket::bind(&self.bind_address).await?;
        let (mut udp_write, udp_read) =
            futures::stream::StreamExt::split(UdpFramed::new(socket, FrameCodec::new()));
        let incoming = udp_read.filter_map(|x| match x {
            Ok((frame, _)) => Some(ApiMessage::Frame(frame)),
            Err(e) => {
                println!("Malformed message received {:?}", e);
                None
            }
        });
        let heartbeat = tokio::time::interval(Duration::from_millis(1000))
            .map(|x| ApiMessage::Heartbeat(x.into()));

        let stream = incoming.merge(mailbox).merge(heartbeat);
        pin_mut!(stream);

        loop {
            match futures::stream::StreamExt::next(&mut stream).await {
                Some(api_message) => {
                    for frame in self.process_message(api_message) {
                        //tokio::time::delay_for(Duration::from_millis(500)).await;
                        udp_write.send((frame, self.destination_address)).await?;
                    }
                }
                _ => {}
            };
        }
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

    let elayday_service = ElaydayService { mailbox: tx };

    let server = tokio::spawn(async move {
        Server::builder()
            .add_service(ElaydayServer::new(elayday_service))
            .serve(api_address)
            .await
    });

    processor.await??;
    server.await??;
}

async fn run_reflector(
    bind_address: SocketAddr,
    delay: std::time::Duration,
) -> Result<(), Box<dyn std::error::Error>> {
    let (mut tx, mut rx) = unbounded_channel();

    let socket = UdpSocket::bind(bind_address).await?;
    let (mut udp_tx, mut udp_rx) =
        futures::stream::StreamExt::split(UdpFramed::new(socket, BytesCodec::new()));

    let writer_task = tokio::spawn(async move {
        while let Some((execute_at, buf, return_address)) = rx.next().await {
            tokio::time::delay_until(execute_at).await;
            udp_tx.send((buf, return_address)).await.unwrap();
        }
    });

    while let Some(Ok((buf, return_address))) = udp_rx.next().await {
        let execute_at = tokio::time::Instant::now() + delay;
        tx.send((execute_at, buf.into(), return_address))
            .unwrap();
    }
    drop(tx);

    writer_task.await?;

    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let matches = App::new("elayday")
        .version("1.0")
        .author("Tibor Djurica Potpara <tibor.djurica@ojdip.net>")
        .about("Does awesome things")
        .subcommand(
            App::new("server") // The name we call argument with
                .about("Runs the server") // The message displayed in "myapp -h"
                .arg(
                    Arg::with_name("bind")
                        .long("bind")
                        .about("Endpoint to bind the UDP socket to")
                        .default_value("[::1]:24601"),
                )
                .arg(
                    Arg::with_name("destination")
                        .long("destination")
                        .about("Where to send the packets")
                        .default_value("[::1]:24601"),
                )
                .arg(
                    Arg::with_name("bind-grpc")
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
                    Arg::with_name("bind")
                        .long("bind")
                        .about("Endpoint to bind the UDP socket to")
                        .default_value("[::1]:24601"),
                )
                .arg(
                    Arg::with_name("delay")
                        .long("delay")
                        .about("How much to delay the packets")
                        .default_value("0"),
                ),
        )
        .get_matches();

    if let Some(ref matches) = matches.subcommand_matches("server") {
        run_server(
            matches.value_of("bind").unwrap().parse()?,
            matches.value_of("destination").unwrap().parse()?,
            matches.value_of("bind-grpc").unwrap().parse()?,
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
