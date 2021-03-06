use std::net::IpAddr;
use std::path::PathBuf;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use arrayfire as af;
use futures::{Future, Stream};
use log::debug;
use structopt::StructOpt;

mod auth;
mod block;
mod chain;
mod class;
mod cluster;
mod collection;
mod error;
mod gateway;
mod general;
mod handler;
mod kernel;
mod lock;
mod logger;
mod object;
mod request;
mod scalar;
mod stream;
mod transaction;

use safecast::*;

type TCBoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + 'a + Send>>;
type TCBoxTryFuture<'a, T> = TCBoxFuture<'a, TCResult<T>>;
type TCResult<T> = Result<T, error::TCError>;
type TCStream<'a, T> = Pin<Box<dyn Stream<Item = T> + Send + Unpin + 'a>>;
type TCTryStream<'a, T> = TCStream<'a, TCResult<T>>;

const VERSION: &str = env!("CARGO_PKG_VERSION");
static LOGGER: logger::Logger = logger::Logger;

fn data_size(flag: &str) -> TCResult<usize> {
    if flag.is_empty() {
        return Err(error::bad_request("Invalid size specified", flag));
    }

    let msg = "Unable to parse value";
    let size = usize::from_str_radix(&flag[0..flag.len() - 1], 10)
        .map_err(|_| error::bad_request(msg, flag))?;

    if flag.ends_with('K') {
        Ok(size * 1000)
    } else if flag.ends_with('M') {
        Ok(size * 1_000_000)
    } else {
        Err(error::bad_request("Unable to parse request_limit", flag))
    }
}

fn duration(flag: &str) -> TCResult<Duration> {
    u64::from_str(flag)
        .map(Duration::from_secs)
        .map_err(|_| error::bad_request("Invalid duration", flag))
}

#[derive(Clone, StructOpt)]
struct Config {
    #[structopt(long = "address", default_value = "127.0.0.1")]
    pub address: IpAddr,

    #[structopt(long = "data_dir", default_value = "/tmp/tc/data")]
    pub data_dir: PathBuf,

    #[structopt(long = "workspace", default_value = "/tmp/tc/tmp")]
    pub workspace: PathBuf,

    #[structopt(long = "http_port", default_value = "8702")]
    pub http_port: u16,

    #[structopt(long = "ext")]
    pub adapters: Vec<scalar::value::link::Link>,

    #[structopt(long = "host")]
    pub hosted: Vec<scalar::value::link::TCPathBuf>,

    #[structopt(long = "peer")]
    pub peers: Vec<scalar::value::link::LinkHost>,

    #[structopt(long = "request_limit", default_value = "10M", parse(try_from_str = data_size))]
    pub request_limit: usize,

    #[structopt(long = "request_ttl", default_value = "30", parse(try_from_str = duration))]
    pub request_ttl: Duration,

    #[structopt(long = "log_level", default_value = "warn")]
    pub log_level: log::LevelFilter,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let config = Config::from_args();

    println!("Tinychain version {}", VERSION);
    println!("Data directory: {}", &config.data_dir.to_str().unwrap());
    println!("Working directory: {}", &config.workspace.to_str().unwrap());
    println!();

    af::info();
    println!();

    log::set_logger(&LOGGER)
        .map(|()| log::set_max_level(config.log_level))
        .map_err(|e| error::internal(format!("Unable to configure logging: {}", e)))?;

    let txn_id = transaction::TxnId::new(gateway::Gateway::time());
    let fs_cache_persistent = block::hostfs::mount(config.data_dir);
    let data_dir = block::Dir::create(fs_cache_persistent, "data_dir");
    let fs_cache_temporary = block::hostfs::mount(config.workspace);
    let workspace = block::Dir::create(fs_cache_temporary, "workspace");

    use transaction::Transact;
    data_dir.commit(&txn_id).await;
    workspace.commit(&txn_id).await;

    let hosted = configure(config.hosted, data_dir.clone(), workspace.clone()).await?;
    let gateway = gateway::Gateway::new(
        config.adapters,
        hosted,
        workspace.clone(),
        config.request_limit,
        config.request_ttl,
    )
    .map_err(Box::new)?;

    Arc::new(gateway)
        .http_listen(config.address, config.http_port)
        .await
        .map_err(|e| e.into())
}

async fn configure(
    clusters: Vec<scalar::value::link::TCPathBuf>,
    data_dir: Arc<block::Dir>,
    workspace: Arc<block::Dir>,
) -> TCResult<gateway::Hosted> {
    const RESERVED: [&str; 2] = ["/sbin", "/transact"];

    let mut hosted = gateway::Hosted::new();
    for path in clusters {
        for reserved in &RESERVED {
            if path.to_string().starts_with(reserved) {
                return Err(error::unsupported(format!(
                    "Cannot host cluster at {} because the path {} is reserved",
                    path, reserved
                )));
            } else {
                debug!("configuring cluster at {}...", path);
            }
        }

        let cluster = cluster::Cluster::create(path.clone(), data_dir.clone(), workspace.clone())?;
        hosted.push(path, cluster);
    }

    Ok(hosted)
}
