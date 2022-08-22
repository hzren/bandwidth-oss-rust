use std::{fmt::Display};

use bytes::{Buf, BufMut, Bytes, BytesMut};
use fast_socks5::util::target_addr::TargetAddr;
use tokio::sync::mpsc::Sender;

pub enum CoreMod {
    Local,
    Remote,
}

impl std::fmt::Display for CoreMod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CoreMod::Local => {
                write!(f, " Local ")
            }
            CoreMod::Remote => {
                write!(f, " Remote ")
            }
        }
    }
}

pub enum Dst {
    Client,
    Oss,
}

impl Display for Dst {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Dst::Client => {
                write!(f, " Client ")
            }
            Dst::Oss => {
                write!(f, " Oss ")
            }
        }
    }
}

pub struct NetMessage {
    pub id: u32,
    pub ope: u8,
    pub data: Bytes,
}

pub enum UploadMsg {
    Data { msg: NetMessage },
    Commit,
}

pub struct CoreMsg {
    pub dst: Dst,
    pub msg: NetMessage,
    pub tx: Option<Sender<NetMessage>>,
}

impl NetMessage {
    pub fn encode_len(&self) -> u32 {
        (self.data.len() + 4 + 1).try_into().unwrap()
    }

    pub fn encode(&self) -> Bytes {
        let len = self.encode_len();
        let mut res = BytesMut::with_capacity((len + 4).try_into().unwrap());
        res.put_u32(len);
        res.put_u32(self.id);
        res.put_u8(self.ope);
        res.put_slice(&self.data);
        res.freeze()
    }

    pub fn decode(bytes: &mut Bytes) -> Vec<NetMessage> {
        let mut res: Vec<NetMessage> = Vec::new();
        while bytes.remaining() > 0 {
            let len = bytes.get_u32();
            let id = bytes.get_u32();
            let ope = bytes.get_u8();
            let target_len = len - 4 - 1;
            let data = bytes.copy_to_bytes(target_len.try_into().unwrap());
            res.push(NetMessage { id, ope, data });
        }
        res
    }
}

pub fn socks_addr_to_string(addr: &TargetAddr) -> String {
    match addr {
        TargetAddr::Ip(socket_addr) => socket_addr.to_string(),
        TargetAddr::Domain(domain, port) => {
            format!("{}:{}", domain, port).to_ascii_lowercase()
        }
    }
}
