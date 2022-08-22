use std::env;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;

use bytes::{Buf, Bytes, BytesMut};
use common::constants::{Global, CLOSE, CONNECT, DATA, PREFIX_LOCAL};
use common::{send_channel_msg, Core, OssDownloader, OssUploader};
use fast_socks5::{consts, ReplyError};
use fast_socks5::{
    server::{Config, Incoming, Socks5Server, Socks5Socket},
    SocksError,
};
use log::info;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::select;
use tokio::sync::mpsc::{channel, Sender};
use tokio::{self, task};

use common::constants::PREFIX_REMOTE;
use common::msg::{CoreMod, CoreMsg, NetMessage, UploadMsg};
use common::oss_http::Config as Cfg;
use tokio_stream::StreamExt;
pub mod start_remote;

#[macro_use]
extern crate log;

pub type Result<T, E = SocksError> = core::result::Result<T, E>;

#[tokio::main]
pub async fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();
    //遍历打印参数索引、值
    for (index, value) in args.iter().enumerate() {
        eprintln!("{} => {}", index, value);
    }

    let config: Cfg = common::get_config();
    //ssh远程服务器启动远程进程
    start_remote::start_remote(&config);
    //启动本地
    let oss_client = common::oss_http::new_client();
    let glo_ref: Arc<Global> = Arc::new(Global {
        config: config,
        client: oss_client,
        core_mod: CoreMod::Local,
    });
    common::init_logger();
    info!(
        "Starting proxy at local side, bucket {} ...",
        &glo_ref.config.ucloudOssBucketName
    );

    common::oss_http::delete_all(&glo_ref).await;

    let (o_tx, o_rx) = channel::<UploadMsg>(4096);
    let (c_tx, c_rx) = channel::<CoreMsg>(4096);

    let downloader = OssDownloader::new(c_tx.clone(), Arc::clone(&glo_ref), PREFIX_REMOTE);
    task::spawn(async move {
        downloader.start().await;
    });

    let mut uploader = OssUploader::new(Arc::clone(&glo_ref), PREFIX_LOCAL, o_tx.clone(), o_rx);
    task::spawn(async move {
        uploader.start().await;
    });

    let mut core = Core::new(CoreMod::Local, c_tx.clone(), c_rx, o_tx.clone());
    task::spawn(async move {
        core.start().await;
    });

    start_socks5_server(&glo_ref.config.localSocksPort, c_tx).await;

    Ok(())
}

async fn start_socks5_server(socks_port: &str, c_tx: Sender<CoreMsg>) {
    let mut listener = Socks5Server::bind(String::from("0.0.0.0:") + socks_port)
        .await
        .unwrap();
    info!("bind port to {}", socks_port);
    listener.set_config(def_config());

    let mut incoming: Incoming = listener.incoming();
    info!("Listen for socks connections @ {}", socks_port);
    let mut id_seq: u32 = 0;

    while let Some(socket_res) = incoming.next().await {
        match socket_res {
            Ok(socket) => {
                let c_tx = c_tx.clone();
                let id = id_seq;
                id_seq = id_seq + 1;
                info!("Accept new socks5 conn,id: {}", &id);
                task::spawn(process_each(socket, id, c_tx));
            }
            Err(err) => {
                error!("accept error = {:?}", err);
            }
        }
    }
}

async fn process_each(socket: Socks5Socket<TcpStream>, id: u32, c_tx: Sender<CoreMsg>) {
    let fut = socket.upgrade_to_socks5().await;
    match fut {
        Ok(mut socks5_socket) => {
            if let Some(t_addr) = socks5_socket.target_addr() {
                let (tx, mut rx) = channel::<NetMessage>(4096);
                let t_addr_str = common::msg::socks_addr_to_string(t_addr);
                let net_msg = NetMessage {
                    id,
                    ope: CONNECT,
                    data: Bytes::from(t_addr_str.clone()),
                };
                info!(
                    "socks5 conn to dst: {}, len: {}",
                    &t_addr_str,
                    net_msg.data.len()
                );
                if !send_channel_msg(
                    CoreMsg {
                        dst: common::msg::Dst::Oss,
                        msg: net_msg,
                        tx: Some(tx),
                    },
                    &c_tx,
                )
                .await
                {
                    return;
                }
                write_connect_ok(&mut socks5_socket).await;
                let mut buffer = BytesMut::with_capacity(4096);
                loop {
                    select! {
                        val = rx.recv() => {
                            if let Some(mut data) = val{
                                info!("socks5 loop recv msg from channel, id {}, ope {}, len {}", data.id, data.ope, data.data.len());
                                if data.ope == CLOSE{
                                    info!("socks5 loop end by close msg,id {}", &id);
                                    return;
                                }
                                let res = socks5_socket.write_all_buf(&mut data.data).await;
                                if let Err(e) = res{
                                    error!("socks5 write err id:{}, {}, close...", &id, &e);
                                    return;
                                }
                            }else {
                                return;
                            }
                        }
                        res = socks5_socket.read_buf(&mut buffer) => {
                            match res {
                                Ok(len) => {
                                    info!("recv bytes from socks5 socket, len: {}", len);
                                    if len == 0 {
                                        info!("socks5 socket closed by return 0, release all...");
                                        return;
                                    }
                                    let net_msg = NetMessage{ id, ope: DATA, data: buffer.copy_to_bytes(buffer.len())};
                                    buffer.clear();
                                    if !send_channel_msg(CoreMsg{ dst: common::msg::Dst::Oss, msg: net_msg, tx: None }, &c_tx).await{
                                        return;
                                    }
                                },
                                Err(e) => {
                                    error!("socks5 socker err, {}", e);
                                    return;
                                }
                            }
                        }
                    }
                }
            } else {
                error!("parse socks5 targrt addr fail, id: {}", &id);
                return;
            }
        }
        Err(e) => {
            error!("upgrade to socks5 socket fail, {}", &e);
            return;
        }
    }
}

async fn write_connect_ok(socket: &mut Socks5Socket<TcpStream>) {
    socket
        .write(&new_reply(
            &ReplyError::Succeeded,
            SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0),
        ))
        .await
        .unwrap();
    socket.flush().await.unwrap();
    debug!("Wrote connect rsp success");
}

fn def_config() -> Config {
    let mut config = Config::default();
    config.set_request_timeout(60);
    config.set_skip_auth(false);
    config.set_dns_resolve(false);
    config.set_execute_command(false);
    config.set_udp_support(false);
    config
}
fn new_reply(error: &ReplyError, sock_addr: SocketAddr) -> Vec<u8> {
    let (addr_type, mut ip_oct, mut port) = match sock_addr {
        SocketAddr::V4(sock) => (
            consts::SOCKS5_ADDR_TYPE_IPV4,
            sock.ip().octets().to_vec(),
            sock.port().to_be_bytes().to_vec(),
        ),
        SocketAddr::V6(sock) => (
            consts::SOCKS5_ADDR_TYPE_IPV6,
            sock.ip().octets().to_vec(),
            sock.port().to_be_bytes().to_vec(),
        ),
    };

    let mut reply = vec![
        consts::SOCKS5_VERSION,
        error.as_u8(), // transform the error into byte code
        0x00,          // reserved
        addr_type,     // address type (ipv4, v6, domain)
    ];
    reply.append(&mut ip_oct);
    reply.append(&mut port);

    reply
}
