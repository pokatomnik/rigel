use clap::Subcommand;

use crate::controllers::{
    chat_controller::chat_controller::ChatController,
    init_controller::init_controller::InitController,
};

#[derive(Subcommand, Clone)]
pub(crate) enum Commands {
    Init(InitController),
    Chat(ChatController),
}
