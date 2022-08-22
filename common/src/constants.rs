
use reqwest::Client;

use crate::{oss_http::Config, msg::CoreMod};

pub const CONTENT_TYPE: &str = "Content-Type";
pub const JSON_UTF8: &str = "application/json; charset=utf-8";
pub const BYTE_STREAM: &str = "application/octet-stream";

pub const CONNECT: u8 = 1;
pub const DATA: u8 = 2;
pub const CLOSE: u8 = 3;

pub const PREFIX_LOCAL: &str = "c_";
pub const PREFIX_REMOTE: &str = "s_";


pub struct Global{
    pub config: Config,
    pub client: Client,
    pub core_mod: CoreMod,
}

impl Global  {

    pub fn oss_host(&self) -> String{
        match self.core_mod {
            CoreMod::Local => {
                String::from("http://") + &self.config.ucloudOssPublicHost
            },
            CoreMod::Remote => {
                String::from("http://") + &self.config.ucloudOssPrivateHost
            },
        }
    }
    
}
