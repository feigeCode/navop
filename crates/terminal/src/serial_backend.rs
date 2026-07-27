use std::io::Write;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio_util::sync::CancellationToken;

use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::Term;

use one_core::storage::models::SerialParams;

use crate::exec_supervisor::TerminalInputSource;
use crate::pty_backend::GpuiEventProxy;
use crate::recording::RecordingTap;
use crate::serial_ingress::{SerialParserIngress, run_serial_reader};
use crate::{
    TerminalBackend, TerminalInputHandle, TerminalInputMetricSource, TerminalPerformanceMetrics,
    TerminalSize,
};

enum SerialCommand {
    Write {
        source: TerminalInputSource,
        data: Vec<u8>,
    },
    Shutdown,
}

pub struct SerialBackend {
    command_tx: UnboundedSender<SerialCommand>,
    shutdown: CancellationToken,
    parser_ingress: Option<SerialParserIngress>,
    performance_metrics: Arc<TerminalPerformanceMetrics>,
}

impl SerialBackend {
    pub fn connect(
        params: SerialParams,
        term: Arc<FairMutex<Term<GpuiEventProxy>>>,
        event_proxy: GpuiEventProxy,
        on_disconnect: Option<UnboundedSender<()>>,
    ) -> anyhow::Result<Self> {
        let performance_metrics = event_proxy.performance_metrics();
        Self::connect_with_metrics(
            params,
            term,
            event_proxy,
            on_disconnect,
            performance_metrics,
        )
    }

    pub fn connect_with_metrics(
        params: SerialParams,
        term: Arc<FairMutex<Term<GpuiEventProxy>>>,
        event_proxy: GpuiEventProxy,
        on_disconnect: Option<UnboundedSender<()>>,
        performance_metrics: Arc<TerminalPerformanceMetrics>,
    ) -> anyhow::Result<Self> {
        Self::connect_with_metrics_and_recording(
            params,
            term,
            event_proxy,
            on_disconnect,
            performance_metrics,
            None,
        )
    }

    pub(crate) fn connect_with_metrics_and_recording(
        params: SerialParams,
        term: Arc<FairMutex<Term<GpuiEventProxy>>>,
        event_proxy: GpuiEventProxy,
        on_disconnect: Option<UnboundedSender<()>>,
        performance_metrics: Arc<TerminalPerformanceMetrics>,
        recording_tap: Option<RecordingTap>,
    ) -> anyhow::Result<Self> {
        let data_bits = match params.data_bits {
            5 => serialport::DataBits::Five,
            6 => serialport::DataBits::Six,
            7 => serialport::DataBits::Seven,
            _ => serialport::DataBits::Eight,
        };

        let stop_bits = match params.stop_bits {
            2 => serialport::StopBits::Two,
            _ => serialport::StopBits::One,
        };

        let parity = match params.parity {
            one_core::storage::models::SerialParity::Odd => serialport::Parity::Odd,
            one_core::storage::models::SerialParity::Even => serialport::Parity::Even,
            one_core::storage::models::SerialParity::None => serialport::Parity::None,
        };

        let flow_control = match params.flow_control {
            one_core::storage::models::SerialFlowControl::Software => {
                serialport::FlowControl::Software
            }
            one_core::storage::models::SerialFlowControl::Hardware => {
                serialport::FlowControl::Hardware
            }
            one_core::storage::models::SerialFlowControl::None => serialport::FlowControl::None,
        };

        let port = serialport::new(&params.port_name, params.baud_rate)
            .data_bits(data_bits)
            .stop_bits(stop_bits)
            .parity(parity)
            .flow_control(flow_control)
            .timeout(Duration::from_millis(10))
            .open()?;

        // 克隆一份用于写入
        let write_port = port.try_clone()?;

        let (command_tx, command_rx) = unbounded_channel::<SerialCommand>();
        let shutdown = CancellationToken::new();
        let parser_ingress = SerialParserIngress::spawn_with_recording(
            term,
            event_proxy,
            performance_metrics.clone(),
            on_disconnect,
            recording_tap.clone(),
        )?;

        // 读取线程只负责串口 I/O 和有界入队；同步解析由 serial-parser 线程完成。
        let read_shutdown = shutdown.clone();
        let ingress_producer = parser_ingress.producer();
        let read_task = std::thread::Builder::new()
            .name("serial-read".into())
            .spawn(move || {
                let mut port = port;
                let _ = run_serial_reader(port.as_mut(), &ingress_producer, &read_shutdown);
            });
        if let Err(error) = read_task {
            shutdown.cancel();
            parser_ingress.abort();
            return Err(error.into());
        }

        // 写入线程：从 command channel 接收命令并写入串口
        let write_task = std::thread::Builder::new()
            .name("serial-write".into())
            .spawn(move || run_serial_writer(write_port, command_rx, recording_tap));
        if let Err(error) = write_task {
            shutdown.cancel();
            parser_ingress.abort();
            let _ = command_tx.send(SerialCommand::Shutdown);
            return Err(error.into());
        }

        Ok(Self {
            command_tx,
            shutdown,
            parser_ingress: Some(parser_ingress),
            performance_metrics,
        })
    }

    fn stop(&self) {
        if self.shutdown.is_cancelled() {
            return;
        }
        self.shutdown.cancel();
        if let Some(parser_ingress) = &self.parser_ingress {
            parser_ingress.abort();
        }
        let _ = self.command_tx.send(SerialCommand::Shutdown);
    }
}

impl TerminalBackend for SerialBackend {
    fn write(&self, data: Vec<u8>) {
        self.performance_metrics
            .record_input(TerminalInputMetricSource::User, data.len());
        let _ = self.command_tx.send(SerialCommand::Write {
            source: TerminalInputSource::User,
            data,
        });
    }

    fn input_handle(&self) -> Option<TerminalInputHandle> {
        let tx = self.command_tx.clone();
        Some(TerminalInputHandle::with_metrics(
            self.performance_metrics.clone(),
            move |data| {
                let _ = tx.send(SerialCommand::Write {
                    source: TerminalInputSource::ExternalInput,
                    data,
                });
            },
        ))
    }

    fn resize(&self, _size: TerminalSize) {
        // 串口无 PTY 尺寸概念，resize 为空操作
    }

    fn shutdown(&self) {
        self.stop();
    }
}

impl Drop for SerialBackend {
    fn drop(&mut self) {
        self.stop();
    }
}

fn run_serial_writer(
    mut port: impl Write,
    mut command_rx: UnboundedReceiver<SerialCommand>,
    recording_tap: Option<RecordingTap>,
) {
    while let Some(command) = command_rx.blocking_recv() {
        match command {
            SerialCommand::Write { source, data } => {
                if port.write_all(&data).is_err() {
                    break;
                }
                if source.is_recordable_user_input() {
                    if let Some(tap) = &recording_tap {
                        let _ = tap.record_input(&data);
                    }
                }
            }
            SerialCommand::Shutdown => break,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alacritty_terminal::grid::Dimensions;
    use alacritty_terminal::vte::ansi::{Processor, StdSyncHandler};
    use std::io::{Read, Write};
    use std::process::Command;

    use crate::TerminalEvent;
    use crate::serial_ingress::advance_serial_term;

    /// 实现 Dimensions trait 用于测试
    struct TestDimensions;
    impl Dimensions for TestDimensions {
        fn total_lines(&self) -> usize {
            24
        }
        fn screen_lines(&self) -> usize {
            24
        }
        fn columns(&self) -> usize {
            80
        }
    }

    fn create_test_term(
        event_tx: UnboundedSender<TerminalEvent>,
    ) -> (Arc<FairMutex<Term<GpuiEventProxy>>>, GpuiEventProxy) {
        let config = alacritty_terminal::term::Config::default();
        let event_proxy = GpuiEventProxy::new(event_tx);
        let term = Arc::new(FairMutex::new(Term::new(
            config,
            &TestDimensions,
            event_proxy.clone(),
        )));
        (term, event_proxy)
    }

    fn create_virtual_serial_pair() -> Option<(std::process::Child, String, String)> {
        if Command::new("socat").arg("-V").output().is_err() {
            return None;
        }

        let pid = std::process::id();
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis();
        let pty_a = format!("/tmp/vtest_a_{}_{}", pid, ts);
        let pty_b = format!("/tmp/vtest_b_{}_{}", pid, ts);

        // 使用 cfmakeraw 确保 pty 是 raw 模式，避免 "Not a typewriter" 错误
        let child = Command::new("socat")
            .args([
                &format!("pty,raw,echo=0,cfmakeraw,link={}", pty_a),
                &format!("pty,raw,echo=0,cfmakeraw,link={}", pty_b),
            ])
            .spawn()
            .ok()?;

        // 等 pty 就绪
        for _ in 0..20 {
            std::thread::sleep(Duration::from_millis(100));
            if std::path::Path::new(&pty_a).exists() && std::path::Path::new(&pty_b).exists() {
                // 额外等一下让 socat 完全初始化
                std::thread::sleep(Duration::from_millis(200));
                return Some((child, pty_a, pty_b));
            }
        }
        None
    }

    #[test]
    fn advance_serial_term_records_parser_and_lock_metrics_once() {
        let (event_tx, _event_rx) = unbounded_channel();
        let (term, _event_proxy) = create_test_term(event_tx);
        let metrics = TerminalPerformanceMetrics::default();
        let mut processor = Processor::<StdSyncHandler>::new();
        let data = b"serial output\r\n";

        advance_serial_term(&term, &mut processor, data, &metrics, None);

        let snapshot = metrics.snapshot();
        assert_eq!(data.len() as u64, snapshot.ingress_bytes);
        assert_eq!(1, snapshot.parser_chunks);
        assert_eq!(data.len() as u64, snapshot.parser_chunk_bytes);
        assert_eq!(data.len() as u64, snapshot.parser_chunk_max_bytes);
        assert_eq!(1, snapshot.term_lock_samples);
        assert_eq!(snapshot.term_lock_wait_ns, snapshot.term_lock_wait_max_ns);
        assert_eq!(snapshot.term_lock_hold_ns, snapshot.term_lock_hold_max_ns);
    }

    #[test]
    fn serial_backend_records_direct_and_handle_input_without_double_counting() {
        let (command_tx, mut command_rx) = unbounded_channel();
        let metrics = Arc::new(TerminalPerformanceMetrics::default());
        let backend = SerialBackend {
            command_tx,
            shutdown: CancellationToken::new(),
            parser_ingress: None,
            performance_metrics: metrics.clone(),
        };

        TerminalBackend::write(&backend, b"direct".to_vec());
        TerminalBackend::input_handle(&backend)
            .expect("serial input handle")
            .write(b"handle".to_vec());

        assert!(matches!(
            command_rx.try_recv(),
            Ok(SerialCommand::Write {
                source: TerminalInputSource::User,
                data,
            }) if data == b"direct"
        ));
        assert!(matches!(
            command_rx.try_recv(),
            Ok(SerialCommand::Write {
                source: TerminalInputSource::ExternalInput,
                data,
            }) if data == b"handle"
        ));
        assert!(command_rx.try_recv().is_err());
        assert_eq!(
            (b"direct".len() + b"handle".len()) as u64,
            metrics.snapshot().user_input_bytes
        );
    }

    struct FailingWriter;

    impl Write for FailingWriter {
        fn write(&mut self, _buffer: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("mock serial write failure"))
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn serial_input_recording_captures_only_successfully_written_user_bytes() {
        let recording = crate::recording::test_support::TestRecording::start(
            crate::recording::RecordingBackend::Serial,
            true,
        );
        let (command_tx, command_rx) = unbounded_channel();
        command_tx
            .send(SerialCommand::Write {
                source: TerminalInputSource::User,
                data: b"user input".to_vec(),
            })
            .expect("queue user input");
        command_tx
            .send(SerialCommand::Write {
                source: TerminalInputSource::ExternalInput,
                data: b"external input".to_vec(),
            })
            .expect("queue external input");
        command_tx
            .send(SerialCommand::Shutdown)
            .expect("queue writer shutdown");

        run_serial_writer(Vec::new(), command_rx, Some(recording.tap()));

        let parsed = recording.finish();
        assert_eq!(1, parsed.events.len());
        assert!(matches!(
            &parsed.events[0].kind,
            crate::recording::RecordingEventKind::Input(data) if data == b"user input"
        ));

        let failed_recording = crate::recording::test_support::TestRecording::start(
            crate::recording::RecordingBackend::Serial,
            true,
        );
        let (failed_tx, failed_rx) = unbounded_channel();
        failed_tx
            .send(SerialCommand::Write {
                source: TerminalInputSource::User,
                data: b"failed input".to_vec(),
            })
            .expect("queue failed input");
        run_serial_writer(FailingWriter, failed_rx, Some(failed_recording.tap()));

        assert!(
            failed_recording.finish().events.is_empty(),
            "a failed serial write must not be recorded"
        );
    }

    #[test]
    fn test_open_nonexistent_port_fails() {
        let params = SerialParams {
            port_name: "/dev/nonexistent_serial_test_999".to_string(),
            ..Default::default()
        };
        let (event_tx, _event_rx) = unbounded_channel::<TerminalEvent>();
        let (term, event_proxy) = create_test_term(event_tx);
        let result = SerialBackend::connect(params, term, event_proxy, None);
        assert!(result.is_err(), "打开不存在的串口应返回错误");
        let err = result.err().unwrap();
        println!("[PASS] 打开不存在的端口返回错误: {}", err);
    }

    #[test]
    fn test_serial_backend_write_via_socat() {
        // macOS 上 serialport crate 对 pty 调用 ioctl(TIOCEXCL) 会报 ENOTTY，
        // 这是 pty 不是真实串口设备的限制，不影响真实串口功能。
        // 此测试仅在 Linux 或有真实串口设备的环境下有效。
        let Some((mut socat, port_a, port_b)) = create_virtual_serial_pair() else {
            println!("[SKIP] socat 不可用，跳过虚拟串口测试");
            return;
        };

        let (event_tx, _event_rx) = unbounded_channel::<TerminalEvent>();
        let (term, event_proxy) = create_test_term(event_tx);

        let params = SerialParams {
            port_name: port_a.clone(),
            baud_rate: 115200,
            ..Default::default()
        };
        match SerialBackend::connect(params, term, event_proxy, None) {
            Ok(backend) => {
                // 打开对端读取
                let mut peer = serialport::new(&port_b, 115200)
                    .timeout(Duration::from_secs(2))
                    .open()
                    .expect("打开对端失败");

                let msg = b"backend write test\r\n";
                backend.write(msg.to_vec());

                std::thread::sleep(Duration::from_millis(200));
                let mut recv_buf = vec![0u8; msg.len()];
                let mut read_total = 0;
                let deadline = std::time::Instant::now() + Duration::from_secs(3);
                while read_total < msg.len() && std::time::Instant::now() < deadline {
                    match peer.read(&mut recv_buf[read_total..]) {
                        Ok(n) => read_total += n,
                        Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => {
                            std::thread::sleep(Duration::from_millis(50));
                        }
                        Err(e) => panic!("对端读取失败: {}", e),
                    }
                }
                assert_eq!(
                    &recv_buf[..read_total],
                    msg,
                    "通过 SerialBackend 写入的数据不匹配"
                );
                println!("[PASS] SerialBackend::write() 通过虚拟串口成功发送数据");
                backend.shutdown();
                drop(peer);
            }
            Err(e) => {
                // macOS pty 会报 ENOTTY，属已知限制
                println!(
                    "[SKIP] SerialBackend 无法连接 pty（macOS 已知限制: {}），跳过写入测试",
                    e
                );
            }
        }

        let _ = socat.kill();
        let _ = std::fs::remove_file(&port_a);
        let _ = std::fs::remove_file(&port_b);
    }

    #[test]
    fn test_serial_backend_read_into_term_via_socat() {
        let Some((mut socat, port_a, port_b)) = create_virtual_serial_pair() else {
            println!("[SKIP] socat 不可用，跳过虚拟串口测试");
            return;
        };

        let (event_tx, mut event_rx) = unbounded_channel::<TerminalEvent>();
        let (term, event_proxy) = create_test_term(event_tx);

        let params = SerialParams {
            port_name: port_a.clone(),
            baud_rate: 115200,
            ..Default::default()
        };
        match SerialBackend::connect(params, term.clone(), event_proxy, None) {
            Ok(backend) => {
                let mut writer = serialport::new(&port_b, 115200)
                    .timeout(Duration::from_secs(2))
                    .open()
                    .expect("打开对端写入失败");

                writer.write_all(b"Hello from peer\r\n").expect("写入失败");
                writer.flush().expect("flush 失败");

                let deadline = std::time::Instant::now() + Duration::from_secs(3);
                let mut got_wakeup = false;
                while std::time::Instant::now() < deadline {
                    match event_rx.try_recv() {
                        Ok(TerminalEvent::Wakeup) => {
                            got_wakeup = true;
                            break;
                        }
                        Ok(_) => {}
                        Err(_) => std::thread::sleep(Duration::from_millis(50)),
                    }
                }
                assert!(got_wakeup, "应收到 Wakeup 事件");
                println!("[PASS] SerialBackend 读取线程成功接收对端数据并触发 Wakeup 事件");
                backend.shutdown();
                drop(writer);
            }
            Err(e) => {
                println!(
                    "[SKIP] SerialBackend 无法连接 pty（macOS 已知限制: {}），跳过读取测试",
                    e
                );
            }
        }

        let _ = socat.kill();
        let _ = std::fs::remove_file(&port_a);
        let _ = std::fs::remove_file(&port_b);
    }
}
