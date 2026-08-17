//! Telnet 终端后端。
//!
//! 参考 oxideterm 的 telnet 实现，结合 navop 的串口/SSH 后端结构：
//! - 连接阶段与 SSH 一致：先完成 TCP 连接再返回后端，避免界面提前进入
//!   Connected 状态；
//! - 传输层使用 Tokio TCP，工作线程负责读/写与 Telnet 协议编解码；
//! - 服务端输出经有界队列交给解析线程更新 alacritty 网格；
//! - 客户端输入、终端响应（DA/颜色/尺寸查询）与窗口尺寸变化都会经过
//!   Telnet 协议编解码（IAC 转义、协商应答、NAWS、NVT CR 编码）；
//! - 支持 SecureCRT/Xshell 风格的 expect/send 登录脚本。

use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::Term;

use one_core::storage::models::TelnetParams;

use crate::exec_supervisor::TerminalInputSource;
use crate::ingress_queue::TerminalDataSendError;
use crate::pty_backend::GpuiEventProxy;
use crate::recording::RecordingTap;
use crate::telnet_expect::TelnetLoginScript;
use crate::telnet_ingress::{TelnetIngressProducer, TelnetParserIngress};
use crate::{
    TerminalBackend, TerminalInputHandle, TerminalInputMetricSource, TerminalPerformanceMetrics,
    TerminalSize,
};

/// Telnet 连接超时（秒）。
const TELNET_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// socket 连续无写入进展的超时。每次成功写出字节后重新计时，避免误杀
/// 虽然较慢但仍持续消费数据的链路。
const TELNET_WRITE_STALL_TIMEOUT: Duration = Duration::from_secs(10);
/// 关闭 socket 写半部的最长等待时间。shutdown 不应因异常传输层永久阻塞。
const TELNET_SHUTDOWN_TIMEOUT: Duration = Duration::from_millis(250);
/// 向服务端声明的终端类型。
const TELNET_TERMINAL_TYPE: &[u8] = b"xterm-256color";
/// 读缓冲大小。
const TELNET_READ_BUFFER_BYTES: usize = 8192;
/// 协议解析器允许缓存的未完成 SB payload 最大字节数，超过后进入丢弃状态，
/// 直到合法的未转义 IAC SE 才恢复 Data 状态。
const TELNET_MAX_PENDING_BYTES: usize = 64 * 1024;
/// 非 BINARY 模式下，跨 write() 分片的 CR 等待后续字节的窗口。
/// 收到 Enter（裸 CR）后等待极短时间，若随后到达 LF 则组合为 CR LF，
/// 否则按 NVT 发送 CR NUL。
const TELNET_NVT_CR_FLUSH_DELAY: Duration = Duration::from_millis(2);

// Telnet 命令字节。
const TELNET_COMMAND_IAC: u8 = 255;
const TELNET_COMMAND_DONT: u8 = 254;
const TELNET_COMMAND_DO: u8 = 253;
const TELNET_COMMAND_WONT: u8 = 252;
const TELNET_COMMAND_WILL: u8 = 251;
const TELNET_COMMAND_SB: u8 = 250;
const TELNET_COMMAND_AYT: u8 = 246;
const TELNET_COMMAND_SE: u8 = 240;
/// RFC 854 AYT（Are You There）的可见应答。
const TELNET_AYT_RESPONSE: &[u8] = b"\r\n[Navop: yes]\r\n";

// Telnet 选项。
const TELNET_OPTION_BINARY: u8 = 0;
const TELNET_OPTION_ECHO: u8 = 1;
const TELNET_OPTION_SUPPRESS_GO_AHEAD: u8 = 3;
const TELNET_OPTION_TERMINAL_TYPE: u8 = 24;
const TELNET_OPTION_NAWS: u8 = 31;
const TELNET_TERMINAL_TYPE_IS: u8 = 0;
const TELNET_TERMINAL_TYPE_SEND: u8 = 1;

enum TelnetCommand {
    Write {
        source: TerminalInputSource,
        data: Vec<u8>,
    },
    Shutdown,
}

/// 单个 option 在一个方向上的协商状态。
///
/// `Unnegotiated` 表示从未收到过该方向的命令；稳定态重复 DO/WILL（Enabled）
/// 或 DONT/WONT（Disabled）不再生成应答，避免异常对端触发无限往返。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TelnetOptionSideState {
    Unnegotiated,
    Disabled,
    Enabled,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum TelnetParseState {
    Data,
    Iac,
    Negotiation {
        command: u8,
    },
    Subnegotiation {
        payload: Vec<u8>,
        /// 上一个字节是 SB payload 内的 IAC。
        previous_iac: bool,
    },
    /// 超长未闭合 SB：丢弃到合法、未转义的 IAC SE。
    DiscardingSubnegotiation {
        previous_iac: bool,
    },
}

/// 非 BINARY 模式的 NVT 编码器。
///
/// RFC 854 要求裸 CR 编码为 CR NUL、换行编码为 CR LF。编码器会把位于 chunk
/// 末尾的 CR 暂存一小段时间，若下一次 write() 以 LF 开头则组合为 CR LF；
/// 否则由调用方 flush 成 CR NUL。
#[derive(Clone, Debug, Default)]
struct TelnetNvtEncoder {
    pending_cr: bool,
}

impl TelnetNvtEncoder {
    fn encode(&mut self, bytes: &[u8], binary_enabled: bool) -> Vec<u8> {
        if bytes.is_empty() {
            return Vec::new();
        }

        let mut encoded = Vec::with_capacity(bytes.len() + 2);
        let mut index = 0;

        if self.pending_cr {
            self.pending_cr = false;
            if binary_enabled {
                // pending_cr 只可能在先前的 NVT 模式中产生。即使 BINARY
                // 已在两个 write 之间启用，也必须先按旧模式提交 CR NUL。
                encoded.extend_from_slice(b"\r\0");
            } else if bytes.first() == Some(&b'\n') {
                encoded.push(b'\r');
                encoded.push(b'\n');
                index = 1;
            } else if bytes.first() == Some(&0) {
                encoded.push(b'\r');
                encoded.push(0);
                index = 1;
            } else {
                encoded.push(b'\r');
                encoded.push(0);
            }
        }

        while index < bytes.len() {
            let byte = bytes[index];
            if binary_enabled {
                encoded.push(byte);
                if byte == TELNET_COMMAND_IAC {
                    encoded.push(TELNET_COMMAND_IAC);
                }
                index += 1;
                continue;
            }

            match byte {
                TELNET_COMMAND_IAC => {
                    encoded.push(TELNET_COMMAND_IAC);
                    encoded.push(TELNET_COMMAND_IAC);
                    index += 1;
                }
                b'\r' => {
                    if index + 1 < bytes.len() {
                        let next = bytes[index + 1];
                        if next == b'\n' || next == 0 {
                            encoded.push(b'\r');
                            encoded.push(next);
                            index += 2;
                        } else {
                            encoded.push(b'\r');
                            encoded.push(0);
                            index += 1;
                        }
                    } else {
                        self.pending_cr = true;
                        index += 1;
                    }
                }
                _ => {
                    encoded.push(byte);
                    index += 1;
                }
            }
        }

        encoded
    }

    fn has_pending_cr(&self) -> bool {
        self.pending_cr
    }

    fn flush(&mut self) -> Vec<u8> {
        if !self.pending_cr {
            return Vec::new();
        }
        self.pending_cr = false;
        // pending_cr 只在 NVT 模式下暂存，因此它的编码方式不应被随后到达
        // 的 BINARY 协商改变。
        vec![b'\r', 0]
    }
}
/// Telnet 协议编解码器。
///
/// 负责：
/// - 有状态地过滤服务端字节流中的 IAC 协商/子协商（可跨 TCP read 分片），
///   并生成应答；
/// - 分别跟踪本端/远端每个 option 的协商状态；
/// - 对客户端数据做 IAC 转义，并在未协商 BINARY 时执行 NVT CR 编码；
/// - 在服务端同意 NAWS 后生成窗口尺寸子协商消息。
#[derive(Clone, Debug)]
pub(crate) struct TelnetCodec {
    cols: u16,
    rows: u16,
    parse_state: TelnetParseState,
    local_options: [TelnetOptionSideState; 256],
    remote_options: [TelnetOptionSideState; 256],
    nvt_encoder: TelnetNvtEncoder,
    #[cfg(test)]
    last_subnegotiation_payload: Option<Vec<u8>>,
}

impl TelnetCodec {
    pub(crate) fn new(cols: u16, rows: u16) -> Self {
        Self {
            cols: cols.max(2),
            rows: rows.max(2),
            parse_state: TelnetParseState::Data,
            local_options: [TelnetOptionSideState::Unnegotiated; 256],
            remote_options: [TelnetOptionSideState::Unnegotiated; 256],
            nvt_encoder: TelnetNvtEncoder::default(),
            #[cfg(test)]
            last_subnegotiation_payload: None,
        }
    }

    pub(crate) fn set_window_size(&mut self, cols: u16, rows: u16) {
        self.cols = cols.max(2);
        self.rows = rows.max(2);
    }

    fn local_state(&self, option: u8) -> TelnetOptionSideState {
        self.local_options[option as usize]
    }

    fn remote_state(&self, option: u8) -> TelnetOptionSideState {
        self.remote_options[option as usize]
    }

    fn set_local_state(&mut self, option: u8, state: TelnetOptionSideState) {
        self.local_options[option as usize] = state;
    }

    fn set_remote_state(&mut self, option: u8, state: TelnetOptionSideState) {
        self.remote_options[option as usize] = state;
    }

    fn local_binary_enabled(&self) -> bool {
        self.local_state(TELNET_OPTION_BINARY) == TelnetOptionSideState::Enabled
    }

    /// 过滤服务端字节：剥离协议字节并生成需要回写的应答。
    fn filter_server_bytes(&mut self, bytes: &[u8]) -> (Vec<u8>, Vec<Vec<u8>>) {
        let mut data = Vec::with_capacity(bytes.len());
        let mut responses = Vec::new();

        for &byte in bytes {
            let state = std::mem::replace(&mut self.parse_state, TelnetParseState::Data);
            match state {
                TelnetParseState::Data => {
                    if byte == TELNET_COMMAND_IAC {
                        self.parse_state = TelnetParseState::Iac;
                    } else {
                        data.push(byte);
                    }
                }
                TelnetParseState::Iac => match byte {
                    TELNET_COMMAND_IAC => {
                        data.push(TELNET_COMMAND_IAC);
                        self.parse_state = TelnetParseState::Data;
                    }
                    TELNET_COMMAND_DO | TELNET_COMMAND_DONT | TELNET_COMMAND_WILL
                    | TELNET_COMMAND_WONT => {
                        self.parse_state = TelnetParseState::Negotiation { command: byte };
                    }
                    TELNET_COMMAND_SB => {
                        self.parse_state = TelnetParseState::Subnegotiation {
                            payload: Vec::new(),
                            previous_iac: false,
                        };
                    }
                    TELNET_COMMAND_AYT => {
                        responses.push(TELNET_AYT_RESPONSE.to_vec());
                        self.parse_state = TelnetParseState::Data;
                    }
                    // NOP、Data Mark、Break 以及孤立的 SE 等两字节命令直接忽略。
                    _ => {
                        self.parse_state = TelnetParseState::Data;
                    }
                },
                TelnetParseState::Negotiation { command } => {
                    self.parse_state = TelnetParseState::Data;
                    responses.extend(self.negotiation_responses(command, byte));
                }
                TelnetParseState::Subnegotiation {
                    mut payload,
                    previous_iac,
                } => {
                    if previous_iac {
                        match byte {
                            TELNET_COMMAND_IAC => {
                                // IAC IAC 在 SB payload 内反转义为一个 0xFF。
                                payload.push(TELNET_COMMAND_IAC);
                                self.parse_state = checked_subnegotiation_state(payload, false);
                            }
                            TELNET_COMMAND_SE => {
                                #[cfg(test)]
                                {
                                    self.last_subnegotiation_payload = Some(payload.clone());
                                }
                                responses.extend(self.subnegotiation_responses(&payload));
                                self.parse_state = TelnetParseState::Data;
                            }
                            _ => {
                                // SB 内出现未转义的 IAC 属于协议违规；丢弃该
                                // IAC 并按普通 payload 字节处理当前字节，保持同步。
                                payload.push(byte);
                                self.parse_state = checked_subnegotiation_state(payload, false);
                            }
                        }
                    } else if byte == TELNET_COMMAND_IAC {
                        self.parse_state = TelnetParseState::Subnegotiation {
                            payload,
                            previous_iac: true,
                        };
                    } else {
                        payload.push(byte);
                        self.parse_state = checked_subnegotiation_state(payload, false);
                    }
                }
                TelnetParseState::DiscardingSubnegotiation { previous_iac } => {
                    if previous_iac {
                        match byte {
                            TELNET_COMMAND_IAC => {
                                // IAC IAC 是丢弃流内的转义数据字节，不是终止符。
                                self.parse_state = TelnetParseState::DiscardingSubnegotiation {
                                    previous_iac: false,
                                };
                            }
                            TELNET_COMMAND_SE => {
                                self.parse_state = TelnetParseState::Data;
                            }
                            _ => {
                                self.parse_state = TelnetParseState::DiscardingSubnegotiation {
                                    previous_iac: false,
                                };
                            }
                        }
                    } else if byte == TELNET_COMMAND_IAC {
                        self.parse_state =
                            TelnetParseState::DiscardingSubnegotiation { previous_iac: true };
                    } else {
                        self.parse_state = TelnetParseState::DiscardingSubnegotiation {
                            previous_iac: false,
                        };
                    }
                }
            }
        }

        (data, responses)
    }

    /// 客户端数据 IAC 转义与 NVT 编码。
    fn encode_client_data(&mut self, bytes: &[u8]) -> Vec<u8> {
        self.nvt_encoder.encode(bytes, self.local_binary_enabled())
    }

    fn has_pending_client_cr(&self) -> bool {
        self.nvt_encoder.has_pending_cr()
    }

    fn flush_client_data(&mut self) -> Vec<u8> {
        self.nvt_encoder.flush()
    }

    /// 构造 NAWS 子协商消息，载荷内的 IAC 同样需要转义。
    /// 服务端尚未协商 NAWS 时返回 `None`。
    fn naws_message(&self) -> Option<Vec<u8>> {
        if self.local_state(TELNET_OPTION_NAWS) != TelnetOptionSideState::Enabled {
            return None;
        }

        let payload = [
            (self.cols >> 8) as u8,
            self.cols as u8,
            (self.rows >> 8) as u8,
            self.rows as u8,
        ];
        let mut bytes = vec![TELNET_COMMAND_IAC, TELNET_COMMAND_SB, TELNET_OPTION_NAWS];
        for byte in payload {
            bytes.push(byte);
            if byte == TELNET_COMMAND_IAC {
                bytes.push(TELNET_COMMAND_IAC);
            }
        }
        bytes.extend_from_slice(&[TELNET_COMMAND_IAC, TELNET_COMMAND_SE]);
        Some(bytes)
    }

    fn negotiation_responses(&mut self, command: u8, option: u8) -> Vec<Vec<u8>> {
        match command {
            TELNET_COMMAND_DO => {
                match self.local_state(option) {
                    TelnetOptionSideState::Enabled => {}
                    TelnetOptionSideState::Disabled | TelnetOptionSideState::Unnegotiated => {
                        if is_option_supported_locally(option) {
                            self.set_local_state(option, TelnetOptionSideState::Enabled);
                            let mut responses =
                                vec![vec![TELNET_COMMAND_IAC, TELNET_COMMAND_WILL, option]];
                            if option == TELNET_OPTION_NAWS {
                                if let Some(message) = self.naws_message() {
                                    responses.push(message);
                                }
                            }
                            return responses;
                        }
                        if self.local_state(option) == TelnetOptionSideState::Unnegotiated {
                            self.set_local_state(option, TelnetOptionSideState::Disabled);
                            return vec![vec![TELNET_COMMAND_IAC, TELNET_COMMAND_WONT, option]];
                        }
                    }
                }
                Vec::new()
            }
            TELNET_COMMAND_WILL => {
                match self.remote_state(option) {
                    TelnetOptionSideState::Enabled => {}
                    TelnetOptionSideState::Disabled | TelnetOptionSideState::Unnegotiated => {
                        if is_option_supported_remotely(option) {
                            self.set_remote_state(option, TelnetOptionSideState::Enabled);
                            return vec![vec![TELNET_COMMAND_IAC, TELNET_COMMAND_DO, option]];
                        }
                        if self.remote_state(option) == TelnetOptionSideState::Unnegotiated {
                            self.set_remote_state(option, TelnetOptionSideState::Disabled);
                            return vec![vec![TELNET_COMMAND_IAC, TELNET_COMMAND_DONT, option]];
                        }
                    }
                }
                Vec::new()
            }
            TELNET_COMMAND_DONT => {
                if self.local_state(option) == TelnetOptionSideState::Enabled {
                    self.set_local_state(option, TelnetOptionSideState::Disabled);
                    vec![vec![TELNET_COMMAND_IAC, TELNET_COMMAND_WONT, option]]
                } else {
                    self.set_local_state(option, TelnetOptionSideState::Disabled);
                    Vec::new()
                }
            }
            TELNET_COMMAND_WONT => {
                if self.remote_state(option) == TelnetOptionSideState::Enabled {
                    self.set_remote_state(option, TelnetOptionSideState::Disabled);
                    vec![vec![TELNET_COMMAND_IAC, TELNET_COMMAND_DONT, option]]
                } else {
                    self.set_remote_state(option, TelnetOptionSideState::Disabled);
                    Vec::new()
                }
            }
            _ => Vec::new(),
        }
    }

    fn subnegotiation_responses(&self, bytes: &[u8]) -> Vec<Vec<u8>> {
        if self.local_state(TELNET_OPTION_TERMINAL_TYPE) == TelnetOptionSideState::Enabled
            && bytes.first().copied() == Some(TELNET_OPTION_TERMINAL_TYPE)
            && bytes.get(1).copied() == Some(TELNET_TERMINAL_TYPE_SEND)
        {
            let mut response = vec![
                TELNET_COMMAND_IAC,
                TELNET_COMMAND_SB,
                TELNET_OPTION_TERMINAL_TYPE,
                TELNET_TERMINAL_TYPE_IS,
            ];
            response.extend_from_slice(TELNET_TERMINAL_TYPE);
            response.extend_from_slice(&[TELNET_COMMAND_IAC, TELNET_COMMAND_SE]);
            return vec![response];
        }
        Vec::new()
    }
}

fn is_option_supported_locally(option: u8) -> bool {
    matches!(
        option,
        TELNET_OPTION_BINARY
            | TELNET_OPTION_SUPPRESS_GO_AHEAD
            | TELNET_OPTION_TERMINAL_TYPE
            | TELNET_OPTION_NAWS
    )
}

fn is_option_supported_remotely(option: u8) -> bool {
    matches!(
        option,
        TELNET_OPTION_BINARY | TELNET_OPTION_ECHO | TELNET_OPTION_SUPPRESS_GO_AHEAD
    )
}

fn checked_subnegotiation_state(payload: Vec<u8>, previous_iac: bool) -> TelnetParseState {
    if payload.len() > TELNET_MAX_PENDING_BYTES {
        tracing::warn!(
            pending_bytes = payload.len(),
            "Telnet SB payload exceeded pending buffer limit; discarding until IAC SE"
        );
        TelnetParseState::DiscardingSubnegotiation {
            previous_iac: false,
        }
    } else {
        TelnetParseState::Subnegotiation {
            payload,
            previous_iac,
        }
    }
}
/// 结构化 Telnet 断线原因。正常 EOF 与显式 shutdown 不产生 error。
#[derive(Debug)]
enum TelnetDisconnectReason {
    Eof,
    Read(io::Error),
    Write {
        operation: &'static str,
        error: io::Error,
    },
    OutputQueue(String),
    Shutdown,
}

impl TelnetDisconnectReason {
    fn user_message(&self) -> Option<String> {
        match self {
            Self::Eof | Self::Shutdown => None,
            Self::Read(error) => Some(format!("读取 Telnet 数据失败: {error}")),
            Self::Write { operation, error } => {
                Some(format!("Telnet 写入失败（{operation}）: {error}"))
            }
            Self::OutputQueue(error) => Some(format!("Telnet 终端输出队列异常: {error}")),
        }
    }
}

pub struct TelnetBackend {
    command_tx: UnboundedSender<TelnetCommand>,
    resize_tx: watch::Sender<(u16, u16)>,
    shutdown: CancellationToken,
    parser_ingress: Option<TelnetParserIngress>,
    performance_metrics: Arc<TerminalPerformanceMetrics>,
}

impl TelnetBackend {
    pub async fn connect(
        params: TelnetParams,
        term: Arc<FairMutex<Term<GpuiEventProxy>>>,
        event_proxy: GpuiEventProxy,
        on_disconnect: Option<UnboundedSender<Option<String>>>,
    ) -> anyhow::Result<Self> {
        let performance_metrics = event_proxy.performance_metrics();
        Self::connect_with_metrics_and_recording(
            params,
            term,
            event_proxy,
            on_disconnect,
            performance_metrics,
            None,
        )
        .await
    }

    pub(crate) async fn connect_with_metrics_and_recording(
        params: TelnetParams,
        term: Arc<FairMutex<Term<GpuiEventProxy>>>,
        event_proxy: GpuiEventProxy,
        on_disconnect: Option<UnboundedSender<Option<String>>>,
        performance_metrics: Arc<TerminalPerformanceMetrics>,
        recording_tap: Option<RecordingTap>,
    ) -> anyhow::Result<Self> {
        let login_script = TelnetLoginScript::new(&params.login_script)
            .map_err(|error| anyhow::anyhow!("Telnet expect 正则无效: {error}"))?;
        // 与 SSH 后端一致，先完成 TCP 连接，成功后才把后端交给 Terminal。
        let stream = connect_telnet_stream(&params).await?;

        // 用户输入不能因为短暂的 socket 背压静默丢失。worker 的连续无写入
        // 超时会在异常链路上主动断开，因此这里与 SSH/Serial 一样使用无界通道。
        let (command_tx, command_rx) = unbounded_channel::<TelnetCommand>();
        let (resize_tx, resize_rx) = watch::channel((80_u16, 24_u16));
        let shutdown = CancellationToken::new();

        // 断开通知由 worker 直接通过 on_disconnect 发送，不依赖终端输出队列
        // 的排空回调，避免 ingress 背压延迟断线状态更新。
        let parser_ingress = TelnetParserIngress::spawn_with_recording(
            term,
            event_proxy.clone(),
            performance_metrics.clone(),
            None,
            recording_tap.clone(),
        );

        // alacritty 生成的终端响应（DA/颜色/尺寸查询）需要回写服务端。
        // 事件代理接口保持 UnboundedSender；该通道由 GPUI 终端锁内代码写入，
        // 并且 worker 只在此处 receive，因此不会反向阻塞终端。
        let (pty_write_tx, pty_write_rx) = unbounded_channel::<Vec<u8>>();
        event_proxy.set_ssh_write_back(pty_write_tx);

        let worker_shutdown = shutdown.clone();
        let worker_producer = parser_ingress.producer();
        tokio::spawn(async move {
            run_telnet_worker(
                stream,
                login_script,
                command_rx,
                resize_rx,
                pty_write_rx,
                worker_producer,
                worker_shutdown,
                recording_tap,
                on_disconnect,
            )
            .await;
        });

        Ok(Self {
            command_tx,
            resize_tx,
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
        let _ = self.command_tx.send(TelnetCommand::Shutdown);
    }
}

impl TerminalBackend for TelnetBackend {
    fn write(&self, data: Vec<u8>) {
        self.performance_metrics
            .record_input(TerminalInputMetricSource::User, data.len());
        if let Err(error) = self.command_tx.send(TelnetCommand::Write {
            source: TerminalInputSource::User,
            data,
        }) {
            tracing::debug!(
                target: "terminal.telnet.input",
                error = %error,
                "Telnet worker 已关闭，忽略用户输入"
            );
        }
    }

    fn input_handle(&self) -> Option<TerminalInputHandle> {
        let tx = self.command_tx.clone();
        Some(TerminalInputHandle::with_metrics(
            self.performance_metrics.clone(),
            move |data| {
                if let Err(error) = tx.send(TelnetCommand::Write {
                    source: TerminalInputSource::ExternalInput,
                    data,
                }) {
                    tracing::debug!(
                        target: "terminal.telnet.input",
                        error = %error,
                        "Telnet worker 已关闭，忽略外部输入"
                    );
                }
            },
        ))
    }

    fn resize(&self, size: TerminalSize) {
        // watch 通道天然合并尺寸，只保留最新值，不会因终端快速 resize 无界堆积。
        let _ = self.resize_tx.send((size.cols.max(2), size.rows.max(2)));
    }

    fn shutdown(&self) {
        self.stop();
    }
}

impl Drop for TelnetBackend {
    fn drop(&mut self) {
        self.stop();
    }
}

async fn connect_telnet_stream(params: &TelnetParams) -> anyhow::Result<TcpStream> {
    let endpoint = (params.host.as_str(), params.port);
    match tokio::time::timeout(TELNET_CONNECT_TIMEOUT, TcpStream::connect(endpoint)).await {
        Ok(Ok(stream)) => {
            if let Err(error) = stream.set_nodelay(true) {
                tracing::warn!(
                    target: "terminal.telnet.runtime",
                    error = %error,
                    "设置 Telnet TCP_NODELAY 失败"
                );
            }
            Ok(stream)
        }
        Ok(Err(error)) => Err(anyhow::anyhow!(
            "连接 {}:{} 失败: {error}",
            params.host,
            params.port
        )),
        Err(_) => Err(anyhow::anyhow!("连接 {}:{} 超时", params.host, params.port)),
    }
}

async fn write_telnet_bytes(
    writer: &mut (impl AsyncWrite + Unpin),
    bytes: &[u8],
    operation: &'static str,
    shutdown: &CancellationToken,
) -> Result<(), TelnetDisconnectReason> {
    write_telnet_bytes_with_timeout(
        writer,
        bytes,
        operation,
        shutdown,
        TELNET_WRITE_STALL_TIMEOUT,
    )
    .await
}

async fn write_telnet_bytes_with_timeout(
    writer: &mut (impl AsyncWrite + Unpin),
    mut bytes: &[u8],
    operation: &'static str,
    shutdown: &CancellationToken,
    stall_timeout: Duration,
) -> Result<(), TelnetDisconnectReason> {
    while !bytes.is_empty() {
        let written = tokio::select! {
            biased;
            _ = shutdown.cancelled() => return Err(TelnetDisconnectReason::Shutdown),
            result = tokio::time::timeout(stall_timeout, writer.write(bytes)) => {
                match result {
                    Ok(Ok(0)) => {
                        return Err(TelnetDisconnectReason::Write {
                            operation,
                            error: io::Error::new(
                                io::ErrorKind::WriteZero,
                                "无法继续写入 Telnet socket",
                            ),
                        });
                    }
                    Ok(Ok(written)) => written,
                    Ok(Err(error)) => {
                        return Err(TelnetDisconnectReason::Write { operation, error });
                    }
                    Err(_) => {
                        return Err(TelnetDisconnectReason::Write {
                            operation,
                            error: io::Error::new(
                                io::ErrorKind::TimedOut,
                                format!(
                                    "连续 {} 秒无写入进展",
                                    stall_timeout.as_secs_f64()
                                ),
                            ),
                        });
                    }
                }
            }
        };
        bytes = &bytes[written..];
    }
    Ok(())
}

async fn shutdown_telnet_writer(writer: &mut (impl AsyncWrite + Unpin)) {
    match tokio::time::timeout(TELNET_SHUTDOWN_TIMEOUT, writer.shutdown()).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            tracing::debug!(
                target: "terminal.telnet.runtime",
                error = %error,
                "关闭 Telnet socket 写半部失败"
            );
        }
        Err(_) => {
            tracing::debug!(
                target: "terminal.telnet.runtime",
                timeout_ms = TELNET_SHUTDOWN_TIMEOUT.as_millis(),
                "关闭 Telnet socket 写半部超时"
            );
        }
    }
}

type TelnetPendingCrFlush = Pin<Box<tokio::time::Sleep>>;

fn arm_pending_client_cr_flush(pending_flush: &mut Option<TelnetPendingCrFlush>) {
    *pending_flush = Some(Box::pin(tokio::time::sleep(TELNET_NVT_CR_FLUSH_DELAY)));
}

async fn wait_for_pending_client_cr(pending_flush: &mut Option<TelnetPendingCrFlush>) {
    pending_flush
        .as_mut()
        .expect("pending Telnet CR flush should exist when polled")
        .as_mut()
        .await;
}

async fn flush_pending_client_cr(
    writer: &mut (impl AsyncWrite + Unpin),
    codec: &mut TelnetCodec,
    record_pending_cr: &mut bool,
    pending_flush: &mut Option<TelnetPendingCrFlush>,
    recording_tap: Option<&RecordingTap>,
    shutdown: &CancellationToken,
) -> Result<(), TelnetDisconnectReason> {
    let encoded = codec.flush_client_data();
    if !encoded.is_empty() {
        write_telnet_bytes(writer, &encoded, "NVT CR flush", shutdown).await?;
        if *record_pending_cr {
            if let Some(tap) = recording_tap {
                let _ = tap.record_input(&encoded);
            }
        }
    }
    *record_pending_cr = false;
    pending_flush.take();
    Ok(())
}

async fn write_telnet_protocol_bytes(
    writer: &mut (impl AsyncWrite + Unpin),
    codec: &mut TelnetCodec,
    bytes: &[u8],
    operation: &'static str,
    record_pending_cr: &mut bool,
    pending_flush: &mut Option<TelnetPendingCrFlush>,
    recording_tap: Option<&RecordingTap>,
    shutdown: &CancellationToken,
) -> Result<(), TelnetDisconnectReason> {
    // Telnet 命令可以插入数据流，但不能插到 NVT 的 CR NUL/CR LF 中间。
    // 因此在协商/NAWS 等协议帧前先提交待决 CR。
    flush_pending_client_cr(
        writer,
        codec,
        record_pending_cr,
        pending_flush,
        recording_tap,
        shutdown,
    )
    .await?;
    write_telnet_bytes(writer, bytes, operation, shutdown).await
}

async fn write_telnet_client_data(
    writer: &mut (impl AsyncWrite + Unpin),
    codec: &mut TelnetCodec,
    bytes: &[u8],
    operation: &'static str,
    record_input: bool,
    record_pending_cr: &mut bool,
    pending_flush: &mut Option<TelnetPendingCrFlush>,
    recording_tap: Option<&RecordingTap>,
    shutdown: &CancellationToken,
) -> Result<(), TelnetDisconnectReason> {
    // 空输入不应消费已有的 NVT CR，也不应改变其 recording 归属或 deadline。
    if bytes.is_empty() {
        return Ok(());
    }

    // 若待决 CR 与当前数据的录制策略不同，先单独 flush，避免自动登录、
    // 外部输入或终端响应消费用户 CR 后造成漏录/误录。
    if codec.has_pending_client_cr() && *record_pending_cr != record_input {
        flush_pending_client_cr(
            writer,
            codec,
            record_pending_cr,
            pending_flush,
            recording_tap,
            shutdown,
        )
        .await?;
    }

    let encoded = codec.encode_client_data(bytes);
    if !encoded.is_empty() {
        write_telnet_bytes(writer, &encoded, operation, shutdown).await?;
        if record_input {
            if let Some(tap) = recording_tap {
                let _ = tap.record_input(&encoded);
            }
        }
    }
    *record_pending_cr = record_input && codec.has_pending_client_cr();
    if codec.has_pending_client_cr() {
        // 非空输入会消费旧的待决 CR；若编码后仍有待决 CR，它必然来自
        // 当前 chunk，因此从当前时刻重新开始 2ms 组合窗口。
        arm_pending_client_cr_flush(pending_flush);
    } else {
        pending_flush.take();
    }
    Ok(())
}

type TelnetPendingIngress =
    Pin<Box<dyn Future<Output = Result<(), TerminalDataSendError>> + Send + 'static>>;

fn pending_ingress_send(producer: TelnetIngressProducer, data: Vec<u8>) -> TelnetPendingIngress {
    Box::pin(async move { producer.send_data(data).await })
}

async fn wait_for_pending_ingress(
    pending_ingress: &mut Option<TelnetPendingIngress>,
) -> Result<(), TerminalDataSendError> {
    pending_ingress
        .as_mut()
        .expect("pending Telnet ingress should exist when polled")
        .as_mut()
        .await
}

/// Telnet 传输工作线程：协议编解码、登录脚本、读写循环。
async fn run_telnet_worker(
    stream: TcpStream,
    mut login_script: TelnetLoginScript,
    mut command_rx: UnboundedReceiver<TelnetCommand>,
    mut resize_rx: watch::Receiver<(u16, u16)>,
    mut pty_write_rx: UnboundedReceiver<Vec<u8>>,
    producer: TelnetIngressProducer,
    shutdown: CancellationToken,
    recording_tap: Option<RecordingTap>,
    on_disconnect: Option<UnboundedSender<Option<String>>>,
) {
    let (mut reader, mut writer) = stream.into_split();
    let mut codec = TelnetCodec::new(80, 24);
    let mut buffer = vec![0_u8; TELNET_READ_BUFFER_BYTES];
    // ingress 满时暂存当前读块，停止 transport read 但仍继续处理控制命令。
    let mut pending_ingress: Option<TelnetPendingIngress> = None;
    // 当前可录制输入是否有待 flush 的 NVT CR。
    let mut record_pending_cr = false;
    // 待决 CR 使用持久 Sleep，避免 worker 每轮 select 重建计时器后在持续
    // 读写负载下永远到不了截止时间。
    let mut pending_cr_flush: Option<TelnetPendingCrFlush> = None;

    let disconnect_reason = loop {
        let has_pending_ingress = pending_ingress.is_some();
        let has_pending_cr_flush = pending_cr_flush.is_some();

        tokio::select! {
            read_result = reader.read(&mut buffer), if pending_ingress.is_none() => {
                match read_result {
                    Ok(0) => break TelnetDisconnectReason::Eof,
                    Ok(read_count) => {
                        let (data, responses) = codec.filter_server_bytes(&buffer[..read_count]);
                        let mut response_write_error = None;
                        for response in responses {
                            if let Err(reason) = write_telnet_protocol_bytes(
                                &mut writer,
                                &mut codec,
                                &response,
                                "协商应答/子协商应答",
                                &mut record_pending_cr,
                                &mut pending_cr_flush,
                                recording_tap.as_ref(),
                                &shutdown,
                            )
                            .await
                            {
                                response_write_error = Some(reason);
                                break;
                            }
                        }
                        if let Some(reason) = response_write_error {
                            break reason;
                        }

                        let script_sends = login_script.advance(&data);
                        if !data.is_empty() {
                            match producer.try_send_data(data) {
                                Ok(()) => {}
                                Err(TerminalDataSendError::Full(data)) => {
                                    pending_ingress =
                                        Some(pending_ingress_send(producer.clone(), data));
                                }
                                Err(TerminalDataSendError::Closed(_)) => {
                                    break TelnetDisconnectReason::Shutdown;
                                }
                                Err(error @ (TerminalDataSendError::Empty(_)
                                | TerminalDataSendError::Oversized { .. })) => {
                                    tracing::warn!(
                                        target: "terminal.telnet.ingress",
                                        error = %error,
                                        "Telnet 终端输出数据无法投递"
                                    );
                                    break TelnetDisconnectReason::OutputQueue(error.to_string());
                                }
                            }
                        }

                        let mut script_write_error = None;
                        for send in script_sends {
                            if let Err(reason) = write_telnet_client_data(
                                &mut writer,
                                &mut codec,
                                &send,
                                "登录脚本发送",
                                false,
                                &mut record_pending_cr,
                                &mut pending_cr_flush,
                                recording_tap.as_ref(),
                                &shutdown,
                            )
                            .await
                            {
                                script_write_error = Some(reason);
                                break;
                            }
                        }
                        if let Some(reason) = script_write_error {
                            break reason;
                        }
                    }
                    Err(error) => break TelnetDisconnectReason::Read(error),
                }
            }
            command = command_rx.recv() => {
                match command {
                    Some(TelnetCommand::Write { source, data }) => {
                        if let Err(reason) = write_telnet_client_data(
                            &mut writer,
                            &mut codec,
                            &data,
                            "用户输入发送",
                            source.is_recordable_user_input(),
                            &mut record_pending_cr,
                            &mut pending_cr_flush,
                            recording_tap.as_ref(),
                            &shutdown,
                        )
                        .await
                        {
                            break reason;
                        }
                    }
                    Some(TelnetCommand::Shutdown) | None => {
                        // 关闭控制信号采用强制取消语义：不再提交待决 CR，避免
                        // 对端已停止读取时为 flush 额外等待完整写停滞超时。
                        break TelnetDisconnectReason::Shutdown;
                    }
                }
            }
            resize = resize_rx.changed() => {
                if resize.is_err() {
                    break TelnetDisconnectReason::Shutdown;
                }
                let (cols, rows) = *resize_rx.borrow();
                codec.set_window_size(cols, rows);
                if let Some(message) = codec.naws_message() {
                    if let Err(reason) = write_telnet_protocol_bytes(
                        &mut writer,
                        &mut codec,
                        &message,
                        "NAWS 窗口尺寸更新",
                        &mut record_pending_cr,
                        &mut pending_cr_flush,
                        recording_tap.as_ref(),
                        &shutdown,
                    )
                    .await
                    {
                        break reason;
                    }
                }
            }
            response = pty_write_rx.recv() => {
                let Some(response) = response else {
                    break TelnetDisconnectReason::Shutdown;
                };
                if let Err(reason) = write_telnet_client_data(
                    &mut writer,
                    &mut codec,
                    &response,
                    "终端响应回写",
                    false,
                    &mut record_pending_cr,
                    &mut pending_cr_flush,
                    recording_tap.as_ref(),
                    &shutdown,
                )
                .await
                {
                    break reason;
                }
            }
            _ = wait_for_pending_client_cr(&mut pending_cr_flush), if has_pending_cr_flush => {
                if let Err(reason) = flush_pending_client_cr(
                    &mut writer,
                    &mut codec,
                    &mut record_pending_cr,
                    &mut pending_cr_flush,
                    recording_tap.as_ref(),
                    &shutdown,
                )
                .await
                {
                    break reason;
                }
            }
            result = wait_for_pending_ingress(&mut pending_ingress), if has_pending_ingress => {
                pending_ingress.take();
                match result {
                    Ok(()) => {}
                    Err(TerminalDataSendError::Closed(_)) => {
                        break TelnetDisconnectReason::Shutdown;
                    }
                    Err(error @ (TerminalDataSendError::Empty(_)
                    | TerminalDataSendError::Oversized { .. }
                    | TerminalDataSendError::Full(_))) => {
                        tracing::warn!(
                            target: "terminal.telnet.ingress",
                            error = %error,
                            "Telnet 待决终端输出数据无法投递"
                        );
                        break TelnetDisconnectReason::OutputQueue(error.to_string());
                    }
                }
            }
            _ = shutdown.cancelled() => {
                // stop() 已进入强制取消语义，不再尝试发送可能阻塞的待决 CR。
                break TelnetDisconnectReason::Shutdown;
            }
        }
    };

    // 任意退出原因都尽力以独立短超时关闭写半部。写错误、读错误和输出背压
    // 等路径同样不应仅靠 drop 关闭 socket，避免遗漏 FIN；关闭失败不覆盖原始
    // disconnect reason。
    shutdown_telnet_writer(&mut writer).await;

    let disconnect_detail = disconnect_reason.user_message();
    if let Some(detail) = &disconnect_detail {
        tracing::info!(
            target: "terminal.telnet.runtime",
            error = %detail,
            "Telnet worker disconnected with an error"
        );
    }
    if let Some(tx) = on_disconnect {
        let _ = tx.send(disconnect_detail.clone());
    }
    if let Some(detail) = &disconnect_detail {
        let message = format!("\r\nTelnet 连接中断: {detail}\r\n");
        let _ = producer.try_send_data(message.into_bytes());
    }
    producer.close_source();
}
#[cfg(test)]
mod tests {
    use super::*;
    use alacritty_terminal::grid::Dimensions;
    use alacritty_terminal::term::Config as TermConfig;
    use std::task::{Context, Poll};

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

    fn terminal_harness() -> (
        Arc<FairMutex<Term<GpuiEventProxy>>>,
        GpuiEventProxy,
        Arc<TerminalPerformanceMetrics>,
        tokio::sync::mpsc::UnboundedReceiver<crate::pty_backend::TerminalEvent>,
    ) {
        let (event_tx, event_rx) = unbounded_channel();
        let metrics = Arc::new(TerminalPerformanceMetrics::enabled());
        let event_proxy = GpuiEventProxy::with_metrics(event_tx, metrics.clone());
        let term = Arc::new(FairMutex::new(Term::new(
            TermConfig::default(),
            &TestDimensions,
            event_proxy.clone(),
        )));
        (term, event_proxy, metrics, event_rx)
    }

    struct SlowProgressWriter {
        delay: Duration,
        sleep: Option<Pin<Box<tokio::time::Sleep>>>,
        written: Vec<u8>,
    }

    struct WriteZeroWriter;

    impl SlowProgressWriter {
        fn new(delay: Duration) -> Self {
            Self {
                delay,
                sleep: None,
                written: Vec::new(),
            }
        }
    }

    impl AsyncWrite for SlowProgressWriter {
        fn poll_write(
            self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buffer: &[u8],
        ) -> Poll<io::Result<usize>> {
            let this = self.get_mut();
            if buffer.is_empty() {
                return Poll::Ready(Ok(0));
            }

            if this.sleep.is_none() {
                this.sleep = Some(Box::pin(tokio::time::sleep(this.delay)));
            }
            let sleep = this.sleep.as_mut().expect("slow writer sleep should exist");
            if sleep.as_mut().poll(cx).is_pending() {
                return Poll::Pending;
            }

            this.sleep = None;
            this.written.push(buffer[0]);
            Poll::Ready(Ok(1))
        }

        fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }

        fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    impl AsyncWrite for WriteZeroWriter {
        fn poll_write(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            _buffer: &[u8],
        ) -> Poll<io::Result<usize>> {
            Poll::Ready(Ok(0))
        }

        fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }

        fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    fn enable_terminal_type(codec: &mut TelnetCodec) {
        let (_, responses) = codec.filter_server_bytes(&[
            TELNET_COMMAND_IAC,
            TELNET_COMMAND_DO,
            TELNET_OPTION_TERMINAL_TYPE,
        ]);
        assert!(responses.contains(&vec![
            TELNET_COMMAND_IAC,
            TELNET_COMMAND_WILL,
            TELNET_OPTION_TERMINAL_TYPE
        ]));
    }

    #[test]
    fn telnet_codec_filters_negotiation_and_answers_supported_options() {
        let mut codec = TelnetCodec::new(80, 24);
        let (data, responses) = codec.filter_server_bytes(&[
            b'h',
            TELNET_COMMAND_IAC,
            TELNET_COMMAND_DO,
            TELNET_OPTION_NAWS,
            TELNET_COMMAND_IAC,
            TELNET_COMMAND_WILL,
            TELNET_OPTION_ECHO,
            b'i',
        ]);

        assert_eq!(data, b"hi");
        assert!(responses.contains(&vec![
            TELNET_COMMAND_IAC,
            TELNET_COMMAND_WILL,
            TELNET_OPTION_NAWS
        ]));
        assert!(responses.contains(&vec![
            TELNET_COMMAND_IAC,
            TELNET_COMMAND_DO,
            TELNET_OPTION_ECHO
        ]));
        assert!(responses.iter().any(|response| response.starts_with(&[
            TELNET_COMMAND_IAC,
            TELNET_COMMAND_SB,
            TELNET_OPTION_NAWS
        ])));
    }

    #[test]
    fn telnet_codec_answers_are_you_there() {
        let mut codec = TelnetCodec::new(80, 24);

        let (data, responses) =
            codec.filter_server_bytes(&[TELNET_COMMAND_IAC, TELNET_COMMAND_AYT]);

        assert!(data.is_empty());
        assert_eq!(responses, vec![TELNET_AYT_RESPONSE.to_vec()]);
    }

    #[test]
    fn telnet_codec_buffers_incomplete_negotiation_across_reads() {
        let mut codec = TelnetCodec::new(80, 24);

        let (data, responses) = codec.filter_server_bytes(&[b'a', TELNET_COMMAND_IAC]);
        assert_eq!(data, b"a");
        assert!(responses.is_empty());

        let (data, responses) =
            codec.filter_server_bytes(&[TELNET_COMMAND_DO, TELNET_OPTION_NAWS, b'b']);
        assert_eq!(data, b"b");
        assert!(responses.contains(&vec![
            TELNET_COMMAND_IAC,
            TELNET_COMMAND_WILL,
            TELNET_OPTION_NAWS
        ]));
        assert!(codec.naws_message().is_some());
    }

    #[test]
    fn telnet_codec_buffers_subnegotiation_across_reads() {
        let mut codec = TelnetCodec::new(80, 24);
        enable_terminal_type(&mut codec);

        let (data, responses) = codec.filter_server_bytes(&[
            TELNET_COMMAND_IAC,
            TELNET_COMMAND_SB,
            TELNET_OPTION_TERMINAL_TYPE,
        ]);
        assert!(data.is_empty());
        assert!(responses.is_empty());

        let (data, responses) = codec.filter_server_bytes(&[
            TELNET_TERMINAL_TYPE_SEND,
            TELNET_COMMAND_IAC,
            TELNET_COMMAND_SE,
            b'x',
        ]);
        assert_eq!(data, b"x");
        assert_eq!(1, responses.len());
        assert!(responses[0].starts_with(&[
            TELNET_COMMAND_IAC,
            TELNET_COMMAND_SB,
            TELNET_OPTION_TERMINAL_TYPE,
            TELNET_TERMINAL_TYPE_IS,
        ]));
    }

    #[test]
    fn telnet_codec_answers_terminal_type_subnegotiation() {
        let mut codec = TelnetCodec::new(80, 24);
        enable_terminal_type(&mut codec);
        let (data, responses) = codec.filter_server_bytes(&[
            TELNET_COMMAND_IAC,
            TELNET_COMMAND_SB,
            TELNET_OPTION_TERMINAL_TYPE,
            TELNET_TERMINAL_TYPE_SEND,
            TELNET_COMMAND_IAC,
            TELNET_COMMAND_SE,
        ]);

        assert!(data.is_empty());
        assert_eq!(1, responses.len());
        assert!(responses[0].starts_with(&[
            TELNET_COMMAND_IAC,
            TELNET_COMMAND_SB,
            TELNET_OPTION_TERMINAL_TYPE,
            TELNET_TERMINAL_TYPE_IS,
        ]));
        assert!(
            responses[0]
                .windows(TELNET_TERMINAL_TYPE.len())
                .any(|window| window == TELNET_TERMINAL_TYPE)
        );
        assert!(responses[0].ends_with(&[TELNET_COMMAND_IAC, TELNET_COMMAND_SE]));
    }

    #[test]
    fn telnet_codec_unescapes_iac_iac_inside_subnegotiation() {
        let mut codec = TelnetCodec::new(80, 24);
        enable_terminal_type(&mut codec);

        // IAC IAC 在 SB payload 内应反转义为单个 0xFF。
        codec.filter_server_bytes(&[
            TELNET_COMMAND_IAC,
            TELNET_COMMAND_SB,
            TELNET_OPTION_TERMINAL_TYPE,
            TELNET_TERMINAL_TYPE_SEND,
            TELNET_COMMAND_IAC,
            TELNET_COMMAND_IAC,
            TELNET_COMMAND_IAC,
            TELNET_COMMAND_SE,
        ]);

        assert_eq!(
            codec.last_subnegotiation_payload.as_deref(),
            Some(&[TELNET_OPTION_TERMINAL_TYPE, TELNET_TERMINAL_TYPE_SEND, 0xFF][..])
        );
    }

    #[test]
    fn telnet_codec_escapes_client_iac_bytes() {
        let mut codec = TelnetCodec::new(80, 24);
        assert_eq!(
            codec.encode_client_data(&[b'a', TELNET_COMMAND_IAC, b'b']),
            vec![b'a', TELNET_COMMAND_IAC, TELNET_COMMAND_IAC, b'b']
        );
    }

    #[test]
    fn telnet_codec_rejects_unsupported_options_once() {
        let mut codec = TelnetCodec::new(80, 24);
        let (data, responses) =
            codec.filter_server_bytes(&[TELNET_COMMAND_IAC, TELNET_COMMAND_DO, TELNET_OPTION_ECHO]);
        assert!(data.is_empty());
        assert!(responses.contains(&vec![
            TELNET_COMMAND_IAC,
            TELNET_COMMAND_WONT,
            TELNET_OPTION_ECHO
        ]));

        let (_, responses) =
            codec.filter_server_bytes(&[TELNET_COMMAND_IAC, TELNET_COMMAND_DO, TELNET_OPTION_ECHO]);
        assert!(responses.is_empty(), "已拒绝的重复 DO 不应反复产生 WONT");
    }

    #[test]
    fn telnet_negotiation_is_idempotent_and_directions_are_independent() {
        let mut codec = TelnetCodec::new(80, 24);

        let (_, responses) =
            codec.filter_server_bytes(&[TELNET_COMMAND_IAC, TELNET_COMMAND_DO, TELNET_OPTION_NAWS]);
        assert_eq!(2, responses.len(), "首次 DO NAWS 应回复 WILL + NAWS");
        let (_, responses) =
            codec.filter_server_bytes(&[TELNET_COMMAND_IAC, TELNET_COMMAND_DO, TELNET_OPTION_NAWS]);
        assert!(responses.is_empty(), "重复 DO NAWS 不应重复应答");

        let (_, responses) = codec.filter_server_bytes(&[
            TELNET_COMMAND_IAC,
            TELNET_COMMAND_DONT,
            TELNET_OPTION_NAWS,
        ]);
        assert_eq!(
            responses,
            vec![vec![
                TELNET_COMMAND_IAC,
                TELNET_COMMAND_WONT,
                TELNET_OPTION_NAWS
            ]]
        );
        let (_, responses) = codec.filter_server_bytes(&[
            TELNET_COMMAND_IAC,
            TELNET_COMMAND_DONT,
            TELNET_OPTION_NAWS,
        ]);
        assert!(responses.is_empty(), "重复 DONT 不应重复 WONT");

        // 远端 BINARY 独立于本端 BINARY。
        let (_, responses) = codec.filter_server_bytes(&[
            TELNET_COMMAND_IAC,
            TELNET_COMMAND_WILL,
            TELNET_OPTION_BINARY,
        ]);
        assert_eq!(
            responses,
            vec![vec![
                TELNET_COMMAND_IAC,
                TELNET_COMMAND_DO,
                TELNET_OPTION_BINARY
            ]]
        );
        let (_, responses) = codec.filter_server_bytes(&[
            TELNET_COMMAND_IAC,
            TELNET_COMMAND_WILL,
            TELNET_OPTION_BINARY,
        ]);
        assert!(responses.is_empty(), "重复 WILL BINARY 不应重复 DO");

        // 远端 WONT 不影响本端发送编码；本端仍按 NVT 编码。
        codec.filter_server_bytes(&[
            TELNET_COMMAND_IAC,
            TELNET_COMMAND_WONT,
            TELNET_OPTION_BINARY,
        ]);
        assert_eq!(codec.encode_client_data(b"\r"), Vec::<u8>::new());
        assert_eq!(codec.flush_client_data(), b"\r\0");
    }

    #[test]
    fn telnet_nvt_encoder_handles_cr_lf_cr_nul_and_cross_chunk_cr() {
        let mut codec = TelnetCodec::new(80, 24);

        assert_eq!(codec.encode_client_data(b"a\r\nb"), b"a\r\nb");
        assert_eq!(codec.encode_client_data(b"a\rb"), b"a\r\0b");

        // 跨 write() 的 CR LF 边界。
        assert_eq!(codec.encode_client_data(b"line\r"), b"line");
        assert_eq!(codec.encode_client_data(b"\nnext"), b"\r\nnext");

        // 单独的 Enter 最终 flush 为 CR NUL。
        assert_eq!(codec.encode_client_data(b"admin\r"), b"admin");
        assert!(codec.has_pending_client_cr());
        assert_eq!(codec.flush_client_data(), b"\r\0");
    }

    #[test]
    fn telnet_binary_enables_raw_cr_but_still_escapes_iac() {
        let mut codec = TelnetCodec::new(80, 24);
        codec.filter_server_bytes(&[TELNET_COMMAND_IAC, TELNET_COMMAND_DO, TELNET_OPTION_BINARY]);

        assert_eq!(codec.encode_client_data(b"\r"), b"\r");
        assert_eq!(codec.flush_client_data(), Vec::<u8>::new());
        assert_eq!(
            codec.encode_client_data(&[TELNET_COMMAND_IAC]),
            vec![TELNET_COMMAND_IAC, TELNET_COMMAND_IAC]
        );
    }

    #[test]
    fn telnet_pending_nvt_cr_keeps_pre_binary_encoding() {
        let mut codec = TelnetCodec::new(80, 24);
        assert!(codec.encode_client_data(b"\r").is_empty());
        assert!(codec.has_pending_client_cr());

        codec.filter_server_bytes(&[TELNET_COMMAND_IAC, TELNET_COMMAND_DO, TELNET_OPTION_BINARY]);

        assert_eq!(
            codec.flush_client_data(),
            b"\r\0",
            "BINARY 协商不应把协商前暂存的 NVT CR 改成裸 CR"
        );
    }

    #[test]
    fn telnet_oversized_subnegotiation_discards_until_valid_iac_se() {
        let mut codec = TelnetCodec::new(80, 24);
        let (data, _) = codec.filter_server_bytes(&[
            TELNET_COMMAND_IAC,
            TELNET_COMMAND_SB,
            TELNET_OPTION_TERMINAL_TYPE,
        ]);
        assert!(data.is_empty());

        let (data, _) = codec.filter_server_bytes(&[b'x'; TELNET_MAX_PENDING_BYTES + 16]);
        assert!(data.is_empty(), "超限后 SB payload 不应泄漏进终端");

        let (data, responses) = codec.filter_server_bytes(&[
            b'l',
            b'e',
            b'a',
            b'k',
            TELNET_COMMAND_IAC,
            TELNET_COMMAND_SE,
            b'o',
            b'k',
        ]);
        assert_eq!(data, b"ok", "丢弃状态应持续到合法 IAC SE");
        assert!(responses.is_empty());
    }

    #[test]
    fn telnet_naws_message_waits_for_server_negotiation_and_escapes_iac_payload() {
        let mut codec = TelnetCodec::new(80, 24);
        assert!(codec.naws_message().is_none());

        // 服务端 DO NAWS 后才允许发送窗口尺寸。
        codec.filter_server_bytes(&[TELNET_COMMAND_IAC, TELNET_COMMAND_DO, TELNET_OPTION_NAWS]);

        // 让宽高字节恰好包含 0xFF，验证子协商载荷内的 IAC 转义。
        codec.set_window_size(0x01FF, 24);
        let message = codec.naws_message().expect("NAWS 已协商");
        let payload = &message[3..message.len() - 2];
        assert_eq!(payload.len(), 5, "0xFF 低字节应被转义为两个字节");
        assert_eq!(payload[0], 0x01);
        assert_eq!(payload[1], TELNET_COMMAND_IAC);
        assert_eq!(payload[2], TELNET_COMMAND_IAC);
        assert_eq!(payload[3], 0x00);
        assert_eq!(payload[4], 0x18);
    }

    #[test]
    fn disconnect_reason_distinguishes_eof_from_write_errors() {
        assert_eq!(TelnetDisconnectReason::Eof.user_message(), None);
        assert_eq!(TelnetDisconnectReason::Shutdown.user_message(), None);

        let write_error = io::Error::new(io::ErrorKind::BrokenPipe, "broken pipe");
        let message = TelnetDisconnectReason::Write {
            operation: "用户输入发送",
            error: write_error,
        }
        .user_message()
        .expect("write error should be visible");
        assert!(message.contains("用户输入发送"));
        assert!(message.contains("broken pipe"));

        let read_error = io::Error::new(io::ErrorKind::ConnectionReset, "reset");
        assert!(
            TelnetDisconnectReason::Read(read_error)
                .user_message()
                .expect("read error should be visible")
                .contains("读取 Telnet 数据失败")
        );
    }

    #[tokio::test]
    async fn telnet_connect_enables_tcp_nodelay() {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind Telnet test listener");
        let address = listener.local_addr().expect("read Telnet listener address");
        let accept_task = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accept Telnet connection");
            stream
        });
        let params = TelnetParams {
            host: address.ip().to_string(),
            port: address.port(),
            credential_reference: None,
            prompt_username: None,
            prompt_password: None,
            login_script: Vec::new(),
        };

        let stream = connect_telnet_stream(&params)
            .await
            .expect("connect to Telnet test listener");

        assert!(stream.nodelay().expect("read TCP_NODELAY"));
        drop(stream);
        drop(accept_task.await.expect("join Telnet accept task"));
    }

    #[tokio::test]
    async fn telnet_protocol_write_flushes_pending_nvt_cr_first() {
        let (mut writer, mut reader) = tokio::io::duplex(64);
        let shutdown = CancellationToken::new();
        let mut codec = TelnetCodec::new(80, 24);
        let mut record_pending_cr = false;
        let mut pending_cr_flush = None;

        assert!(codec.encode_client_data(b"\r").is_empty());
        write_telnet_protocol_bytes(
            &mut writer,
            &mut codec,
            &[TELNET_COMMAND_IAC, TELNET_COMMAND_WILL, TELNET_OPTION_NAWS],
            "测试协议帧",
            &mut record_pending_cr,
            &mut pending_cr_flush,
            None,
            &shutdown,
        )
        .await
        .expect("协议帧写入应成功");

        let mut received = [0_u8; 5];
        reader
            .read_exact(&mut received)
            .await
            .expect("应收到 CR NUL 和协议帧");
        assert_eq!(
            received,
            [
                b'\r',
                0,
                TELNET_COMMAND_IAC,
                TELNET_COMMAND_WILL,
                TELNET_OPTION_NAWS
            ]
        );
        assert!(!codec.has_pending_client_cr());
        assert!(pending_cr_flush.is_none());
    }

    #[tokio::test]
    async fn telnet_write_stall_timeout_resets_after_each_progress() {
        let mut writer = SlowProgressWriter::new(Duration::from_millis(10));
        let shutdown = CancellationToken::new();
        let bytes = b"slow-but-progressing";
        let stall_timeout = Duration::from_millis(50);
        let started = tokio::time::Instant::now();

        write_telnet_bytes_with_timeout(
            &mut writer,
            bytes,
            "测试慢速写入",
            &shutdown,
            stall_timeout,
        )
        .await
        .expect("每次停滞都短于超时的慢速写入不应被断开");

        assert_eq!(writer.written, bytes);
        assert!(
            started.elapsed() > stall_timeout,
            "测试必须覆盖总写入时长超过单次停滞超时的场景"
        );
    }

    #[tokio::test]
    async fn telnet_write_timeout_disconnects_stalled_writer() {
        let (mut writer, _reader) = tokio::io::duplex(1);
        let shutdown = CancellationToken::new();

        let reason = write_telnet_bytes_with_timeout(
            &mut writer,
            b"blocked",
            "测试超时",
            &shutdown,
            Duration::from_millis(20),
        )
        .await
        .expect_err("写缓冲无人消费时应超时");

        match reason {
            TelnetDisconnectReason::Write { operation, error } => {
                assert_eq!(operation, "测试超时");
                assert_eq!(error.kind(), io::ErrorKind::TimedOut);
            }
            other => panic!("预期写超时，实际为 {other:?}"),
        }
    }

    #[tokio::test]
    async fn telnet_write_zero_is_reported_as_write_error() {
        let mut writer = WriteZeroWriter;
        let shutdown = CancellationToken::new();

        let reason = write_telnet_bytes_with_timeout(
            &mut writer,
            b"data",
            "测试 WriteZero",
            &shutdown,
            Duration::from_secs(1),
        )
        .await
        .expect_err("非空写入返回 0 必须视为 WriteZero");

        match reason {
            TelnetDisconnectReason::Write { operation, error } => {
                assert_eq!(operation, "测试 WriteZero");
                assert_eq!(error.kind(), io::ErrorKind::WriteZero);
            }
            other => panic!("预期 WriteZero，实际为 {other:?}"),
        }
    }

    #[tokio::test]
    async fn telnet_shutdown_cancels_stalled_write() {
        let (mut writer, _reader) = tokio::io::duplex(1);
        let shutdown = CancellationToken::new();
        let cancel = shutdown.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            cancel.cancel();
        });

        let reason = write_telnet_bytes_with_timeout(
            &mut writer,
            b"blocked",
            "测试取消",
            &shutdown,
            Duration::from_secs(5),
        )
        .await
        .expect_err("shutdown 应取消卡住的写入");

        assert!(matches!(reason, TelnetDisconnectReason::Shutdown));
    }

    #[tokio::test]
    async fn telnet_shutdown_closes_writer_even_after_cancellation() {
        let (mut writer, mut reader) = tokio::io::duplex(8);
        let shutdown = CancellationToken::new();
        shutdown.cancel();

        // worker 的 cancellation 分支不再经过 cancellation-aware 数据写入，
        // 而是直接使用独立短超时关闭写半部。
        shutdown_telnet_writer(&mut writer).await;

        let mut byte = [0_u8; 1];
        assert_eq!(
            reader.read(&mut byte).await.expect("读取 shutdown EOF"),
            0,
            "关闭写半部后对端应观察到 EOF"
        );
    }

    #[tokio::test]
    async fn telnet_pending_cr_deadline_tracks_the_latest_pending_cr() {
        let (mut writer, mut reader) = tokio::io::duplex(64);
        let shutdown = CancellationToken::new();
        let mut codec = TelnetCodec::new(80, 24);
        let mut record_pending_cr = false;
        let mut pending_cr_flush = None;

        write_telnet_client_data(
            &mut writer,
            &mut codec,
            b"secret\r",
            "登录脚本发送",
            false,
            &mut record_pending_cr,
            &mut pending_cr_flush,
            None,
            &shutdown,
        )
        .await
        .expect("登录脚本写入应成功");
        assert!(codec.has_pending_client_cr());
        assert!(!record_pending_cr, "登录脚本的待决 CR 不应标记为可录制");
        assert!(pending_cr_flush.is_some());

        write_telnet_client_data(
            &mut writer,
            &mut codec,
            b"",
            "空用户输入",
            true,
            &mut record_pending_cr,
            &mut pending_cr_flush,
            None,
            &shutdown,
        )
        .await
        .expect("空输入应为 no-op");
        assert!(codec.has_pending_client_cr(), "空输入不应消费已有 CR");
        assert!(!record_pending_cr, "空输入不应改变 CR 的录制归属");
        assert!(pending_cr_flush.is_some(), "空输入不应清除固定 deadline");

        // 用户输入与登录脚本录制策略不同，应先提交脚本 CR，再为用户 CR
        // 创建新的固定截止时间，不能复用或丢失旧状态。
        write_telnet_client_data(
            &mut writer,
            &mut codec,
            b"user\r",
            "用户输入发送",
            true,
            &mut record_pending_cr,
            &mut pending_cr_flush,
            None,
            &shutdown,
        )
        .await
        .expect("用户输入写入应成功");

        assert!(codec.has_pending_client_cr());
        assert!(record_pending_cr, "新的用户 CR 应标记为可录制");
        assert!(pending_cr_flush.is_some());

        flush_pending_client_cr(
            &mut writer,
            &mut codec,
            &mut record_pending_cr,
            &mut pending_cr_flush,
            None,
            &shutdown,
        )
        .await
        .expect("待决用户 CR 应可提交");

        let mut received = [0_u8; 16];
        let count = reader.read(&mut received).await.expect("读取编码结果");
        assert_eq!(&received[..count], b"secret\r\0user\r\0");
        assert!(!record_pending_cr);
        assert!(pending_cr_flush.is_none());
    }

    #[tokio::test]
    async fn telnet_worker_answers_are_you_there_over_tcp() {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind Telnet test listener");
        let address = listener.local_addr().expect("read Telnet test address");
        let server_task = tokio::spawn(async move {
            listener
                .accept()
                .await
                .expect("accept Telnet test connection")
                .0
        });

        let client = TcpStream::connect(address)
            .await
            .expect("connect Telnet test client");
        let mut server = server_task.await.expect("join Telnet test server");
        let (term, event_proxy, metrics, _event_rx) = terminal_harness();
        let (drained_tx, mut drained_rx) = unbounded_channel();
        let ingress = TelnetParserIngress::spawn_with_recording(
            term,
            event_proxy,
            metrics,
            Some(drained_tx),
            None,
        );
        let producer = ingress.producer();
        let (command_tx, command_rx) = unbounded_channel();
        let (resize_tx, resize_rx) = watch::channel((80_u16, 24_u16));
        let (pty_write_tx, pty_write_rx) = unbounded_channel();
        let shutdown = CancellationToken::new();
        let (disconnect_tx, mut disconnect_rx) = unbounded_channel();

        let worker = tokio::spawn(run_telnet_worker(
            client,
            TelnetLoginScript::new(&[]).expect("empty Telnet login script"),
            command_rx,
            resize_rx,
            pty_write_rx,
            producer,
            shutdown.clone(),
            None,
            Some(disconnect_tx),
        ));

        server
            .write_all(&[TELNET_COMMAND_IAC, TELNET_COMMAND_AYT])
            .await
            .expect("send AYT request");
        let mut response = vec![0_u8; TELNET_AYT_RESPONSE.len()];
        tokio::time::timeout(Duration::from_secs(5), server.read_exact(&mut response))
            .await
            .expect("AYT response timed out")
            .expect("read AYT response");
        assert_eq!(response, TELNET_AYT_RESPONSE);

        shutdown.cancel();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(5), disconnect_rx.recv())
                .await
                .expect("Telnet worker disconnect timed out"),
            Some(None)
        );
        tokio::time::timeout(Duration::from_secs(5), worker)
            .await
            .expect("Telnet worker join timed out")
            .expect("Telnet worker panicked");
        tokio::time::timeout(Duration::from_secs(5), drained_rx.recv())
            .await
            .expect("Telnet ingress drain timed out")
            .expect("Telnet ingress drain signal missing");

        // Keep all worker input senders alive until the worker has observed the
        // explicit cancellation above, rather than accidentally selecting a
        // closed channel and masking the behavior under test.
        drop((command_tx, resize_tx, pty_write_tx, ingress));
    }

    #[tokio::test]
    async fn telnet_worker_preserves_user_command_order_without_drops() {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind Telnet test listener");
        let address = listener.local_addr().expect("read Telnet test address");
        let server_task = tokio::spawn(async move {
            listener
                .accept()
                .await
                .expect("accept Telnet test connection")
                .0
        });

        let client = TcpStream::connect(address)
            .await
            .expect("connect Telnet test client");
        let mut server = server_task.await.expect("join Telnet test server");
        let (term, event_proxy, metrics, _event_rx) = terminal_harness();
        let (drained_tx, mut drained_rx) = unbounded_channel();
        let ingress = TelnetParserIngress::spawn_with_recording(
            term,
            event_proxy,
            metrics,
            Some(drained_tx),
            None,
        );
        let producer = ingress.producer();
        let (command_tx, command_rx) = unbounded_channel();
        let (resize_tx, resize_rx) = watch::channel((80_u16, 24_u16));
        let (pty_write_tx, pty_write_rx) = unbounded_channel();
        let shutdown = CancellationToken::new();
        let (disconnect_tx, mut disconnect_rx) = unbounded_channel();

        let worker = tokio::spawn(run_telnet_worker(
            client,
            TelnetLoginScript::new(&[]).expect("empty Telnet login script"),
            command_rx,
            resize_rx,
            pty_write_rx,
            producer,
            shutdown,
            None,
            Some(disconnect_tx),
        ));

        let mut expected = Vec::new();
        for index in 0..64 {
            let payload = format!("cmd-{index:03}\n").into_bytes();
            expected.extend_from_slice(&payload);
            command_tx
                .send(TelnetCommand::Write {
                    source: TerminalInputSource::User,
                    data: payload,
                })
                .expect("send Telnet user command");
        }

        let mut received = vec![0_u8; expected.len()];
        tokio::time::timeout(Duration::from_secs(5), server.read_exact(&mut received))
            .await
            .expect("Telnet user command read timed out")
            .expect("read Telnet user commands");
        assert_eq!(received, expected);

        command_tx
            .send(TelnetCommand::Shutdown)
            .expect("send Telnet worker shutdown");
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(5), disconnect_rx.recv())
                .await
                .expect("Telnet worker disconnect timed out"),
            Some(None)
        );
        tokio::time::timeout(Duration::from_secs(5), worker)
            .await
            .expect("Telnet worker join timed out")
            .expect("Telnet worker panicked");
        tokio::time::timeout(Duration::from_secs(5), drained_rx.recv())
            .await
            .expect("Telnet ingress drain timed out")
            .expect("Telnet ingress drain signal missing");

        drop((resize_tx, pty_write_tx, ingress));
    }
}
