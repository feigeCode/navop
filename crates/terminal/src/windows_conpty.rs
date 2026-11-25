//! Windows ConPTY 封装：避免 GUI 子系统启动本地终端时闪出独立系统控制台窗口。
//!
//! `alacritty_terminal 0.26` 的 `tty::windows::conpty::new` 在构建 `STARTUPINFOEXW`
//! 时只设置了 `STARTF_USESTDHANDLES`，未设置 `STARTF_USESHOWWINDOW` + `wShowWindow = SW_HIDE`。
//! 当宿主进程以 `windows_subsystem = "windows"`（无附加控制台）运行时，Windows 在为
//! ConPTY 子进程初始化控制台的瞬间会闪出一个独立的系统终端窗口。
//!
//! 本模块实现 `alacritty_terminal::tty::EventedPty` 与 `OnResize`，在 `CreateProcessW`
//! 时显式传入 `STARTF_USESHOWWINDOW` + `SW_HIDE`，并安全处理关闭后的 `ResizePseudoConsole`
//! 返回值（避免 `assert_eq!(result, S_OK)` 引起事件循环线程 panic）。

use std::collections::{HashMap, HashSet};
use std::ffi::{OsStr, c_void};
use std::io::{self, Error, Read, Write};
use std::iter::once;
use std::marker::PhantomData;
use std::num::NonZeroU32;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::IntoRawHandle;
use std::os::windows::process::ExitStatusExt;
use std::process::ExitStatus;
use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::task::{Context, Poll, Wake, Waker};
use std::{mem, ptr, thread};

use alacritty_terminal::event::{OnResize, WindowSize};
use alacritty_terminal::tty::{ChildEvent, EventedPty, EventedReadWrite, Options};
use miow::pipe::{AnonRead, AnonWrite};
use piper::{Reader as PipeReader, Writer as PipeWriter, pipe};
use polling::os::iocp::{CompletionPacket, PollerIocpExt};
use polling::{Event, PollMode, Poller};
use windows_sys::Win32::Foundation::{CloseHandle, FALSE, HANDLE, S_OK};
use windows_sys::Win32::System::Console::{
    COORD, ClosePseudoConsole, CreatePseudoConsole, HPCON, ResizePseudoConsole,
};
use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows_sys::Win32::System::Threading::{
    CREATE_UNICODE_ENVIRONMENT, CreateProcessW, DeleteProcThreadAttributeList,
    EXTENDED_STARTUPINFO_PRESENT, GetExitCodeProcess, GetProcessId, INFINITE,
    InitializeProcThreadAttributeList, PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE, PROCESS_INFORMATION,
    RegisterWaitForSingleObject, STARTF_USESHOWWINDOW, STARTF_USESTDHANDLES, STARTUPINFOEXW,
    STARTUPINFOW, UnregisterWait, UpdateProcThreadAttribute, WT_EXECUTEINWAITTHREAD,
    WT_EXECUTEONLYONCE,
};
use windows_sys::Win32::UI::WindowsAndMessaging::SW_HIDE;
use windows_sys::core::{HRESULT, PWSTR};
use windows_sys::s;

const PTY_CHILD_EVENT_TOKEN: usize = 1;
const PTY_READ_WRITE_TOKEN: usize = 2;
const PIPE_CAPACITY: usize = 0x10_000;

type CreatePseudoConsoleFn =
    unsafe extern "system" fn(COORD, HANDLE, HANDLE, u32, *mut HPCON) -> HRESULT;
type ResizePseudoConsoleFn = unsafe extern "system" fn(HPCON, COORD) -> HRESULT;
type ClosePseudoConsoleFn = unsafe extern "system" fn(HPCON);

struct ConptyApi {
    create: CreatePseudoConsoleFn,
    resize: ResizePseudoConsoleFn,
    close: ClosePseudoConsoleFn,
}

impl ConptyApi {
    fn new() -> Self {
        match Self::load_conpty() {
            Some(conpty) => {
                tracing::debug!("Using conpty.dll for pseudoconsole");
                conpty
            }
            None => {
                tracing::debug!("Using Windows API for pseudoconsole");
                Self {
                    create: CreatePseudoConsole,
                    resize: ResizePseudoConsole,
                    close: ClosePseudoConsole,
                }
            }
        }
    }

    fn load_conpty() -> Option<Self> {
        // 只从宿主 exe 同目录加载随应用发布的 `conpty.dll`，避免通过 PATH 误加载
        // 第三方终端（如 WezTerm）目录里的 `conpty.dll`。
        // 实测：从 PATH 加载 `C:\Program Files\WezTerm\conpty.dll` 时，它拉起
        // `OpenConsole.exe` 会忽略宿主 `CreateProcessW` 的 `SW_HIDE`，导致 GUI 进程
        // 每次打开本地终端都会闪出一个独立控制台窗口。
        type LoadedFn = unsafe extern "system" fn() -> isize;
        let exe_dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
        let bundled = exe_dir.join("conpty.dll");
        if !bundled.is_file() {
            return None;
        }
        let wide_path = win32_string(&bundled);
        unsafe {
            let hmodule = LoadLibraryW(wide_path.as_ptr());
            if hmodule.is_null() {
                return None;
            }
            let create_fn = GetProcAddress(hmodule, s!("CreatePseudoConsole"))?;
            let resize_fn = GetProcAddress(hmodule, s!("ResizePseudoConsole"))?;
            let close_fn = GetProcAddress(hmodule, s!("ClosePseudoConsole"))?;

            Some(Self {
                create: mem::transmute::<LoadedFn, CreatePseudoConsoleFn>(create_fn),
                resize: mem::transmute::<LoadedFn, ResizePseudoConsoleFn>(resize_fn),
                close: mem::transmute::<LoadedFn, ClosePseudoConsoleFn>(close_fn),
            })
        }
    }
}

struct Conpty {
    handle: HPCON,
    api: ConptyApi,
}

impl Drop for Conpty {
    fn drop(&mut self) {
        // 必须先于 conout 管道释放，否则 ClosePseudoConsole 会阻塞等待管道排空。
        unsafe { (self.api.close)(self.handle) };
    }
}

unsafe impl Send for Conpty {}

impl OnResize for Conpty {
    fn on_resize(&mut self, window_size: WindowSize) {
        let coord = COORD {
            X: window_size.num_cols as i16,
            Y: window_size.num_lines as i16,
        };
        let result = unsafe { (self.api.resize)(self.handle, coord) };
        if result != S_OK {
            tracing::debug!(
                hresult = format_args!("0x{:08x}", result as u32),
                cols = window_size.num_cols,
                rows = window_size.num_lines,
                "ResizePseudoConsole returned non-S_OK"
            );
        }
    }
}

pub(crate) struct WindowsConpty {
    // 字段顺序有意安排：backend 必须先于 conout drop，避免 ClosePseudoConsole 死锁。
    backend: Conpty,
    conout: UnblockedReader<AnonRead>,
    conin: UnblockedWriter<AnonWrite>,
    child_watcher: ChildExitWatcher,
}

pub(crate) fn spawn(config: &Options, window_size: WindowSize) -> io::Result<WindowsConpty> {
    let api = ConptyApi::new();
    let mut pty_handle: HPCON = 0;

    let (conout, conout_pty_handle) = miow::pipe::anonymous(0)?;
    let (conin_pty_handle, conin) = miow::pipe::anonymous(0)?;

    let coord = COORD {
        X: window_size.num_cols as i16,
        Y: window_size.num_lines as i16,
    };
    let result = unsafe {
        (api.create)(
            coord,
            conin_pty_handle.into_raw_handle() as HANDLE,
            conout_pty_handle.into_raw_handle() as HANDLE,
            0,
            &mut pty_handle as *mut _,
        )
    };
    if result != S_OK {
        return Err(io::Error::other(format!(
            "CreatePseudoConsole failed with HRESULT 0x{:08x}",
            result as u32
        )));
    }
    let conpty = Conpty {
        handle: pty_handle,
        api,
    };

    let mut startup_info_ex = build_hidden_conpty_startup_info();
    let mut attr_size: usize = 0;
    unsafe {
        let ok = InitializeProcThreadAttributeList(ptr::null_mut(), 1, 0, &mut attr_size) > 0;
        if ok {
            return Err(Error::last_os_error());
        }
    }

    let mut attr_list: Box<[u8]> = vec![0; attr_size].into_boxed_slice();
    #[allow(clippy::cast_ptr_alignment)]
    {
        startup_info_ex.lpAttributeList = attr_list.as_mut_ptr() as _;
    }

    unsafe {
        if InitializeProcThreadAttributeList(
            startup_info_ex.lpAttributeList,
            1,
            0,
            &mut attr_size,
        ) == 0
        {
            return Err(Error::last_os_error());
        }
    }

    struct AttrListGuard(windows_sys::Win32::System::Threading::LPPROC_THREAD_ATTRIBUTE_LIST);
    impl Drop for AttrListGuard {
        fn drop(&mut self) {
            unsafe {
                DeleteProcThreadAttributeList(self.0);
            }
        }
    }
    let _attr_guard = AttrListGuard(startup_info_ex.lpAttributeList);

    unsafe {
        if UpdateProcThreadAttribute(
            startup_info_ex.lpAttributeList,
            0,
            PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE as usize,
            conpty.handle as *mut c_void,
            mem::size_of::<HPCON>(),
            ptr::null_mut(),
            ptr::null_mut(),
        ) == 0
        {
            return Err(Error::last_os_error());
        }
    }

    let cmdline_str = win32_string(&cmdline(config));
    let cwd = config.working_directory.as_ref().map(win32_string);
    let mut creation_flags = EXTENDED_STARTUPINFO_PRESENT;
    let custom_env_block = convert_custom_env(&config.env);
    let custom_env_block_pointer = match &custom_env_block {
        Some(block) => {
            creation_flags |= CREATE_UNICODE_ENVIRONMENT;
            block.as_ptr() as *mut c_void
        }
        None => ptr::null_mut(),
    };

    let mut proc_info: PROCESS_INFORMATION = unsafe { mem::zeroed() };
    unsafe {
        let success = CreateProcessW(
            ptr::null(),
            cmdline_str.as_ptr() as PWSTR,
            ptr::null_mut(),
            ptr::null_mut(),
            0,
            creation_flags,
            custom_env_block_pointer,
            cwd.as_ref().map_or_else(ptr::null, |s| s.as_ptr()),
            &mut startup_info_ex.StartupInfo as *mut STARTUPINFOW,
            &mut proc_info as *mut PROCESS_INFORMATION,
        );
        if success == 0 {
            return Err(Error::last_os_error());
        }
        if !proc_info.hThread.is_null() {
            CloseHandle(proc_info.hThread);
        }
    }

    let conin = UnblockedWriter::new(conin, PIPE_CAPACITY);
    let conout = UnblockedReader::new(conout, PIPE_CAPACITY);
    let child_watcher = ChildExitWatcher::new(proc_info.hProcess)?;

    Ok(WindowsConpty {
        backend: conpty,
        conout,
        conin,
        child_watcher,
    })
}

/// 构造隐藏子进程窗口的 `STARTUPINFOEXW`：
/// - `STARTF_USESTDHANDLES`（配合全空句柄）避免子进程继承宿主句柄；
/// - `STARTF_USESHOWWINDOW` + `wShowWindow = SW_HIDE` 抑制 GUI 进程拉起 ConPTY 时的控制台闪窗。
fn build_hidden_conpty_startup_info() -> STARTUPINFOEXW {
    let mut startup_info_ex: STARTUPINFOEXW = unsafe { mem::zeroed() };
    startup_info_ex.StartupInfo.lpTitle = ptr::null_mut();
    startup_info_ex.StartupInfo.cb = mem::size_of::<STARTUPINFOEXW>() as u32;
    startup_info_ex.StartupInfo.dwFlags = STARTF_USESTDHANDLES | STARTF_USESHOWWINDOW;
    startup_info_ex.StartupInfo.wShowWindow = SW_HIDE as u16;
    startup_info_ex
}

fn with_key(mut event: Event, key: usize) -> Event {
    event.key = key;
    event
}

impl EventedReadWrite for WindowsConpty {
    type Reader = UnblockedReader<AnonRead>;
    type Writer = UnblockedWriter<AnonWrite>;

    unsafe fn register(
        &mut self,
        poll: &Arc<Poller>,
        interest: Event,
        poll_opts: PollMode,
    ) -> io::Result<()> {
        self.conin
            .register(poll, with_key(interest, PTY_READ_WRITE_TOKEN), poll_opts);
        self.conout
            .register(poll, with_key(interest, PTY_READ_WRITE_TOKEN), poll_opts);
        self.child_watcher
            .register(poll, with_key(interest, PTY_CHILD_EVENT_TOKEN));
        Ok(())
    }

    fn reregister(
        &mut self,
        poll: &Arc<Poller>,
        interest: Event,
        poll_opts: PollMode,
    ) -> io::Result<()> {
        self.conin
            .register(poll, with_key(interest, PTY_READ_WRITE_TOKEN), poll_opts);
        self.conout
            .register(poll, with_key(interest, PTY_READ_WRITE_TOKEN), poll_opts);
        self.child_watcher
            .register(poll, with_key(interest, PTY_CHILD_EVENT_TOKEN));
        Ok(())
    }

    fn deregister(&mut self, _poll: &Arc<Poller>) -> io::Result<()> {
        self.conin.deregister();
        self.conout.deregister();
        self.child_watcher.deregister();
        Ok(())
    }

    fn reader(&mut self) -> &mut Self::Reader {
        &mut self.conout
    }

    fn writer(&mut self) -> &mut Self::Writer {
        &mut self.conin
    }
}

impl EventedPty for WindowsConpty {
    fn next_child_event(&mut self) -> Option<ChildEvent> {
        match self.child_watcher.event_rx().try_recv() {
            Ok(ev) => Some(ev),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => Some(ChildEvent::Exited(None)),
        }
    }
}

impl OnResize for WindowsConpty {
    fn on_resize(&mut self, window_size: WindowSize) {
        self.backend.on_resize(window_size);
    }
}

fn push_escaped_arg(cmd: &mut String, arg: &str) {
    let arg_bytes = arg.as_bytes();
    let quote = arg_bytes.iter().any(|c| *c == b' ' || *c == b'\t') || arg_bytes.is_empty();
    if quote {
        cmd.push('"');
    }

    let mut backslashes: usize = 0;
    for ch in arg.chars() {
        if ch == '\\' {
            backslashes += 1;
        } else {
            if ch == '"' {
                cmd.extend((0..=backslashes).map(|_| '\\'));
            }
            backslashes = 0;
        }
        cmd.push(ch);
    }

    if quote {
        cmd.extend((0..backslashes).map(|_| '\\'));
        cmd.push('"');
    }
}

/// 从 `alacritty_terminal::tty::Shell` 的 `Debug` 输出解析 `program` 和 `args`
/// （`alacritty_terminal 0.26` 未公开这两个字段，也不提供 getter）。
fn extract_shell(config: &Options) -> (String, Vec<String>) {
    let Some(shell) = config.shell.as_ref() else {
        return ("powershell".to_owned(), Vec::new());
    };
    parse_shell_debug(&format!("{shell:?}"))
        .unwrap_or_else(|| ("powershell".to_owned(), Vec::new()))
}

fn parse_shell_debug(debug_str: &str) -> Option<(String, Vec<String>)> {
    // 格式固定为：`Shell { program: "...", args: ["...", ...] }`
    let rest = debug_str.strip_prefix("Shell { program: ")?;
    let (program, rest) = parse_rust_debug_string(rest)?;
    let rest = rest.strip_prefix(", args: [")?;
    let mut args = Vec::new();
    let mut cursor = rest;
    loop {
        if let Some(tail) = cursor.strip_prefix(']') {
            if tail == " }" {
                return Some((program, args));
            }
            return None;
        }
        let (arg, tail) = parse_rust_debug_string(cursor)?;
        args.push(arg);
        cursor = tail.strip_prefix(", ").unwrap_or(tail);
    }
}

fn parse_rust_debug_string(input: &str) -> Option<(String, &str)> {
    let mut chars = input.char_indices();
    if chars.next()?.1 != '"' {
        return None;
    }
    let mut out = String::new();
    while let Some((idx, ch)) = chars.next() {
        match ch {
            '"' => return Some((out, &input[idx + 1..])),
            '\\' => {
                let (_, esc) = chars.next()?;
                match esc {
                    '\\' => out.push('\\'),
                    '"' => out.push('"'),
                    '\'' => out.push('\''),
                    'n' => out.push('\n'),
                    'r' => out.push('\r'),
                    't' => out.push('\t'),
                    '0' => out.push('\0'),
                    'u' => {
                        if chars.next()?.1 != '{' {
                            return None;
                        }
                        let mut hex = String::new();
                        for (_, h) in chars.by_ref() {
                            if h == '}' {
                                break;
                            }
                            hex.push(h);
                        }
                        let code = u32::from_str_radix(&hex, 16).ok()?;
                        out.push(char::from_u32(code)?);
                    }
                    other => out.push(other),
                }
            }
            other => out.push(other),
        }
    }
    None
}

fn cmdline(config: &Options) -> String {
    let (program, args) = extract_shell(config);

    let mut cmd = String::new();
    push_escaped_arg(&mut cmd, &program);

    for arg in &args {
        cmd.push(' ');
        if config.escape_args {
            push_escaped_arg(&mut cmd, arg);
        } else {
            cmd.push_str(arg);
        }
    }
    cmd
}

fn win32_string<S: AsRef<OsStr> + ?Sized>(value: &S) -> Vec<u16> {
    OsStr::new(value).encode_wide().chain(once(0)).collect()
}

fn convert_custom_env(custom_env: &HashMap<String, String>) -> Option<Vec<u16>> {
    if custom_env.is_empty() {
        return None;
    }

    let mut converted_block = Vec::new();
    let mut all_env_keys = HashSet::new();
    for (custom_key, custom_value) in custom_env {
        let custom_key_os = OsStr::new(custom_key);
        if all_env_keys.insert(custom_key_os.to_ascii_uppercase()) {
            add_windows_env_key_value_to_block(
                &mut converted_block,
                custom_key_os,
                OsStr::new(custom_value),
            );
        }
    }

    for (inherited_key, inherited_value) in std::env::vars_os() {
        if all_env_keys.insert(inherited_key.to_ascii_uppercase()) {
            add_windows_env_key_value_to_block(
                &mut converted_block,
                &inherited_key,
                &inherited_value,
            );
        }
    }

    converted_block.push(0);
    Some(converted_block)
}

fn add_windows_env_key_value_to_block(block: &mut Vec<u16>, key: &OsStr, value: &OsStr) {
    block.extend(key.encode_wide());
    block.push('=' as u16);
    block.extend(value.encode_wide());
    block.push(0);
}

// ---------------------------------------------------------------------------
// 非阻塞管道读写桥接（IOCP Poller）
// ---------------------------------------------------------------------------

struct PipeRegistration {
    interest: Mutex<Option<PipeInterest>>,
    end: PipeEnd,
}

#[derive(Copy, Clone)]
enum PipeEnd {
    Reader,
    Writer,
}

struct PipeInterest {
    event: Event,
    poller: Arc<Poller>,
    mode: PollMode,
}

pub(crate) struct UnblockedReader<R> {
    interest: Arc<PipeRegistration>,
    pipe: PipeReader,
    first_register: bool,
    _reader: PhantomData<R>,
}

impl<R: Read + Send + 'static> UnblockedReader<R> {
    fn new(mut source: R, pipe_capacity: usize) -> Self {
        let (reader, mut writer) = pipe(pipe_capacity);
        let interest = Arc::new(PipeRegistration {
            interest: Mutex::new(None),
            end: PipeEnd::Reader,
        });

        let _ = thread::Builder::new()
            .name("navop-conpty-reader".to_string())
            .spawn(move || {
                let waker = Waker::from(Arc::new(ThreadWaker(thread::current())));
                let mut context = Context::from_waker(&waker);
                loop {
                    match writer.poll_fill(&mut context, &mut source) {
                        Poll::Ready(Ok(0)) => return,
                        Poll::Ready(Ok(_)) => continue,
                        Poll::Ready(Err(e)) if e.kind() == io::ErrorKind::Interrupted => continue,
                        Poll::Ready(Err(e)) => {
                            tracing::debug!(error = %e, "conpty reader pipe stopped");
                            return;
                        }
                        Poll::Pending => thread::park(),
                    }
                }
            });

        Self {
            interest,
            pipe: reader,
            first_register: true,
            _reader: PhantomData,
        }
    }

    fn register(&mut self, poller: &Arc<Poller>, event: Event, mode: PollMode) {
        let mut interest = self
            .interest
            .interest
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        *interest = Some(PipeInterest {
            event,
            poller: poller.clone(),
            mode,
        });
        if (!self.pipe.is_empty() && event.readable) || self.first_register {
            self.first_register = false;
            let _ = poller.post(CompletionPacket::new(event));
        }
    }

    fn deregister(&self) {
        let mut interest = self
            .interest
            .interest
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        *interest = None;
    }

    fn try_read(&mut self, buf: &mut [u8]) -> usize {
        let waker = Waker::from(self.interest.clone());
        match self
            .pipe
            .poll_drain_bytes(&mut Context::from_waker(&waker), buf)
        {
            Poll::Pending => 0,
            Poll::Ready(n) => n,
        }
    }
}

impl<R: Read + Send + 'static> Read for UnblockedReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        Ok(self.try_read(buf))
    }
}

pub(crate) struct UnblockedWriter<W> {
    interest: Arc<PipeRegistration>,
    pipe: PipeWriter,
    _writer: PhantomData<W>,
}

impl<W: Write + Send + 'static> UnblockedWriter<W> {
    fn new(mut sink: W, pipe_capacity: usize) -> Self {
        let (mut reader, writer) = pipe(pipe_capacity);
        let interest = Arc::new(PipeRegistration {
            interest: Mutex::new(None),
            end: PipeEnd::Writer,
        });

        let _ = thread::Builder::new()
            .name("navop-conpty-writer".to_string())
            .spawn(move || {
                let waker = Waker::from(Arc::new(ThreadWaker(thread::current())));
                let mut context = Context::from_waker(&waker);
                loop {
                    match reader.poll_drain(&mut context, &mut sink) {
                        Poll::Ready(Ok(0)) => return,
                        Poll::Ready(Ok(_)) => continue,
                        Poll::Ready(Err(e)) if e.kind() == io::ErrorKind::Interrupted => continue,
                        Poll::Ready(Err(e)) => {
                            tracing::debug!(error = %e, "conpty writer pipe stopped");
                            return;
                        }
                        Poll::Pending => thread::park(),
                    }
                }
            });

        Self {
            interest,
            pipe: writer,
            _writer: PhantomData,
        }
    }

    fn register(&self, poller: &Arc<Poller>, event: Event, mode: PollMode) {
        let mut interest = self
            .interest
            .interest
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        *interest = Some(PipeInterest {
            event,
            poller: poller.clone(),
            mode,
        });
        if !self.pipe.is_full() && event.writable {
            let _ = poller.post(CompletionPacket::new(event));
        }
    }

    fn deregister(&self) {
        let mut interest = self
            .interest
            .interest
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        *interest = None;
    }

    fn try_write(&mut self, buf: &[u8]) -> usize {
        let waker = Waker::from(self.interest.clone());
        match self
            .pipe
            .poll_fill_bytes(&mut Context::from_waker(&waker), buf)
        {
            Poll::Pending => 0,
            Poll::Ready(n) => n,
        }
    }
}

impl<W: Write + Send + 'static> Write for UnblockedWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        Ok(self.try_write(buf))
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct ThreadWaker(thread::Thread);

impl Wake for ThreadWaker {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.0.unpark();
    }
}

impl Wake for PipeRegistration {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        let mut interest_lock = self.interest.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(interest) = interest_lock.as_ref() {
            let send_event = match self.end {
                PipeEnd::Reader => interest.event.readable,
                PipeEnd::Writer => interest.event.writable,
            };
            if send_event {
                let _ = interest.poller.post(CompletionPacket::new(interest.event));
                if matches!(interest.mode, PollMode::Oneshot | PollMode::EdgeOneshot) {
                    *interest_lock = None;
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 子进程退出监听
// ---------------------------------------------------------------------------

struct ChildInterest {
    poller: Arc<Poller>,
    event: Event,
}

struct ChildExitSender {
    sender: mpsc::Sender<ChildEvent>,
    interest: Arc<Mutex<Option<ChildInterest>>>,
    child_handle: AtomicPtr<c_void>,
}

extern "system" fn child_exit_callback(ctx: *mut c_void, timed_out: bool) {
    if timed_out {
        return;
    }

    let event_tx: Box<ChildExitSender> = unsafe { Box::from_raw(ctx as *mut ChildExitSender) };
    let mut exit_code = 0_u32;
    let child_handle = event_tx.child_handle.load(Ordering::Relaxed) as HANDLE;
    let status = unsafe { GetExitCodeProcess(child_handle, &mut exit_code) };
    let exit_status = if status == FALSE {
        None
    } else {
        Some(ExitStatus::from_raw(exit_code))
    };
    let _ = event_tx.sender.send(ChildEvent::Exited(exit_status));

    let interest = event_tx.interest.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(interest) = interest.as_ref() {
        let _ = interest.poller.post(CompletionPacket::new(interest.event));
    }
}

struct ChildExitWatcher {
    wait_handle: AtomicPtr<c_void>,
    event_rx: mpsc::Receiver<ChildEvent>,
    interest: Arc<Mutex<Option<ChildInterest>>>,
    child_handle: AtomicPtr<c_void>,
    _pid: Option<NonZeroU32>,
}

impl ChildExitWatcher {
    fn new(child_handle: HANDLE) -> Result<Self, Error> {
        let (event_tx, event_rx) = mpsc::channel();
        let mut wait_handle: HANDLE = ptr::null_mut();
        let interest = Arc::new(Mutex::new(None));
        let sender_ref = Box::new(ChildExitSender {
            sender: event_tx,
            interest: interest.clone(),
            child_handle: AtomicPtr::from(child_handle),
        });

        let success = unsafe {
            RegisterWaitForSingleObject(
                &mut wait_handle,
                child_handle,
                Some(child_exit_callback),
                Box::into_raw(sender_ref).cast(),
                INFINITE,
                WT_EXECUTEINWAITTHREAD | WT_EXECUTEONLYONCE,
            )
        };

        if success == 0 {
            Err(Error::last_os_error())
        } else {
            let pid = unsafe { NonZeroU32::new(GetProcessId(child_handle)) };
            Ok(Self {
                event_rx,
                interest,
                _pid: pid,
                child_handle: AtomicPtr::from(child_handle),
                wait_handle: AtomicPtr::from(wait_handle),
            })
        }
    }

    fn event_rx(&self) -> &mpsc::Receiver<ChildEvent> {
        &self.event_rx
    }

    fn register(&self, poller: &Arc<Poller>, event: Event) {
        *self.interest.lock().unwrap_or_else(|e| e.into_inner()) = Some(ChildInterest {
            poller: poller.clone(),
            event,
        });
    }

    fn deregister(&self) {
        *self.interest.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
}

impl Drop for ChildExitWatcher {
    fn drop(&mut self) {
        unsafe {
            let wait_handle = self.wait_handle.load(Ordering::Relaxed) as HANDLE;
            if !wait_handle.is_null() {
                UnregisterWait(wait_handle);
            }
            let child_handle = self.child_handle.load(Ordering::Relaxed) as HANDLE;
            if !child_handle.is_null() {
                CloseHandle(child_handle);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alacritty_terminal::tty::Shell;

    #[test]
    fn hidden_conpty_startup_info_sets_sw_hide_and_stdhandles() {
        let info = build_hidden_conpty_startup_info();
        assert_ne!(info.StartupInfo.dwFlags & STARTF_USESTDHANDLES, 0);
        assert_ne!(info.StartupInfo.dwFlags & STARTF_USESHOWWINDOW, 0);
        assert_eq!(info.StartupInfo.wShowWindow, SW_HIDE as u16);
    }

    #[test]
    fn cmdline_quotes_program_path_with_spaces() {
        let options = Options {
            shell: Some(Shell::new(
                r"C:\Program Files\PowerShell\7\pwsh.exe".to_string(),
                vec!["-NoLogo".to_string(), r"C:\My Scripts\init.ps1".to_string()],
            )),
            working_directory: None,
            drain_on_exit: true,
            env: HashMap::new(),
            escape_args: true,
        };
        assert_eq!(
            cmdline(&options),
            r#""C:\Program Files\PowerShell\7\pwsh.exe" -NoLogo "C:\My Scripts\init.ps1""#
        );
    }
}
