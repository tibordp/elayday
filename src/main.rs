#![feature(never_type)]
#![feature(async_closure)]

pub mod api;
pub mod codec;
pub mod error;
pub mod processor;

use tonic::transport::Server;

use std::time::{Duration, Instant};

use elayday::elayday_server::ElaydayServer;
use elayday::{frame, Fragment, Frame, FrameType, Value};

use std::collections::HashMap;

use clap::{App, AppSettings, Arg};
use futures::pin_mut;
use futures::SinkExt;
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
use crate::processor::Processor;

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
