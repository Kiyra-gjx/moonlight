//! Moonlight —— 企业研发团队内部管理工具
//! 核心逻辑（负载统计 / 智能派工 / 风险预警 / 健康度）全部由 Rust 实现，
//! 交互界面为内嵌的单页 Web 应用，启动后浏览器访问即可。

mod api;
mod date;
mod http;
mod insight;
mod model;
mod store;

use std::sync::Mutex;

use store::Store;

fn main() {
    let (addr, data_path) = parse_args();

    let store = match Store::load(&data_path) {
        Ok(store) => Mutex::new(store),
        Err(e) => {
            eprintln!("[moonlight] 数据加载失败: {e}");
            eprintln!("[moonlight] 为避免覆盖原数据，服务未启动");
            std::process::exit(1);
        }
    };
    {
        let s = store.lock().unwrap();
        println!(
            "[moonlight] 数据文件 {data_path}（成员 {} 人，任务 {} 条）",
            s.members.len(),
            s.tasks.len()
        );
    }

    println!("[moonlight] 控制台已启动 -> http://{addr}");
    if let Err(e) = http::serve(&addr, move |req| api::route(&store, req)) {
        eprintln!("[moonlight] 监听 {addr} 失败: {e}");
        std::process::exit(1);
    }
}

/// 解析命令行：`moonlight [--port 8088] [--data ./data/moonlight.json]`
fn parse_args() -> (String, String) {
    let mut port = "8088".to_string();
    let mut data = "data/moonlight.json".to_string();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--port" | "-p" if i + 1 < args.len() => {
                port = args[i + 1].clone();
                i += 2;
            }
            "--data" | "-d" if i + 1 < args.len() => {
                data = args[i + 1].clone();
                i += 2;
            }
            "--help" | "-h" => {
                println!("用法: moonlight [--port 8088] [--data data/moonlight.json]");
                std::process::exit(0);
            }
            other => {
                eprintln!("[moonlight] 未知参数: {other}");
                std::process::exit(1);
            }
        }
    }
    (format!("127.0.0.1:{port}"), data)
}
