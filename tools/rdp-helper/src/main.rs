use std::io::{self, BufRead, Write};
use std::thread::JoinHandle;

use ironrdp::input::Database;
use tracing::error;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::prelude::*;

use crate::protocol::HelperEvent;

mod clipboard;
mod pixels;
mod protocol;
mod rdp;

fn main() {
    if let Err(error) = run() {
        error!(?error, "RDP helper failed");
        let _ = write_event(&HelperEvent::ConnectionFailure {
            message: format!("{error:#}"),
        });
        std::process::exit(1);
    }
}

fn run() -> anyhow::Result<()> {
    setup_logging()?;
    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();
    let connect = read_connect_request(&mut lines)?;
    write_event(&HelperEvent::Status {
        message: format!("connecting to RDP {}", connect.destination),
    })?;
    let initial_width = connect.width;
    let initial_height = connect.height;

    let runtime = rdp::start(connect)?;
    write_event(&HelperEvent::Connected {
        width: initial_width,
        height: initial_height,
    })?;
    let output_thread = spawn_output_writer(runtime.output_rx);
    let mut input_database = Database::new();

    for line in lines {
        let request = protocol::decode_request_line(&line?)?;
        if !rdp::apply_input_request(
            request,
            &runtime.input_tx,
            &mut input_database,
            &runtime.clipboard,
        )? {
            break;
        }
    }

    let _ = runtime
        .input_tx
        .send(ironrdp_client::rdp::RdpInputEvent::Close);
    let _ = output_thread.join();
    Ok(())
}

fn read_connect_request(
    lines: &mut impl Iterator<Item = io::Result<String>>,
) -> anyhow::Result<protocol::ConnectRequest> {
    let line = lines
        .next()
        .transpose()?
        .ok_or_else(|| anyhow::anyhow!("missing Connect request"))?;
    protocol::connect_request(protocol::decode_request_line(&line)?)
}

fn spawn_output_writer(
    output_rx: std::sync::mpsc::Receiver<HelperEvent>,
) -> JoinHandle<anyhow::Result<()>> {
    std::thread::Builder::new()
        .name("onetcli-rdp-helper-output".to_string())
        .spawn(move || {
            for event in output_rx {
                write_event(&event)?;
            }
            Ok(())
        })
        .expect("spawn output writer")
}

fn write_event(event: &HelperEvent) -> anyhow::Result<()> {
    let line = protocol::encode_event_line(event)?;
    let mut stdout = io::stdout().lock();
    stdout.write_all(line.as_bytes())?;
    stdout.flush()?;
    Ok(())
}

fn setup_logging() -> anyhow::Result<()> {
    let env_filter = EnvFilter::builder()
        .with_env_var("ONETCLI_RDP_HELPER_LOG")
        .from_env_lossy();
    tracing_subscriber::registry()
        .with(env_filter)
        .with(tracing_subscriber::fmt::layer().with_writer(io::stderr))
        .try_init()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_first_stdin_line_as_connect_request() {
        let input = vec![Ok(
            r#"{"type":"Connect","destination":"host:3389","username":null,"password":null,"domain":null,"width":800,"height":600}"#
                .to_string(),
        )];
        let mut lines = input.into_iter();

        let request = read_connect_request(&mut lines).expect("connect request");

        assert_eq!(request.destination, "host:3389");
        assert_eq!(request.width, 800);
        assert_eq!(request.height, 600);
    }

    #[test]
    fn rejects_non_connect_first_line() {
        let input = vec![Ok(r#"{"type":"Close"}"#.to_string())];
        let mut lines = input.into_iter();

        let error = read_connect_request(&mut lines).expect_err("not a connect request");

        assert!(error.to_string().contains("first helper request"));
    }
}
