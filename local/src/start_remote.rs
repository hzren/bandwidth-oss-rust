use common::oss_http::Config;
use ssh2::Session;
use std::fs::File;
use std::{io, thread};
use std::net::TcpStream;

pub fn start_remote(config: &Config) {
    // Connect to the local SSH server
    let addr: String = config.serverHost.clone().clone() + "22";
    let tcp = TcpStream::connect(addr).unwrap();
    let mut sess = Session::new().unwrap();
    sess.set_tcp_stream(tcp);
    sess.handshake().unwrap();

    sess.userauth_password(&config.serverUser, &config.serverPasswd).unwrap();
    assert!(sess.authenticated());
    let mut channel = sess.channel_session().unwrap();
    channel.exec("/home/remote").unwrap();
    let mut writer = File::open("remote.log").unwrap();
    thread::spawn(move || {
        let res = io::copy(&mut channel, &mut writer);
        match res {
            Ok(len) => {
                info!("remote log len: {}", &len);
            },
            Err(e) => {
                error!("remote log write fail, {}", &e);
            }
        }
        std::process::exit(0);
    });

}
