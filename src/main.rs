use std::collections::HashMap;
use std::io::{self, Write};
use std::sync::Arc;
use std::time::Duration;

use pcap::{Capture, Device};
use dotenv::dotenv;

mod tcp_stream;
mod ip_header;
mod tcp_header;
mod packet_processor;
mod ip_reassembly;
mod protocol_identifier;
mod async_log_inserter;

use ip_reassembly::IpReassembler;
use packet_processor::process_packet;
use tcp_stream::TcpStream;
use tcp_stream::TcpStreamKey;
use crate::async_log_inserter::AsyncLogInserter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // .envファイルを読み込む
    dotenv().ok();

    // 環境変数から接続情報を取得
    let db_host = std::env::var("MYSQL_HOST").expect("MYSQL_HOST must be set");
    let db_user = std::env::var("MYSQL_USER").expect("MYSQL_USER must be set");
    let db_password = std::env::var("MYSQL_PASSWORD").expect("MYSQL_PASSWORD must be set");
    let db_name = std::env::var("MYSQL_DATABASE").expect("MYSQL_DATABASE must be set");
    let db_port = std::env::var("MYSQL_PORT").expect("MYSQL_PORT must be set");

    // MySQL接続文字列を構築
    let connection_string = format!(
        "mysql://{}:{}@{}:{}/{}",
        db_user, db_password, db_host, db_port, db_name
    );

    let inserter = Arc::new(AsyncLogInserter::new(&connection_string).await?);
    let device_list = Device::list()?;

    println!("利用可能なデバイス:");
    for (index, device) in device_list.iter().enumerate() {
        println!("{}. {}", index + 1, device.name);
        println!("   説明: {}", device.desc.as_deref().unwrap_or("説明なし"));
        println!("   アドレス: {:?}", device.addresses);
        println!();
    }

    print!("キャプチャするデバイスの番号を入力してください: ");
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let device_index: usize = input.trim().parse()?;

    if device_index == 0 || device_index > device_list.len() {
        return Err("無効なデバイス番号です".into());
    }

    let selected_device = &device_list[device_index - 1];
    println!("選択されたデバイス: {}", selected_device.name);

    let mut cap = Capture::from_device(selected_device.clone())?
        .promisc(true)
        .snaplen(65535)
        .timeout(0)
        .immediate_mode(true)
        .buffer_size(3 * 1024 * 1024)
        .open()?;

    println!("パケットのキャプチャを開始します。Ctrl+Cで終了します。");

    let mut streams: HashMap<TcpStreamKey, TcpStream> = HashMap::new();
    let mut ip_reassembler = IpReassembler::new(Duration::from_secs(30));

    while let Ok(packet) = cap.next_packet() {
        process_packet(&packet, &mut streams, &mut ip_reassembler, Arc::clone(&inserter)).await?;

        // 古いストリームの削除
        streams.retain(|_, stream| {
            stream.last_activity.elapsed() < Duration::from_secs(300) || stream.state != tcp_stream::TcpState::Closed
        });
    }

    Ok(())
}