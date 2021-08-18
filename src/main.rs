use std::collections::HashMap;
use std::error::Error;
use futures::future::join_all;


use clap::{App, Arg};
use log::{debug, info, trace, warn, error, max_level};
use mqttrs::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::net::TcpListener;
use log::Level::Info;
use std::net::SocketAddr;
use std::hash::{Hash, Hasher};

struct ClientConnection {
    remote_addr: SocketAddr,
    stream: TcpStream,
}

impl PartialEq for ClientConnection {
    fn eq(&self, other: &Self) -> bool {
        self.stream.local_addr().unwrap() == other.stream.local_addr().unwrap()
    }
}

impl Hash for ClientConnection {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.stream.local_addr().unwrap().hash(state);
        self.remote_addr.hash(state);
    }
}

impl Eq for ClientConnection {}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let matches = App::new("mqtt2tcp")
        .version(option_env!("CARGO_PKG_VERSION").unwrap_or("unknown"))
        .about("Send MQTT message payload to connected clients")
        .arg(
            Arg::new("mqtt-host")
                .short('m')
                .value_name("HOST:PORT")
                .required(true)
                .about("Host and port of the MQTT host")
                .takes_value(true),
        )
        .arg(
            Arg::new("topic")
                .short('t')
                .value_name("TOPIC")
                .required(true)
                .about("The MQTT topic to subscribe to")
                .takes_value(true),
        )
        .arg(
            Arg::new("client-id")
                .short('c')
                .value_name("CLIENT-ID")
                .about("The MQTT client id")
                .default_value("mqtt2tcp")
                .takes_value(true),
        )
        .arg(
            Arg::new("listen-address")
                .short('p')
                .value_name("LISTEN-ADDRESS")
                .about("The address:port to listen on")
                .required(true)
                .default_value("0.0.0.0:12345")
                .takes_value(true),
        )
        .arg(
            Arg::new("verbose")
                .short('v')
                .multiple(true)
                .required(false)
                .takes_value(false)
                .about("Sets the level of verbosity (multiple times increases verbosity)"),
        )
        .get_matches();

    let host_and_port = matches.value_of("mqtt-host").unwrap();
    let mut mqtt_conn = TcpStream::connect(&host_and_port).await?;

    // TODO: make this configurable
    stderrlog::new()
        .module(module_path!())
        .quiet(false)
        .timestamp(stderrlog::Timestamp::Millisecond)
        .show_module_names(true)
        .verbosity(matches.occurrences_of("verbose") as usize)
        .init()
        .unwrap();

    let mut buf = [0u8; 1024];

    // Encode an MQTT Connect packet.
    let pkt = Packet::Connect(Connect {
        protocol: Protocol::MQTT311,
        keep_alive: 30,
        client_id: matches.value_of("client-id").unwrap(),
        clean_session: true,
        last_will: None,
        username: None,
        password: None,
    });

    let num_bytes = encode_slice(&pkt, &mut buf)?;
    mqtt_conn.write(&buf[..num_bytes]).await?;
    let num_read = mqtt_conn.read(&mut buf).await?;

    if let Some(pkt) = decode_slice(&buf[..num_read])? {
        match pkt {
            Packet::Connack(ack) => { trace!("Received connack {:?}", ack); }
            _ => {
                return Err(format!("Expected Connack, got {:?}", pkt).into());
            }
        }
    }

    let pid = Default::default();
    let pkt = Packet::Subscribe(Subscribe {
        pid,
        topics: vec![SubscribeTopic {
            topic_path: matches.value_of("topic").unwrap().to_string(),
            qos: QoS::AtMostOnce,
        }],
    });
    let num_bytes = encode_slice(&pkt, &mut buf)?;
    mqtt_conn.write(&buf[..num_bytes]).await?;
    let num_read = mqtt_conn.read(&mut buf).await?;
    if let Some(pkt) = decode_slice(&buf[..num_read])? {
        match pkt {
            Packet::Suback(ack) => { trace!("Received suback {:?}", ack); }
            _ => {
                return Err(format!("Expected Suback, got {:?}", pkt).into());
            }
        }
    }

    let _listener = TcpListener::bind(matches.value_of("listen-address").unwrap()).await.unwrap();

    // let mut clients = HashMap::new();
    let mut clients = HashMap::new();

    loop {
        tokio::select! {
            v = _listener.accept() => {
                let (conn, addr) = v.unwrap();
                debug!("Accepted connection: {:?}", conn);
                clients.insert(addr, Box::new(ClientConnection {
                    remote_addr: addr,
                    stream: conn,
                }));
            }
            v = mqtt_conn.read(&mut buf) => {
                let num_read = v.unwrap();
                match decode_slice(&buf[..num_read]) {
                    Ok(Some(pkt)) => {
                        match pkt {
                            Packet::Publish(msg) => {
                                if max_level() >= Info {
                                    let message = String::from_utf8_lossy(&msg.payload);
                                    info!("Received message:\n\"{}\"", message);
                                }

                                let mut futures = Vec::new();
                                // let mut written_to_clients = Vec::new();
                                for (addr, client_conn) in clients.iter_mut() {
                                    futures.push(client_conn.stream.write(&msg.payload));
                                    // TODO: Immutable borrow so failure
                                    // written_to_clients.push(&client_conn);
                                }

                                if !futures.is_empty() {
                                    let write_results = join_all(futures).await;
                                    for (pos, write_result) in write_results.iter().enumerate() {
                                        // let c = written_to_clients[pos].remote_addr;

                                        match write_result {
                                            Ok(v) => {
                                                // trace!("Write result to {:?}: {:?}", c, v);
                                                trace!("Write result to {:?}", v);
                                            }
                                            Err(e) => {
                                                // TODO: remove from hash as the client has likely disconnected.
                                                error!("Write failed: {:?}", e);
                                                // error!("Write to {:?} failed: {:?}", c, e);
                                            }
                                        }
                                    }
                                }

                                let pub_ack = Packet::Puback(pid);
                                let num_bytes = encode_slice(&pub_ack, &mut buf)?;
                                mqtt_conn.write(&buf[..num_bytes]).await?;
                            }
                            Packet::Pingreq => {
                                debug!("Received ping request");
                                let ping_response = Packet::Pingresp;
                                let num_bytes = encode_slice(&ping_response, &mut buf)?;
                                mqtt_conn.write(&buf[..num_bytes]).await?;
                            }
                            Packet::Pingresp => {
                                debug!("Received ping response");
                            }
                            _ => { trace!("Received packet: {:?}", pkt); }
                        }
                    }
                    Ok(None) => {
                        warn!("Probably didn't receive enough data (got {} bytes)", num_read)
                    }
                    other => panic!("Unexpected: {:?}", other)
                }
            }
        }
    }
}
