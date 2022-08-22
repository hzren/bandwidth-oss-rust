use std::{env, sync::Arc};

use common::{msg::{CoreMsg, CoreMod, UploadMsg}, constants::{PREFIX_REMOTE, PREFIX_LOCAL, Global}, OssDownloader, OssUploader, Core};
use fast_socks5::{SocksError};
use log::info;
use tokio::sync::mpsc::{channel};
use tokio::task;

pub type Result<T, E = SocksError> = core::result::Result<T, E>;


/// 服务端代理程序
/// tokio异步实现
/// 协程定义
/// OSS协程(1)， HTTP协程(1)， 各代理协程(N)
/// OSS协程: 从管道监听数据，根据数据包目的地，转发数据到相应协程管道
/// HTTP协程: 从oss下载文件，解析，发送解析之后的数据包给OSS协程。从自身管道接收消息，合并上传文件至OSS服务器
/// 各代理协程: 接收来自socket和管道的消息，socket消息包装后发给oss协程管道，管道消息解包从socket发出
#[tokio::main]
async fn main() -> Result<()>{
    let args: Vec<String> = env::args().collect();
    //遍历打印参数索引、值
    for (index, value) in args.iter().enumerate() {
        eprintln!("{} => {}", index,value );
    }
    common::init_logger();

    let config = common::get_config();
    let oss_client = common::oss_http::new_client();
    let glo_ref:Arc<Global> = Arc::new(Global { config: config, client: oss_client, core_mod: CoreMod::Local });
    info!("Starting proxy at remote side ...");

    common::oss_http::delete_all(&glo_ref).await;

    let (o_tx, o_rx) = channel::<UploadMsg>(4096);
    let (c_tx, c_rx) = channel::<CoreMsg>(4096);
    
    let downloader = OssDownloader::new(c_tx.clone(), Arc::clone(&glo_ref), PREFIX_LOCAL);
    task::spawn(async move {
        downloader.start().await;
    });

    let mut uploader = OssUploader::new(Arc::clone(&glo_ref), PREFIX_REMOTE, o_tx.clone(), o_rx);
    task::spawn(async move {
        uploader.start().await;
    });

    let mut core = Core::new(CoreMod::Remote, c_tx, c_rx, o_tx.clone());
    core.start().await;
    Ok(())
}

