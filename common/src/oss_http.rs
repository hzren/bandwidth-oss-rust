use crate::constants::*;
use base64::encode;
use bytes::Bytes;
use chrono::prelude::*;
use hmac::{Hmac, Mac};
use reqwest::{Method, Client};
use serde::{Deserialize, Serialize};
use serde_json;
use sha1::Sha1;
use std::fmt::Debug;

type HmacSha1 = Hmac<Sha1>;

#[allow(non_snake_case)]
#[derive(Serialize, Deserialize)]
pub struct Config{
    pub localSocksPort: String,
    pub localHttpPort: String,
    pub ucloudOssPublicKey: String,
    pub ucloudOssPrivateKey: String,
    pub ucloudOssBucketName: String,
    pub ucloudOssPublicHost: String,
    pub ucloudOssPrivateHost: String,
    pub dnsList: String,
    pub serverHost: String,
    pub serverUser: String,
    pub serverPasswd: String,
}

impl Config {

    pub fn auth_token(&self, key_name: &str, date: &str, method: Method) -> String {
        let mut src = String::from(method.as_str());
        src.push_str("\n");
        src.push_str("\n");
        src.push_str(JSON_UTF8);
        src.push_str("\n");
        src.push_str(date);
        src.push_str("\n");
        src.push_str("/");
        src.push_str(&self.ucloudOssBucketName);
        src.push_str("/");
        src.push_str(key_name);
    
        let mut mac =
            HmacSha1::new_from_slice(self.ucloudOssPrivateKey.as_bytes()).expect("This should not happen");
        mac.update(src.as_bytes());
        let result = mac.finalize();
        "UCloud ".to_owned() + &self.ucloudOssPublicKey + ":" + encode(result.into_bytes()).as_str()
    }
    
}

// {
//     "BucketName": "blue",
//     "BucketId": "ufile-qs20fr",
//     "NextMarker": "",
//     "DataSet": [
//         {
//             "BucketName": "blue",
//             "FileName": "aaa.jpg",
//             "Hash": "fbfc1aba39fdac0e0f298461970529d3",
//             "MimeType": "image/jpeg",
//             "Size": 344500,
//             "CreateTime": 1408298579,
//             "ModifyTime": 1408298579
//        }
//     ]
// }

#[allow(non_snake_case)]
#[derive(Serialize, Deserialize, Debug)]
pub struct ListedFile {
    #[warn(non_snake_case)]
    BucketName: String,
    FileName: String,
    Hash: String,
    MimeType: String,
    Size: i32,
    CreateTime: i64,
    ModifyTime: i64,
}
#[allow(non_snake_case)]
#[derive(Serialize, Deserialize, Debug)]
pub struct ListFileRsp {
    BucketName: String,
    BucketId: String,
    NextMarker: String,
    DataSet: Vec<ListedFile>,
}

pub fn get_now_date_str() -> String {
    Local::now().format("%Y%m%d%H%M%S").to_string()
}

// GET /?list&prefix=<prefix>&marker=<marker>&limit=<limit>
// Host: <bucket_name>.ufile.ucloud.cn
// Authorization: <token>
pub async fn list_files(glo: &Global, prefix: &str) -> Result<Vec<String>, reqwest::Error> {
    let mut res: Vec<String> = Vec::new();
    let mut marker = String::from("");
    let limit = "200";
    loop {
        let date = get_now_date_str();
        let mut url = String::from(glo.oss_host());
        url.push_str("/?list&");
        let rsp = glo.client.clone()
            .get(url)
            .header("Content-Type", JSON_UTF8)
            .header("Date", &date)
            .header("authorization", glo.config.auth_token("", &date, Method::GET))
            .header("Accpet", "*/*")
            .query(&[
                ("prefix", prefix),
                ("marker", &marker.clone()),
                ("limit", &limit),
            ])
            .send()
            .await?
            .text()
            .await?;
        debug!("Oss list rsp : {}", &rsp);
        let lfr: ListFileRsp = serde_json::from_str(&rsp).unwrap();
        for f in lfr.DataSet {
            res.push(f.FileName.clone());
        }
        marker = lfr.NextMarker.clone();

        if marker.is_empty() {
            break;
        }
    }
    Ok(res)
}

pub async fn put_file(glo: &Global, key_name: &str, bytes: Bytes) {
    let date = get_now_date_str();
    let mut url = String::from(glo.oss_host());
    url.push_str("/");
    url.push_str(key_name);
    loop {
        let rsp = glo.client.clone()
            .put(&url)
            .header("Content-Type", JSON_UTF8)
            .header("Date", &date)
            .header("authorization", glo.config.auth_token(key_name, &date, Method::PUT))
            .header("Accpet", "*/*")
            .body(reqwest::Body::from(bytes.clone()))
            .send()
            .await;
        if let Ok(_r) = rsp {
            info!("upload file to oss, fnmae: {}", key_name);
            return;
        }
    }
}

pub async fn delete_file(glo: &Global, key_name: &str) {
    let date = get_now_date_str();
    let mut url = String::from(glo.oss_host());
    url.push_str("/");
    url.push_str(key_name);
    let rsp = glo.client.clone()
        .delete(url)
        .header("Content-Type", JSON_UTF8)
        .header("Date", &date)
        .header("authorization", glo.config.auth_token(key_name, &date, Method::DELETE))
        .header("Accpet", "*/*")
        .send()
        .await;
    match rsp {
        Ok(r) => info!("del file {} rsp: {}", key_name, r.status()),
        Err(e) => error!("del file {} fail, err: {}", key_name, e.to_string()),
    }
}

pub async fn get_file(glo: &Global, key_name: &str) -> Bytes {
    let date = get_now_date_str();
    let mut url = String::from(glo.oss_host());
    url.push_str("/");
    url.push_str(key_name);
    let rsp = glo.client.clone()
        .get(url)
        .header("Content-Type", JSON_UTF8)
        .header("Date", &date)
        .header("authorization", glo.config.auth_token(key_name, &date, Method::GET))
        .header("Accpet", "*/*")
        .send()
        .await
        .expect("req fail")
        .bytes()
        .await
        .expect("download fail");
    rsp
}

async fn delete_all_by_prefix(glo: &Global, prefix: &str) {
    info!("delete all file with prefix: {}", prefix);
    loop {
        let files = list_files(glo, prefix).await.unwrap();
        info!("list file by prefix, prefix:{}, size: {}", prefix, files.capacity());
        if !files.is_empty() {
            for name in files {
                delete_file(glo, &name).await;
            }
        } else {
            break;
        }
    }
}

pub async fn delete_all(glo: &Global) {
    delete_all_by_prefix(glo, PREFIX_LOCAL).await;
    delete_all_by_prefix(glo, PREFIX_REMOTE).await;
}

pub fn new_client() -> Client{
    let client = reqwest::ClientBuilder::new().no_trust_dns().no_proxy().build().unwrap();
    client
}

#[cfg(test)]
mod tests {

    use bytes::Bytes;

    const UCLOUD_OSS_PUB_HOST: &str = "";
    use crate::{oss_http::{ delete_file, get_file, list_files, put_file}, get_config, constants::Global};

    use super::new_client;

    #[test]
    fn auth_token_test() {
        let date = "20220526160702";
        let config = get_config();
        let sign = config.auth_token("", date, reqwest::Method::GET);
        assert_eq!(
            &sign,
            "UCloud TOKEN_09281239-7aa2-478f-b4d6-a1da1db55703:Hst2I1fVL/+qZkGk2PuH7wVf/nQ="
        );
    }

    #[test]
    fn list_file_test() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let client = new_client();
        let glo = Global{ config: get_config(), client, core_mod: crate::msg::CoreMod::Local };
        let res = rt.block_on(list_files(&glo, UCLOUD_OSS_PUB_HOST));
        match res {
            Ok(r) => {
                println!(" FILES {:?}", r)
            }
            Err(e) => {
                eprintln!("ERR {}", e)
            }
        }
    }

    #[test]
    fn put_get_delete_test() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let src = "abcdefghijklmnopqrstuvwxyz1234567890poiuytrewqasdfgghjklmnbvcxz";
        let key_name = "tmp_f.txt";
        let client = new_client();
        let glo = Global{ config: get_config(), client, core_mod: crate::msg::CoreMod::Local };
        let _pr = rt.block_on(put_file(&glo, key_name, Bytes::from(src)));
        let dr = rt.block_on(get_file(&glo, key_name));
        if dr.eq(src) {
            println!("UPLOAD DOWNLOAD OK")
        } else {
            panic!("UPLPAD DOWNLOAD FAIL")
        }
        rt.block_on(delete_file(&glo, key_name))
    }
}
