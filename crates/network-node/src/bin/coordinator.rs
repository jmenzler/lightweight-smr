//! Barrier coordinator: per round, collect one RoundDone from every node, then broadcast RoundGo.

use network_node::cli::arg;
use network_node::net::read_frame;
use network_node::wire::{Msg, decode, encode};
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};

const MAX_FRAME: u32 = 1024 * 1024;

fn die(msg: &str) -> ! {
    network_node::cli::die("coordinator", msg)
}

fn main() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    rt.block_on(run());
}

async fn run() {
    let port: u16 = arg("--port")
        .unwrap_or_else(|| die("--port required"))
        .parse()
        .unwrap_or_else(|_| die("--port"));
    let n: usize = arg("--n")
        .unwrap_or_else(|| die("--n required"))
        .parse()
        .unwrap_or_else(|_| die("--n"));
    let rounds: u64 = arg("--rounds")
        .unwrap_or_else(|| die("--rounds required"))
        .parse()
        .unwrap_or_else(|_| die("--rounds"));

    let listener = TcpListener::bind(("0.0.0.0", port))
        .await
        .unwrap_or_else(|e| die(&format!("bind :{port}: {e}")));
    let mut conns: Vec<TcpStream> = Vec::with_capacity(n);
    while conns.len() < n {
        match listener.accept().await {
            Ok((stream, _)) => conns.push(stream),
            Err(e) => die(&format!("accept: {e}")),
        }
    }

    for round in 1..=rounds {
        // accept order is unrelated to node ids, so completion is tracked by connection
        for (i, stream) in conns.iter_mut().enumerate() {
            let frame = read_frame(stream, MAX_FRAME)
                .await
                .unwrap_or_else(|e| die(&format!("round {round}: node conn {i}: {e}")));
            match decode(&frame, MAX_FRAME) {
                Ok(Msg::RoundDone { round: r, .. }) if r == round => {}
                other => die(&format!("round {round}: unexpected {other:?}")),
            }
        }
        let go = encode(&Msg::RoundGo { round }, MAX_FRAME, 0).expect("control frame");
        for stream in conns.iter_mut() {
            if stream.write_all(&go).await.is_err() {
                die(&format!("round {round}: RoundGo write failed"));
            }
        }
    }
}
