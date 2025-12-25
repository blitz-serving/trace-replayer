use std::convert::Infallible;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use clap::Parser;
use http_body_util::Full;
use hyper::body::Bytes;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response};
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;
use tokio::spawn;
use tokio::time::sleep;

static COUNTER: AtomicUsize = AtomicUsize::new(0);

#[derive(Parser, Debug)]
pub struct Args {
    /// Mock server listening address.
    #[clap(short, long, required = true)]
    pub addr: String,
}

async fn hello(_: Request<hyper::body::Incoming>) -> Result<Response<Full<Bytes>>, Infallible> {
    COUNTER.fetch_add(1, Ordering::Relaxed);
    sleep(Duration::from_millis(rand::random::<u64>() % 200 + 100)).await;
    Ok(Response::new(Full::new(Bytes::from("Hello, World!"))))
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    let listener = TcpListener::bind(args.addr).await.unwrap();

    spawn(async move {
        let interval = 5;
        let mut prev = 0;
        let mut epoch = 0;
        loop {
            epoch += 1;
            sleep(Duration::from_secs(interval)).await;
            let count = COUNTER.load(Ordering::Relaxed);
            println!(
                "{:<4} rps = {}",
                epoch,
                (count - prev) as f64 / interval as f64
            );
            prev = count;
        }
    });

    loop {
        let (stream, _) = listener.accept().await.unwrap();
        let io = TokioIo::new(stream);
        spawn(async move {
            if let Err(err) = http1::Builder::new()
                .serve_connection(io, service_fn(hello))
                .await
            {
                eprintln!("Error serving connection: {:?}", err);
            }
        });
    }
}
