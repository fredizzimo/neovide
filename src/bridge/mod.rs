mod clipboard;
mod command;
mod events;
mod handler;
pub mod session;
mod setup;
mod ui_commands;

use anyhow::{bail, Context, Result};
use log::{error, info};
use nvim_rs::{error::CallError, Neovim, UiAttachOptions, Value};
use std::{io::Error, process::exit, sync::Arc};
use tokio::{
    runtime::{Builder, Runtime},
    task::JoinHandle,
};

use crate::{
    cmd_line::CmdLineSettings, dimensions::Dimensions, error_handling::ResultPanicExplanation,
    running_tracker::*, settings::*,
};
use handler::NeovimHandler;
use session::{NeovimInstance, NeovimSession};
use setup::setup_neovide_specific_state;

pub use command::create_nvim_command;
pub use events::*;
pub use session::NeovimWriter;
pub use ui_commands::{start_ui_command_handler, ParallelCommand, SerialCommand, UiCommand};

const INTRO_MESSAGE_LUA: &str = include_str!("../../lua/intro.lua");
const NEOVIM_REQUIRED_VERSION: &str = "0.9.2";

enum RuntimeState {
    Idle,
    Invalid,
    Launched(NeovimSession),
    Attached(JoinHandle<()>),
}

pub struct NeovimRuntime {
    runtime: Runtime,
    state: RuntimeState,
}

fn neovim_instance() -> Result<NeovimInstance> {
    if let Some(address) = SETTINGS.get::<CmdLineSettings>().server {
        Ok(NeovimInstance::Server { address })
    } else {
        let cmd = create_nvim_command()?;
        Ok(NeovimInstance::Embedded(cmd))
    }
}

pub async fn setup_intro_message_autocommand(
    nvim: &Neovim<NeovimWriter>,
) -> Result<Value, Box<CallError>> {
    let args = vec![Value::from("setup_autocommand")];
    nvim.exec_lua(INTRO_MESSAGE_LUA, args).await
}

pub async fn show_intro_message(
    nvim: &Neovim<NeovimWriter>,
    message: &[String],
) -> Result<Value, Box<CallError>> {
    let mut args = vec![Value::from("show_intro")];
    let lines = message.iter().map(|line| Value::from(line.as_str()));
    args.extend(lines);
    nvim.exec_lua(INTRO_MESSAGE_LUA, args).await
}

async fn launch() -> Result<NeovimSession> {
    let neovim_instance = neovim_instance()?;
    let handler = NeovimHandler::new();
    let session = NeovimSession::new(neovim_instance, handler)
        .await
        .context("Could not locate or start neovim process")?;

    // Check the neovim version to ensure its high enough
    match session
        .neovim
        .command_output(&format!("echo has('nvim-{NEOVIM_REQUIRED_VERSION}')"))
        .await
        .as_deref()
    {
        Ok("1") => {} // This is just a guard
        _ => {
            bail!("Neovide requires nvim version {NEOVIM_REQUIRED_VERSION} or higher. Download the latest version here https://github.com/neovim/neovim/wiki/Installing-Neovim");
        }
    }
    let settings = SETTINGS.get::<CmdLineSettings>();

    let should_handle_clipboard = settings.wsl || settings.server.is_some();
    setup_neovide_specific_state(&session.neovim, should_handle_clipboard).await;

    start_ui_command_handler(Arc::clone(&session.neovim));
    SETTINGS.read_initial_values(&session.neovim).await;
    SETTINGS.setup_changed_listeners(&session.neovim).await;
    Ok(session)
}

async fn run(session: NeovimSession, grid_size: Option<Dimensions>) {
    let settings = SETTINGS.get::<CmdLineSettings>();
    let mut options = UiAttachOptions::new();
    options.set_linegrid_external(true);
    options.set_multigrid_external(!settings.no_multi_grid);
    options.set_rgb(true);

    // Triggers loading the user config

    let grid_size = grid_size.map_or(DEFAULT_GRID_SIZE, |v| v.clamped_grid_size());
    session
        .neovim
        .ui_attach(grid_size.width as i64, grid_size.height as i64, &options)
        .await
        .unwrap_or_explained_panic("Could not attach ui to neovim process");

    info!("Neovim process attached");

    match session.io_handle.await {
        Err(join_error) => error!("Error joining IO loop: '{}'", join_error),
        Ok(Err(error)) => {
            if !error.is_channel_closed() {
                error!("Error: '{}'", error);
            }
        }
        Ok(Ok(())) => {}
    };
    RUNNING_TRACKER.quit("neovim processed failed");
}

impl NeovimRuntime {
    pub fn new() -> Result<Self, Error> {
        let runtime = Builder::new_multi_thread().enable_all().build()?;

        Ok(Self {
            runtime,
            state: RuntimeState::Idle,
        })
    }

    pub fn launch(&mut self) -> Result<()> {
        assert!(matches!(self.state, RuntimeState::Idle));
        let res = self.runtime.block_on(launch())?;
        self.state = RuntimeState::Launched(res);
        Ok(())
    }

    pub fn attach(&mut self, grid_size: Option<Dimensions>) {
        assert!(matches!(self.state, RuntimeState::Launched(..)));
        if let RuntimeState::Launched(session) =
            std::mem::replace(&mut self.state, RuntimeState::Invalid)
        {
            self.state = RuntimeState::Attached(self.runtime.spawn(run(session, grid_size)));
        }
    }
}

impl Drop for NeovimRuntime {
    fn drop(&mut self) {
        if let RuntimeState::Attached(join_handle) =
            std::mem::replace(&mut self.state, RuntimeState::Idle)
        {
            let _ = self.runtime.block_on(join_handle);
        }
    }
}
