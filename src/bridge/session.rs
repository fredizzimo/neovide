//! This module contains adaptations of the functions found in
//! https://github.com/KillTheMule/nvim-rs/blob/master/src/create/tokio.rs

#[cfg(debug_assertions)]
use core::fmt;
use std::{
    io::{Error, IsTerminal, Result},
    process::Stdio,
};

use anyhow::Context;
use nvim_rs::{error::LoopError, neovim::Neovim, Handler};
#[cfg(target_os = "windows")]
use std::os::windows::io::AsRawHandle;
use tokio::{
    io::{split, AsyncBufReadExt, AsyncRead, AsyncWrite, BufReader},
    net::TcpStream,
    process::{Child, Command},
    spawn,
    task::JoinHandle,
};
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};
#[cfg(target_os = "windows")]
use windows::Win32::{
    Foundation::{DuplicateHandle, DUPLICATE_SAME_ACCESS, HANDLE, INVALID_HANDLE_VALUE, SetHandleInformation, HANDLE_FLAG_INHERIT, HANDLE_FLAGS},
    Security::SECURITY_ATTRIBUTES,
    System::{
        Pipes::CreatePipe,
        Threading::{GetCurrentProcess},
    },
};
#[cfg(target_os = "windows")]
use std::{
    fs::File,
    os::windows::io::FromRawHandle,
};

pub type NeovimWriter = Box<dyn futures::AsyncWrite + Send + Unpin + 'static>;

type BoxedReader = Box<dyn AsyncRead + Send + Unpin + 'static>;
type BoxedWriter = Box<dyn AsyncWrite + Send + Unpin + 'static>;

pub struct NeovimSession {
    pub neovim: Neovim<NeovimWriter>,
    pub io_handle: JoinHandle<std::result::Result<(), Box<LoopError>>>,
    pub neovim_process: Option<Child>,
    pub stderr_task: Option<JoinHandle<Vec<String>>>,
    #[cfg(not(target_os = "windows"))]
    pub stdin_fd: Option<rustix::fd::OwnedFd>,
    #[cfg(target_os = "windows")]
    pub stdin_fd: Option<(Option<File>, File)>,
}

#[cfg(target_os = "windows")]
pub struct SendableHandle(pub HANDLE);
#[cfg(target_os = "windows")]
unsafe impl Send for SendableHandle {}

#[cfg(debug_assertions)]
impl fmt::Debug for NeovimSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NeovimSession")
            .field("io_handle", &self.io_handle)
            .finish()
    }
}

impl NeovimSession {
    pub async fn new(
        instance: NeovimInstance,
        handler: impl Handler<Writer = NeovimWriter>,
    ) -> anyhow::Result<Self> {
        // This needs to be done before the process is spawned, since the file descriptors are
        // inherited.
        let stdin_fd = instance.forward_stdin();
        let (reader, writer, stderr_reader, neovim_process) = instance.connect().await?;
        // // But on window after, because DuplicateHandle needs access to the target process id
        // #[cfg(target_os = "windows")]
        // let stdin_fd = forward_stdin(&neovim_process);
        // Spawn a background task to read from stderr
        let stderr_task = stderr_reader.map(|reader| {
            tokio::spawn(async move {
                let mut lines = Vec::new();
                let mut reader = BufReader::new(reader).lines();
                while let Some(line) = reader.next_line().await.unwrap_or_default() {
                    log::error!("{line}");
                    lines.push(line);
                }
                lines
            })
        });
        let handshake_message = "NeovideToNeovimMagicHandshakeMessage";

        let handshake_res = Neovim::<NeovimWriter>::handshake(
            reader.compat(),
            Box::new(writer.compat_write()),
            handler,
            handshake_message,
        )
        .await;
        match handshake_res {
            Err(err) => {
                if let Some(stderr_task) = stderr_task {
                    let stderr = "stderr output:\n".to_owned() + &stderr_task.await?.join("\n");
                    Err(err).context(stderr)
                } else {
                    Err(err.into())
                }
            }
            Ok((neovim, io)) => {
                let io_handle = spawn(io);

                Ok(Self {
                    neovim,
                    io_handle,
                    neovim_process,
                    stderr_task,
                    stdin_fd,
                })
            }
        }
    }
}

/// An existing or future Neovim instance along with a means for establishing a connection.
#[derive(Debug)]
pub enum NeovimInstance {
    /// A new embedded instance to be spawned by the given command.
    Embedded(Command),

    /// An existing instance listening on `address`.
    ///
    /// Interprets `address` in the same way as `:help --server`: If it contains a `:` it's
    /// interpreted as a TCP/IPv4/IPv6 address. Otherwise it's interpreted as a named pipe or Unix
    /// domain socket path. Spawns and connects to an embedded Neovim instance.
    Server { address: String },
}

impl NeovimInstance {
    async fn connect(
        self,
    ) -> Result<(BoxedReader, BoxedWriter, Option<BoxedReader>, Option<Child>)> {
        match self {
            NeovimInstance::Embedded(cmd) => Self::spawn_process(cmd).await,
            NeovimInstance::Server { address } => Self::connect_to_server(address)
                .await
                .map(|(reader, writer)| (reader, writer, None, None)),
        }
    }

    async fn spawn_process(
        mut cmd: Command,
    ) -> Result<(BoxedReader, BoxedWriter, Option<BoxedReader>, Option<Child>)> {
        log::debug!("Starting neovim with: {cmd:?}");
        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let reader = Box::new(
            child
                .stdout
                .take()
                .ok_or_else(|| Error::other("Can't open stdout"))?,
        );
        let writer = Box::new(
            child
                .stdin
                .take()
                .ok_or_else(|| Error::other("Can't open stdin"))?,
        );

        let stderr_reader = Box::new(
            child
                .stderr
                .take()
                .ok_or_else(|| Error::other("Can't open stderr"))?,
        );

        Ok((reader, writer, Some(stderr_reader), Some(child)))
    }

    async fn connect_to_server(address: String) -> Result<(BoxedReader, BoxedWriter)> {
        if address.contains(':') {
            Ok(Self::split(TcpStream::connect(address).await?))
        } else {
            #[cfg(unix)]
            return Ok(Self::split(tokio::net::UnixStream::connect(address).await?));

            #[cfg(windows)]
            {
                // Fixup the address if the pipe on windows does not start with \\.\pipe\.
                let address = if address.starts_with("\\\\.\\pipe\\") {
                    address
                } else {
                    format!("\\\\.\\pipe\\{address}")
                };
                Ok(Self::split(
                    tokio::net::windows::named_pipe::ClientOptions::new().open(address)?,
                ))
            }

            #[cfg(not(any(unix, windows)))]
            Err(Error::new(
                ErrorKind::Unsupported,
                "Unix Domain Sockets and Named Pipes are not supported on this platform",
            ))
        }
    }

    #[cfg(not(target_os = "windows"))]
    fn forward_stdin(&self) -> Option<rustix::fd::OwnedFd> {
        // stdin should be forwarded only in embedded mode when stdio is piped
        match self {
            Self::Embedded(..) => {
                let stdin = std::io::stdin();
                let is_pipe = !stdin.is_terminal();
                is_pipe.then(|| rustix::io::dup(stdin).ok()).flatten()
            }
            Self::Server { .. } => None,
        }
    }

    #[cfg(target_os = "windows")]
    fn forward_stdin(&self) -> Option<(Option<File>, File)> {
        // stdin should be forwarded only in embedded mode when stdio is piped
        match self {
            Self::Embedded(..) => {
                let stdin = std::io::stdin();
                let is_pipe = !stdin.is_terminal();
                is_pipe
                    .then(|| unsafe {
                        let mut read_handle = HANDLE::default();
                        let mut write_handle = HANDLE::default();
                        let mut sa = SECURITY_ATTRIBUTES {
                            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                            lpSecurityDescriptor: std::ptr::null_mut(),
                            bInheritHandle: true.into(),
                        };
                        CreatePipe(&mut read_handle, &mut write_handle, Some(&mut sa), 0).unwrap();
                        //SetHandleInformation(write_handle, HANDLE_FLAG_INHERIT.0, HANDLE_FLAGS(0));
                        Some((
                            Some(File::from_raw_handle(read_handle.0 as *mut _)),
                            File::from_raw_handle(write_handle.0 as *mut _),
                        ))
                    })
                    .flatten()
            }
            Self::Server { .. } => None,
        }
    }

    fn split(
        stream: impl AsyncRead + AsyncWrite + Send + Unpin + 'static,
    ) -> (BoxedReader, BoxedWriter) {
        let (reader, writer) = split(stream);
        (Box::new(reader), Box::new(writer))
    }
}

// #[cfg(target_os = "windows")]
// fn forward_stdin(child: &Option<Child>) -> Option<SendableHandle> {
//     // stdin should be forwarded only in embedded mode when stdio is piped
//     if let Some(child) = child {
//         let stdin = std::io::stdin();
//         let is_pipe = !stdin.is_terminal();
//         is_pipe
//             .then(|| {
//                 let current_process = unsafe { GetCurrentProcess() };
//                 let target_process = child;
//                 let mut target_handle = INVALID_HANDLE_VALUE;
//                 unsafe {
//                     if DuplicateHandle(
//                         current_process,
//                         HANDLE(stdin.as_raw_handle()),
//                         HANDLE(target_process.raw_handle().unwrap_or(INVALID_HANDLE_VALUE.0)),
//                         &mut target_handle,
//                         0,
//                         false,
//                         DUPLICATE_SAME_ACCESS,
//                     ).is_ok() {
//                         Some(SendableHandle(target_handle))
//                     } else {
//                         None
//                     }
//                 }
//             }).flatten()
//     } else {
//         None
//     }
// }
