#[macro_use]
extern crate log;

use std::{
    collections::HashMap,
    time::{Duration, SystemTime}, fs::File, io::{BufReader, Read}, sync::Arc,
};

use bytes::{BufMut, BytesMut, Bytes, Buf};
use constants::{CLOSE, CONNECT, DATA, Global};
use env_logger::{Builder, Target};
use msg::{CoreMod, CoreMsg, Dst, NetMessage, UploadMsg};
use oss_http::Config;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    select,
    sync::mpsc::{channel, error::TryRecvError, Receiver, Sender},
    task,
    time::sleep,
};
use trust_dns_resolver::{AsyncResolver, name_server::{GenericConnection, GenericConnectionProvider, TokioRuntime}};

pub async fn send_channel_msg(msg: CoreMsg, tx: &Sender<CoreMsg>) -> bool {
    let id = msg.msg.id;
    let ope = msg.msg.ope;
    let dst = msg.dst.to_string();
    let res = tx.send(msg).await;
    match res {
        Ok(_) => {
            info!("send msg to core channel; id:{},ope:{},dst:{}", id, ope, dst);
            true
        }
        Err(_) => {
            error!(
                "send msg to core channel fail, core channel closed ?; id:{},ope:{},dst:{}",
                id, ope, dst
            );
            false
        }
    }
}

pub async fn send_upload_msg(msg: UploadMsg, tx: &Sender<UploadMsg>) -> bool {
    let res = tx.send(msg).await;
    match res {
        Ok(_) => {
            info!("send upload msg to oss channel ok");
            true
        }
        Err(_) => {
            error!("send upload msg to oss channel fail, oss channel closed ?");
            false
        }
    }
}

pub async fn send_net_msg(msg: NetMessage, tx: &Sender<NetMessage>) -> bool {
    let id = msg.id;
    let ope = msg.ope;
    let res = tx.send(msg).await;
    match res {
        Ok(_) => {
            debug!("send msg to client channel; id:{},ope:{}", id, ope);
            true
        }
        Err(_) => {
            error!(
                "send msg to client channel fail, client channel closed ?; id:{},ope:{}",
                id, ope
            );
            false
        }
    }
}

fn new_version() -> String {
    match SystemTime::now().duration_since(SystemTime::UNIX_EPOCH) {
        Ok(n) => {
            let seconds = n.as_secs();
            seconds.to_string()
        }
        Err(_) => {
            panic!("SystemTime before UNIX EPOCH!")
        }
    }
}

fn new_fname(prefix: &str, version: &str, file_id: &mut u32) -> String {
    let mut fname = String::from(prefix);
    fname.push_str(version);
    fname.push_str("_");
    fname.push_str(&file_id.to_string());
    fname.push_str(".css");
    *file_id = *file_id + 1;
    fname
}

pub struct OssUploader {
    o_tx: Sender<UploadMsg>,
    o_rx: Receiver<UploadMsg>,
    prefix: &'static str,
    version: String,
    file_id: u32,
    glo: Arc<Global>,
    upload_buffer: BytesMut,
}

impl OssUploader {
    pub fn new(
        glo: Arc<Global>,
        prefix: &'static str,
        o_tx: Sender<UploadMsg>,
        o_rx: Receiver<UploadMsg>
    ) -> OssUploader {
        let version = new_version();
        let file_id: u32 = 0;
        let upload_buffer = BytesMut::with_capacity(2 * 4096 * 1024);
        OssUploader {
            o_tx,
            o_rx,
            prefix,
            version,
            file_id,
            glo,
            upload_buffer
        }
    }

    pub async fn start(&mut self) {
        info!("oss uploader start ...");
        loop {
            let msg = self.o_rx.recv().await;
            if let Some(m) = msg {
                self.process_each_msg(m).await;
                loop {
                    let msg = self.o_rx.try_recv();
                    match msg {
                        Ok(m) => {
                            self.process_each_msg(m).await;
                        }
                        Err(e) => match e {
                            TryRecvError::Empty => {
                                break;
                            }
                            TryRecvError::Disconnected => {
                                error!("oss recv fail, disconnected");
                                return;
                            }
                        },
                    }
                }
            } else {
                error!("oss uploader channel recv fail. exit");
                return;
            }
        }
    }

    async fn process_each_msg(&mut self, m: UploadMsg) {
        match m {
            UploadMsg::Data { msg: m } => {
                info!("oss uploader process Data msg, upload buffer len: {} ...", self.upload_buffer.len());
                if self.upload_buffer.len() == 0 {
                    let o_tx = self.o_tx.clone();
                    task::spawn(OssUploader::schdule_commit(o_tx));
                }
                self.upload_buffer.put(m.encode());
            }
            UploadMsg::Commit => {
                info!("oss uploader process Commit msg, upload buffer len: {} ...", self.upload_buffer.len());
                let body = self.upload_buffer.copy_to_bytes(self.upload_buffer.len());
                self.upload_buffer.clear();
                let fname = new_fname(self.prefix, &self.version, &mut self.file_id);
                oss_http::put_file(&self.glo, &fname, body).await;
            }
        }
    }

    async fn schdule_commit(o_tx: Sender<UploadMsg>) {
        sleep(Duration::from_millis(100)).await;
        send_upload_msg(UploadMsg::Commit, &o_tx).await;
    }
}

pub struct OssDownloader {
    c_tx: Sender<CoreMsg>,
    glo: Arc<Global>,
    prefix: &'static str,
}

impl OssDownloader {
    pub fn new(c_tx: Sender<CoreMsg>, glo: Arc<Global> , prefix: &'static str) -> OssDownloader {
        OssDownloader { c_tx, glo, prefix }
    }

    pub async fn start(&self) {
        info!("oss download loop start...");
        let mut last_fname: Option<String> = None;
        loop {
            let files = oss_http::list_files(&self.glo, self.prefix).await.unwrap();
            for f in files {
                if !last_fname.is_none() {
                    let last_fname = last_fname.clone();
                    if let Some(last_fname) = last_fname {
                        let cmp = f.cmp(&last_fname);
                        match cmp {
                            std::cmp::Ordering::Less => {
                                oss_http::delete_file(&self.glo, &f).await;
                                continue;
                            },
                            std::cmp::Ordering::Equal => {
                                oss_http::delete_file(&self.glo, &f).await;
                                continue;
                            },
                            std::cmp::Ordering::Greater => {},
                        }
                    }
                }
                let mut body = oss_http::get_file(&self.glo, &f).await;
                info!("download from oss, {}, {}", &f, body.len());
                let msgs = NetMessage::decode(&mut body);
                for m in msgs {
                    info!("send download msg to core, id {}, len {}", m.id, m.data.len());
                    if !send_channel_msg(CoreMsg { dst: Dst::Client, msg: m, tx: None, }, &self.c_tx).await{
                        return;
                    }
                }
                oss_http::delete_file(&self.glo, &f).await;
                last_fname = Some(f);
            }
        }
    }
}

pub struct Core {
    c_tx: Sender<CoreMsg>,
    c_rx: Receiver<CoreMsg>,
    o_tx: Sender<UploadMsg>,
    txs: HashMap<u32, Sender<NetMessage>>,
    mode: CoreMod,
    reslover: Option<AsyncResolver<GenericConnection, GenericConnectionProvider<TokioRuntime>>>
}

impl Core {
    pub fn new(
        mode: CoreMod,
        c_tx: Sender<CoreMsg>,
        c_rx: Receiver<CoreMsg>,
        o_tx: Sender<UploadMsg>,
    ) -> Core {
        let txs = HashMap::new();
        let mut reslover = None;
        if let CoreMod::Remote = mode {
            reslover = Some(crate::dns::default_resolver());
        }
        Core {
            c_tx,
            c_rx,
            o_tx,
            txs,
            mode,
            reslover
        }
    }

    pub async fn start(&mut self) {
        info!(" Core start as {} ...", self.mode);
        loop {
            let msg = self.c_rx.recv().await;
            match msg {
                Some(m) => {
                    info!("core route msg to {}, len {}", m.dst, m.msg.data.len());
                    if let Dst::Oss = m.dst {
                        self.route_to_oss(m).await;
                        continue;
                    }
                    match self.mode {
                        CoreMod::Local => {
                            self.local_mode(m).await;
                        }
                        CoreMod::Remote => {
                            self.remote_mode(m).await;
                        }
                    }
                },
                None => {
                    error!("recv msg from c_rx channel fail");
                    break;
                }
            }
        }
    }

    async fn route_to_oss(&mut self, m: CoreMsg){
        let id = m.msg.id;
        let ope = m.msg.ope;
        if ope == CONNECT {
            if let Some(sender) = m.tx {
                self.txs.insert(id, sender);
            }
        } else if ope == CLOSE {
            self.txs.remove(&id);
        }
        send_upload_msg(UploadMsg::Data { msg: m.msg }, &self.o_tx).await;
    }

    async fn local_mode(&mut self, m: CoreMsg) {
        let id = m.msg.id;
        let ope = m.msg.ope;
        let val = self.txs.get(&id);

        if let Some(sender) = val {
            if !send_net_msg(m.msg, &sender).await {
                error!(
                    "send net msg fail, remove sender entry, clent recv close ?,id:{}",
                    &id
                );
                self.txs.remove(&id);
            }
            if ope == CLOSE {
                self.txs.remove(&id);
            }
        }
    }

    async fn remote_mode(&mut self, m: CoreMsg) {
        let net_msg = m.msg;
        let id = net_msg.id;
        let ope = net_msg.ope;

        if ope == CONNECT {
            let (tx, rx) = channel::<NetMessage>(4096);
            self.txs.insert(id, tx);
            let str = String::from_utf8(net_msg.data.to_vec()).unwrap();
            if let Some(reslover) = &self.reslover{
                task::spawn(client_loop(id, str, rx, self.c_tx.clone(), reslover.clone()));
            }
        } else if ope == DATA {
            let val = self.txs.get(&id);
            if let Some(sender) = val {
                send_net_msg(net_msg, sender).await;
            }
        } else if ope == CLOSE {
            self.txs.remove(&id);
        } else {
            error!("unknown ope {}", ope)
        }
    }
}

async fn client_loop(
    id: u32,
    host_port: String,
    mut rx: Receiver<NetMessage>,
    c_tx: Sender<CoreMsg>,
    reslover: AsyncResolver<GenericConnection, GenericConnectionProvider<TokioRuntime>>
) {
    let host_port = host_port.rsplit_once(':');
    if let Some((host, port)) = host_port {
        let dns_rsp = dns::dns_resolv(&reslover, host).await;
        if dns_rsp.is_none() {
            error!("dns reslove fail, host {}", host);
            send_channel_msg(CoreMsg{ dst: Dst::Oss, msg: NetMessage { id: id, ope: CLOSE, data: Bytes::new() }, tx: None }, &c_tx).await;
            return;
        }
        let ip = dns_rsp.unwrap().to_string();
        let conn_res = TcpStream::connect(ip + ":" + port).await;
        match conn_res {
            Ok(mut socket) => loop {
                let mut buffer = BytesMut::with_capacity(4096 * 1024);
                loop{
                    select! {
                        val = rx.recv() => {
                            match val {
                                Some(net_msg)=>{
                                    socket.write_all(&net_msg.data).await.unwrap();
                                }
                                None => {
                                    return;
                                }
                            }
                        }
                        res = socket.read_buf(&mut buffer) => {
                            match res{
                                Ok(len) => {
                                    info!("read data from socket, id {}, len {}", id, len);
                                    if len != 0{
                                        let net_msg = NetMessage{ id, ope: DATA, data: buffer.copy_to_bytes(buffer.len())};
                                        buffer.clear();
                                        send_channel_msg(CoreMsg{ dst: crate::msg::Dst::Oss, msg: net_msg, tx: None }, &c_tx).await;
                                    }else {
                                        send_channel_msg(CoreMsg{ dst: crate::msg::Dst::Oss, msg: NetMessage{ id, ope: CLOSE, data: Bytes::new()}, tx: None }, &c_tx).await;
                                        return;
                                    }
                                },
                                Err(e) => {
                                    error!("socket err due {}", &e);
                                    send_channel_msg(CoreMsg{ dst: crate::msg::Dst::Oss, msg: NetMessage{ id, ope: CLOSE, data: Bytes::new()}, tx: None }, &c_tx).await;
                                    return;
                                },
                            }
                        }
                    }
                }
            },
            Err(e) => {
                send_channel_msg(CoreMsg{ dst: Dst::Oss, msg: NetMessage { id: id, ope: CLOSE, data: Bytes::new() }, tx: None }, &c_tx).await;
                error!("socket connect err, id{}, {}", &id, e);
                return;
            },
        }
    }
}

pub fn init_logger() {
    let mut builder = Builder::from_default_env();
    builder.target(Target::Stdout);
    builder.init();
}


pub fn get_config() -> Config{
    let path = dirs::home_dir();
    if let Some(mut pb) = path{
        pb.push("config.json");
        warn!("read config file from path {}", String::from(pb.to_str().unwrap()));
        let file = File::open(pb).unwrap();
        let mut buf_reader = BufReader::new(file);
        let mut contents = String::new();
        buf_reader.read_to_string(&mut contents).unwrap();
        let c: Config = serde_json::from_str::<Config>(&contents).unwrap();
        return c;
    }
    panic!("get home dir fail");
}

pub mod constants;
pub mod dns;
pub mod msg;
pub mod oss_http;
