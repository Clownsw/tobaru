#![feature(build_hasher_simple_hash_one)]
#![feature(once_cell)]

mod async_stream;
mod config;
mod copy_bidirectional;
mod iptables_util;
mod rustls_util;
mod tcp;
mod tokio_util;
mod udp;

use futures::future::try_join_all;
use log::{debug, error, info};
use tokio::runtime::Builder;

use crate::config::{ServerConfig, TargetConfigs};
use crate::tcp::run_tcp_server;
use crate::udp::run_udp_server;

async fn run(server_config: ServerConfig) {
    let ServerConfig {
        server_address,
        use_iptables,
        target_configs,
    } = server_config;

    match target_configs {
        TargetConfigs::Tcp(target_configs) => {
            // TODO: restart or panic?
            if let Err(e) = run_tcp_server(server_address, use_iptables, target_configs).await {
                error!("TCP forwarder finished with error: {}", e);
            }
        }
        TargetConfigs::Udp(target_configs) => {
            // TODO: restart or panic?
            if let Err(e) = run_udp_server(server_address, use_iptables, target_configs).await {
                error!("UDP forwarder finished with error: {}", e);
            }
        }
    }
}

fn main() {
    env_logger::init();

    let mut config_paths = vec![];
    let mut config_urls = vec![];
    let mut clear_iptables_only = false;
    let mut num_threads = 0usize;
    for arg in std::env::args().skip(1) {
        if arg == "--clear-iptables" {
            clear_iptables_only = true;
        } else if arg.starts_with("-t") {
            num_threads = arg[2..].parse::<usize>().expect("Invalid thread count");
        } else if arg.find("://").is_some() {
            config_urls.push(arg);
        } else {
            config_paths.push(arg);
        }
    }

    let server_configs: Vec<ServerConfig> = config::load_configs(config_paths, config_urls);

    if server_configs.is_empty() {
        error!("No server configs found.");
        return;
    }

    debug!("Loaded server configs: {:#?}", &server_configs);

    for server_config in server_configs.iter() {
        if server_config.use_iptables {
            iptables_util::clear_iptables(server_config.server_address);
        }
    }

    if clear_iptables_only {
        info!("iptables cleared, exiting.");
        return;
    }

    if num_threads == 0 {
        num_threads = std::cmp::max(
            2,
            std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(1),
        );
    }

    debug!("Runtime threads: {}", num_threads);

    let runtime = Builder::new_multi_thread()
        .worker_threads(num_threads)
        .enable_io()
        .enable_time()
        .build()
        .expect("Could not build tokio runtime");

    runtime.block_on(async move {
        let mut join_handles = Vec::with_capacity(server_configs.len());
        for server_config in server_configs {
            join_handles.push(tokio::spawn(async move {
                run(server_config).await;
            }));
        }

        // Die on any server error.
        try_join_all(join_handles).await.unwrap();
    });
}
