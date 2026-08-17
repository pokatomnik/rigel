use clap::Subcommand;

use crate::controllers::{chat_controller::ChatController, init_controller::InitController};

#[derive(Subcommand, Clone)]
pub(crate) enum Commands {
    Init(InitController),
    Chat(ChatController),
}
