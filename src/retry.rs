use std::time::Duration;
use tokio::{sync::mpsc, time};

pub async fn run_retry_loop(
    mut rx_stop_request: mpsc::UnboundedReceiver<()>,
    tx_retry_request: mpsc::UnboundedSender<()>,
    retry_frequency: Duration,
) {
    loop {
        tokio::select! {
            _ = time::sleep(retry_frequency) => {
                let _ = tx_retry_request.send(());
            },
            _ = rx_stop_request.recv() => {
                break;
            }
        }
    }
}
